use crate::token_source::credentials::ExternalAccount;
use secret_vault_value::SecretValue;
use std::collections::HashMap;

use tracing::*;

#[derive(Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
pub enum ExternalCredentialSource {
    #[cfg(feature = "external-account-aws")]
    Aws(Aws),
    UrlBased(ExternalCredentialUrl),
    FileBased(ExternalCredentialFile),
}

#[derive(Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ExternalCredentialUrl {
    url: String,
    headers: Option<HashMap<String, SecretValue>>,
    format: Option<ExternalCredentialUrlFormat>,
}

#[derive(Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ExternalCredentialFile {
    file: String,
    format: Option<ExternalCredentialUrlFormat>,
}

/// https://google.aip.dev/auth/4117#determining-the-subject-token-in-aws
#[derive(Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Aws {
    /// This defines the regional AWS GetCallerIdentity action URL. This URL should be used
    ///  to determine the AWS account ID and its roles.
    pub regional_cred_verification_url: String,
    /// This is the environment identifier, of format `aws{version}`.
    pub environment_id: String,
    /// This URL should be used to determine the current AWS region needed for the signed
    /// request construction when the region environment variables are not present.
    pub region_url: Option<String>,
    /// This AWS metadata server URL should be used to retrieve the access key, secret key
    /// and security token needed to sign the GetCallerIdentity request.
    pub url: Option<String>,
    /// Presence of this URL enforces the auth libraries to fetch a Session Token from AWS.
    /// This field is required for EC2 instances using IMDSv2. This Session Token would
    /// later be used while making calls to the metadata endpoint.
    pub imdsv2_session_token_url: Option<String>,
}

#[derive(Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type")]
#[serde(rename_all = "camelCase")]
pub enum ExternalCredentialUrlFormat {
    Json(ExternalCredentialUrlFormatJson),
    Text,
}

#[derive(Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ExternalCredentialUrlFormatJson {
    pub subject_token_field_name: String,
}

pub async fn subject_token(
    client: &reqwest::Client,
    external_account: &ExternalAccount,
) -> crate::error::Result<SecretValue> {
    match &external_account.credential_source {
        ExternalCredentialSource::UrlBased(ref url_creds) => {
            subject_token_url(client, url_creds).await
        }
        ExternalCredentialSource::FileBased(ref url_creds) => subject_token_file(url_creds).await,
        #[cfg(feature = "external-account-aws")]
        ExternalCredentialSource::Aws(Aws {
            regional_cred_verification_url,
            environment_id,
            ..
        }) => {
            debug!(
                "Using external credentials AWS source. Regional URL: {}",
                regional_cred_verification_url
            );
            if environment_id.starts_with("aws") {
                if environment_id != "aws1" {
                    return Err(crate::error::ErrorKind::ExternalCredsSourceError(
                        "unsupported aws version".to_string(),
                    )
                    .into());
                }
            };
            let (credentials, region) = aws::get_aws_props().await?;
            aws::subject_token_aws(
                regional_cred_verification_url.as_str(),
                credentials,
                region,
                std::time::SystemTime::now(),
                &external_account.audience,
            )
            .await
        }
    }
}

pub async fn subject_token_url(
    client: &reqwest::Client,
    url_creds: &ExternalCredentialUrl,
) -> crate::error::Result<SecretValue> {
    debug!(
        "Using external credentials URL source {}. Format: {:?}",
        &url_creds.url, &url_creds.format
    );
    let mut request = client.get(url_creds.url.as_str());

    if let Some(headers) = &url_creds.headers {
        for (header_name, header_value) in headers {
            request = request.header(header_name, header_value.as_sensitive_str());
        }
    }

    let response = request.send().await?;

    if response.status().is_success() {
        match &url_creds.format {
            None | Some(ExternalCredentialUrlFormat::Text) => Ok(response.text().await?.into()),
            Some(ExternalCredentialUrlFormat::Json(json_settings)) => {
                let json: serde_json::Value = response.json().await?;
                subject_token_from_json(&json, &json_settings.subject_token_field_name)
            }
        }
    } else {
        let status = response.status();
        let err_body = response.text().await?;
        let err_text = format!(
            "Unable to receive subject using external credential url: {}. HTTP: {} {}",
            &url_creds.url, status, err_body
        );
        Err(crate::error::ErrorKind::ExternalCredsSourceError(err_text).into())
    }
}

pub async fn subject_token_file(
    url_creds: &ExternalCredentialFile,
) -> crate::error::Result<SecretValue> {
    debug!(
        "Using external credentials file source {}. Format: {:?}",
        &url_creds.file, &url_creds.format
    );
    let file_content: String = std::fs::read_to_string(url_creds.file.as_str()).map_err(|e| {
        crate::error::ErrorKind::ExternalCredsSourceError(format!(
            "External file is not readable: {}",
            e
        ))
    })?;
    match &url_creds.format {
        None | Some(ExternalCredentialUrlFormat::Text) => Ok(file_content.into()),
        Some(ExternalCredentialUrlFormat::Json(json_settings)) => {
            let json: serde_json::Value =
                serde_json::from_str(file_content.as_str()).map_err(|e| {
                    crate::error::ErrorKind::ExternalCredsSourceError(format!(
                        "External file JSON format error: {}",
                        e
                    ))
                })?;
            subject_token_from_json(&json, &json_settings.subject_token_field_name)
        }
    }
}

fn subject_token_from_json(
    json: &serde_json::Value,
    subject_token_field_name: &str,
) -> crate::error::Result<SecretValue> {
    let json_object = json.as_object().ok_or_else(|| {
        crate::error::ErrorKind::ExternalCredsSourceError(format!(
            "External subject JSON format is not object: {}",
            json
        ))
    })?;
    let subject_json_value = json_object.get(subject_token_field_name).ok_or_else(|| {
        crate::error::ErrorKind::ExternalCredsSourceError(format!(
            "External subject JSON format doesn't contain required field: {}",
            subject_token_field_name
        ))
    })?;
    subject_json_value.as_str().map(Into::into).ok_or_else(|| {
        crate::error::ErrorKind::ExternalCredsSourceError(format!(
            "External subject JSON field must have string type: {}",
            subject_token_field_name
        ))
        .into()
    })
}

#[cfg(feature = "external-account-aws")]
mod aws {
    use crate::error::Error;
    use crate::error::ErrorKind;
    use aws_config::Region;
    use aws_credential_types::provider::ProvideCredentials;
    use aws_credential_types::Credentials;
    use aws_sigv4::http_request::{
        SignableBody, SignatureLocation, SigningParams, SigningSettings,
    };
    use aws_sigv4::sign::v4::SigningParams as V4SigningParams;
    use hyper::http::{Method, Request};
    use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
    use secret_vault_value::SecretValue;
    use serde::Serialize;
    use std::time::SystemTime;

    pub async fn subject_token_aws(
        regional_cred_verification_url: &str,
        credentials: Credentials,
        region: Region,
        sign_at: SystemTime,
        audience: &str,
    ) -> crate::error::Result<SecretValue> {
        let identity = credentials.into();
        let signature_time = sign_at;

        let mut signing_settings = SigningSettings::default();
        signing_settings.signature_location = SignatureLocation::Headers;
        let v4_signing_params = V4SigningParams::builder()
            .name("sts")
            .identity(&identity)
            .region(region.as_ref())
            .time(signature_time)
            .settings(signing_settings)
            .build()
            .map_err(|e| Error::from(ErrorKind::ExternalCredsSourceError(e.to_string())))?;
        let params = SigningParams::V4(v4_signing_params);

        let regional_cred_verification_url =
            regional_cred_verification_url.replace("{region}", region.as_ref());
        let subject_token_url = regional_cred_verification_url;
        let url = url::Url::parse(&subject_token_url)
            .map_err(|e| Error::from(ErrorKind::ExternalCredsSourceError(e.to_string())))?;
        let method = Method::POST;
        let mut headers = vec![("x-goog-cloud-target-resource", audience)];
        if let Some(host) = url.host_str() {
            headers.push(("Host", host))
        }
        let mut req = Request::builder().uri(url.to_string()).method(&method);
        for header in &headers {
            req = req.header(header.0, header.1);
        }
        let mut request = req
            .body(())
            .map_err(|e| Error::from(ErrorKind::ExternalCredsSourceError(e.to_string())))?;

        let signable_request = aws_sigv4::http_request::SignableRequest::new(
            method.as_str(),
            &subject_token_url,
            headers.into_iter(),
            SignableBody::empty(),
        )
        .map_err(|e| Error::from(ErrorKind::ExternalCredsSourceError(e.to_string())))?;
        let (instruction, _) = aws_sigv4::http_request::sign(signable_request, &params)
            .map_err(|e| Error::from(ErrorKind::ExternalCredsSourceError(e.to_string())))?
            .into_parts();
        instruction.apply_to_request_http1x(&mut request);
        let payload = AWSRequest {
            url: subject_token_url.to_string(),
            method: method.to_string(),
            headers: request
                .headers()
                .into_iter()
                .flat_map(|(k, v)| {
                    v.to_str()
                        .ok()
                        .map(|v| AWSRequestHeader::new(k.to_string(), v.to_string()))
                })
                .collect(),
        };
        let payload =
            serde_json::to_string(&payload).map_err(|e| Error::from(ErrorKind::TokenJson(e)))?;
        let sts_token = utf8_percent_encode(&payload, NON_ALPHANUMERIC).to_string();

        Ok(sts_token.into())
    }

    pub async fn get_aws_props() -> crate::error::Result<(Credentials, Region)> {
        let region_provider =
            aws_config::default_provider::region::DefaultRegionChain::builder().build();
        let region = region_provider.region().await.ok_or_else(|| {
            Error::from(ErrorKind::ExternalCredsSourceError(
                "region not found".to_string(),
            ))
        })?;
        let credentials_provider =
            aws_config::default_provider::credentials::DefaultCredentialsChain::builder()
                .build()
                .await;
        let credentials: Credentials = credentials_provider
            .provide_credentials()
            .await
            .map_err(|e| Error::from(ErrorKind::ExternalCredsSourceError(e.to_string())))?
            .into();
        Ok((credentials, region))
    }

    #[derive(Debug, Serialize)]
    struct AWSRequest {
        url: String,
        method: String,
        headers: Vec<AWSRequestHeader>,
    }

    #[derive(Debug, Serialize, Clone)]
    struct AWSRequestHeader {
        key: String,
        value: String,
    }

    impl AWSRequestHeader {
        pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
            Self {
                key: key.into(),
                value: value.into(),
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use std::time::SystemTime;

        use aws_config::Region;
        use aws_credential_types::Credentials;
        use chrono::NaiveDateTime;

        use super::subject_token_aws;
        #[tokio::test]
        async fn sanity_check_subject_token() {
            // This test uses the following implementation for reference
            // https://github.com/yoshidan/google-cloud-rust/blob/8d09d6156dfb29965cd20539375896f16b3f739d/foundation/auth/src/token_source/external_account_source/aws_subject_token_source.rs#L381
            let credentials = Credentials::new(
                "AccessKeyId",
                "SecretAccessKey",
                Some("SecurityToken".to_string()),
                None,
                "test",
            );
            let sign_at: SystemTime =
                NaiveDateTime::parse_from_str("2022-12-31 00:00:00", "%Y-%m-%d %H:%M:%S")
                    .unwrap()
                    .and_utc()
                    .into();
            let region = Region::from_static("ap-northeast-1b");
            let audience = "//iam.googleapis.com/projects/myprojectnumber/locations/global/workloadIdentityPools/aws-test/providers/aws-test";
            let regional_cred_verification_url =
                "https://sts.{region}.amazonaws.com?Action=GetCallerIdentity&Version=2011-06-15";
            let result = subject_token_aws(
                regional_cred_verification_url,
                credentials,
                region.clone(),
                sign_at,
                audience,
            )
            .await;
            assert_eq!(
            result.unwrap().sensitive_value_to_str().unwrap(),
            "%7B%22url%22%3A%22https%3A%2F%2Fsts%2Eap%2Dnortheast%2D1b%2Eamazonaws%2Ecom%3FAction%3DGetCallerIdentity%26Version%3D2011%2D06%2D15%22%2C%22method%22%3A%22POST%22%2C%22headers%22%3A%5B%7B%22key%22%3A%22x%2Dgoog%2Dcloud%2Dtarget%2Dresource%22%2C%22value%22%3A%22%2F%2Fiam%2Egoogleapis%2Ecom%2Fprojects%2Fmyprojectnumber%2Flocations%2Fglobal%2FworkloadIdentityPools%2Faws%2Dtest%2Fproviders%2Faws%2Dtest%22%7D%2C%7B%22key%22%3A%22host%22%2C%22value%22%3A%22sts%2Eap%2Dnortheast%2D1b%2Eamazonaws%2Ecom%22%7D%2C%7B%22key%22%3A%22x%2Damz%2Ddate%22%2C%22value%22%3A%2220221231T000000Z%22%7D%2C%7B%22key%22%3A%22authorization%22%2C%22value%22%3A%22AWS4%2DHMAC%2DSHA256%20Credential%3DAccessKeyId%2F20221231%2Fap%2Dnortheast%2D1b%2Fsts%2Faws4%5Frequest%2C%20SignedHeaders%3Dhost%3Bx%2Damz%2Ddate%3Bx%2Damz%2Dsecurity%2Dtoken%3Bx%2Dgoog%2Dcloud%2Dtarget%2Dresource%2C%20Signature%3D168a40df8b7c11fb0588a13cada1443e31e4736de702232f9a2177b26edda21c%22%7D%2C%7B%22key%22%3A%22x%2Damz%2Dsecurity%2Dtoken%22%2C%22value%22%3A%22SecurityToken%22%7D%5D%7D"
        );
        }
    }
}
