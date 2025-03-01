use std::{
    env, fs,
    path::{Path, PathBuf},
};

use crate::token_source::ext_creds_source::ExternalCredentialSource;
use crate::token_source::{BoxSource, Source, Token};
use async_trait::async_trait;
use secret_vault_value::SecretValue;
use tracing::debug;

#[derive(Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
pub enum Credentials {
    ServiceAccount(ServiceAccount),
    User(User),
    // GCP Keyless integration with external parties such as GitHub
    ExternalAccount(ExternalAccount),
    ServiceAccountImpersonation(ServiceAccountImpersonation),
}

impl Credentials {
    pub(crate) fn quota_project_id(&self) -> Option<&str> {
        match self {
            Self::ExternalAccount(ea) => ea.quota_project_id.as_deref(),
            Self::User(u) => u.quota_project_id.as_deref(),
            Self::ServiceAccount(sa) => sa.quota_project_id.as_deref(),
            Self::ServiceAccountImpersonation(sa) => match &sa.source_credentials {
                ServiceAccountImpersonationSourceCredentials::ServiceAccount(sa) => {
                    sa.quota_project_id.as_deref()
                }
                ServiceAccountImpersonationSourceCredentials::User(u) => {
                    u.quota_project_id.as_deref()
                }
            },
        }
    }
}

#[derive(Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ServiceAccount {
    pub client_email: String,
    private_key_id: String,
    private_key: SecretValue,
    token_uri: String,
    #[serde(skip)]
    pub scopes: Vec<String>,
    pub quota_project_id: Option<String>,
}

#[derive(Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct User {
    client_secret: SecretValue,
    pub client_id: String,
    refresh_token: SecretValue,
    pub quota_project_id: Option<String>,
}

#[derive(Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ExternalAccount {
    pub audience: String,
    pub subject_token_type: String,
    pub token_url: String,
    pub service_account_impersonation_url: Option<String>,
    pub service_account_impersonation: Option<ServiceAccountImpersonationSettings>,
    pub quota_project_id: Option<String>,
    pub credential_source: ExternalCredentialSource,
    #[serde(skip)]
    pub scopes: Vec<String>,
}

#[derive(Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ServiceAccountImpersonationSettings {
    pub token_lifetime_seconds: Option<u64>,
}

#[derive(Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ServiceAccountImpersonation {
    pub service_account_impersonation_url: String,
    pub source_credentials: ServiceAccountImpersonationSourceCredentials,
}

#[derive(Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
pub enum ServiceAccountImpersonationSourceCredentials {
    ServiceAccount(ServiceAccount),
    User(User),
}

#[async_trait]
impl Source for Credentials {
    async fn token(&self) -> crate::error::Result<Token> {
        match self {
            Credentials::ServiceAccount(sa) => jwt::token(sa).await,
            Credentials::User(user) => oauth2::token(user).await,
            Credentials::ExternalAccount(external_account) => {
                external_account::token(external_account).await
            }
            Credentials::ServiceAccountImpersonation(sa) => impersonate_account::token(sa).await,
        }
    }
}

impl From<Credentials> for BoxSource {
    fn from(v: Credentials) -> Self {
        Box::new(v)
    }
}

const ENV_KEY: &str = "GOOGLE_APPLICATION_CREDENTIALS";

pub fn from_env_var(scopes: &[String]) -> crate::error::Result<Option<Credentials>> {
    match env::var(ENV_KEY) {
        Ok(path) => from_file(path, scopes).map(Some),
        Err(_) => Ok(None),
    }
}

pub fn from_well_known_file(scopes: &[String]) -> crate::error::Result<Option<Credentials>> {
    match well_known_file_path() {
        Some(path) if path.exists() => from_file(path, scopes).map(Some),
        _ => Ok(None),
    }
}

fn well_known_file_path() -> Option<PathBuf> {
    let mut buf = {
        #[cfg(target_os = "windows")]
        {
            PathBuf::from(env::var("APPDATA").ok()?)
        }
        #[cfg(not(target_os = "windows"))]
        {
            let mut buf = PathBuf::from(env::var("HOME").ok()?);
            buf.push(".config");
            buf
        }
    };
    buf.push("gcloud");
    buf.push("application_default_credentials.json");
    Some(buf)
}

pub fn from_json(buf: &[u8], scopes: &[String]) -> crate::error::Result<Credentials> {
    let mut creds =
        serde_json::from_slice(buf).map_err(crate::error::ErrorKind::CredentialsJson)?;
    if let Credentials::ServiceAccount(ref mut sa) = creds {
        sa.scopes = scopes.to_owned()
    } else if let Credentials::ExternalAccount(ref mut external_account) = creds {
        external_account.scopes = scopes.to_owned()
    }
    Ok(creds)
}

pub fn from_file(path: impl AsRef<Path>, scopes: &[String]) -> crate::error::Result<Credentials> {
    debug!("Reading credentials from file: {:?}", path.as_ref());
    let buf = fs::read(path).map_err(crate::error::ErrorKind::CredentialsFile)?;
    from_json(&buf, scopes)
}

#[inline]
fn httpc_post(url: &str) -> reqwest::RequestBuilder {
    reqwest::Client::new()
        .post(url)
        .header(reqwest::header::USER_AGENT, crate::GCLOUD_SDK_USER_AGENT)
}

#[derive(Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct IamCredentialsGenerateAccessToken {
    pub scope: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct IamCredentialsTokenResponse {
    access_token: SecretValue,
    #[allow(unused)]
    expire_time: chrono::DateTime<chrono::Utc>,
}

mod jwt {
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};

    use std::{convert::TryFrom, time::SystemTime};

    use crate::token_source::{
        credentials::{httpc_post, ServiceAccount},
        Token, TokenResponse,
    };

    #[derive(Debug, serde::Serialize)]
    struct Claims<'a> {
        iss: &'a str,
        scope: &'a str,
        aud: &'a str,
        iat: u64,
        exp: u64,
    }

    // If client machine's time is in the future according
    // to Google servers, an access token will not be issued.
    fn issued_at() -> u64 {
        SystemTime::UNIX_EPOCH.elapsed().unwrap().as_secs() - 10
    }

    // https://cloud.google.com/iot/docs/concepts/device-security#security_standards
    fn header(typ: &str, prv_key_id: &str) -> Header {
        Header {
            typ: Some(typ.into()),
            alg: Algorithm::RS256,
            kid: Some(prv_key_id.into()),
            ..Header::default()
        }
    }

    #[derive(Debug, serde::Serialize)]
    struct Payload<'a> {
        grant_type: &'a str,
        assertion: &'a str,
    }

    const DEFAULT_EXPIRE: u64 = 60 * 60;

    pub async fn token(sa: &ServiceAccount) -> crate::error::Result<Token> {
        let iat = issued_at();
        let claims = Claims {
            iss: &sa.client_email,
            scope: &sa.scopes.join(" "),
            aud: &sa.token_uri,
            iat,
            exp: iat + DEFAULT_EXPIRE,
        };
        let header = header("JWT", &sa.private_key_id);
        let key = EncodingKey::from_rsa_pem(sa.private_key.as_sensitive_bytes())?;
        let assertion = &encode(&header, &claims, &key)?;

        let req = httpc_post(&sa.token_uri).form(&Payload {
            grant_type: "urn:ietf:params:oauth:grant-type:jwt-bearer",
            assertion,
        });
        let resp = req.send().await?;
        if resp.status().is_success() {
            let resp = resp.json::<TokenResponse>().await?;
            Token::try_from(resp)
        } else {
            Err(crate::error::ErrorKind::HttpStatus(resp.status()).into())
        }
    }
}

mod oauth2 {
    use std::convert::TryFrom;

    use crate::token_source::{
        credentials::{httpc_post, User},
        Token, TokenResponse,
    };

    #[derive(serde::Serialize)]
    struct Payload<'a> {
        client_id: &'a str,
        client_secret: &'a str,
        grant_type: &'a str,
        refresh_token: &'a str,
    }

    // https://github.com/golang/oauth2/blob/bf48bf16ab8d622ce64ec6ce98d2c98f916b6303/google/google.go#L21-L26
    const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
    const GRANT_TYPE: &str = "refresh_token";

    pub async fn token(user: &User) -> crate::error::Result<Token> {
        fetch_token(TOKEN_URL, user).await
    }

    pub(super) async fn fetch_token(url: &str, user: &User) -> crate::error::Result<Token> {
        let req = httpc_post(url).form(&Payload {
            client_id: &user.client_id,
            client_secret: user.client_secret.as_sensitive_str(),
            grant_type: GRANT_TYPE,
            // The refresh token is not included in the response from google's server,
            // so it always uses the specified refresh token from the file.
            refresh_token: user.refresh_token.as_sensitive_str(),
        });
        let resp = req.send().await?;
        if resp.status().is_success() {
            let resp = resp.json::<TokenResponse>().await?;
            Token::try_from(resp)
        } else {
            Err(crate::error::ErrorKind::HttpStatus(resp.status()).into())
        }
    }
}

mod external_account {
    use crate::GCP_DEFAULT_SCOPES;
    use secret_vault_value::SecretValue;
    use std::convert::TryFrom;
    use tracing::*;

    use crate::token_source::credentials::{
        ExternalAccount, IamCredentialsGenerateAccessToken, IamCredentialsTokenResponse,
    };
    use crate::token_source::{ext_creds_source, Token, TokenResponse};

    #[derive(Debug, PartialEq, Eq, serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct StsRequest {
        pub grant_type: String,
        pub audience: String,
        pub requested_token_type: String,
        pub subject_token_type: String,
        pub subject_token: SecretValue,
        pub scope: String,
    }

    // This implements source described here: https://google.aip.dev/auth/4117
    pub async fn token(external_account: &ExternalAccount) -> crate::error::Result<Token> {
        debug!(
            "Detected external account {}",
            &external_account.subject_token_type
        );
        let client = reqwest::Client::new();
        let subject_token = ext_creds_source::subject_token(&client, external_account).await?;

        let scopes = if external_account.service_account_impersonation_url.is_some() {
            GCP_DEFAULT_SCOPES.clone()
        } else {
            external_account.scopes.clone()
        };

        let sts_request_body = StsRequest {
            grant_type: "urn:ietf:params:oauth:grant-type:token-exchange".to_string(),
            audience: external_account.audience.to_string(),
            requested_token_type: "urn:ietf:params:oauth:token-type:access_token".to_string(),
            subject_token_type: external_account.subject_token_type.clone(),
            subject_token,
            scope: scopes.join(" "),
        };

        let sts_http_request = client
            .post(external_account.token_url.as_str())
            .header(reqwest::header::CONTENT_TYPE, "application/json");
        let sts_http_response = sts_http_request.json(&sts_request_body).send().await?;

        if sts_http_response.status().is_success() {
            let resp = sts_http_response.json::<TokenResponse>().await?;
            let sts_token = Token::try_from(resp)?;
            if let Some(service_account_impersonation_url) =
                &external_account.service_account_impersonation_url
            {
                debug!(
                    "Using impersonation URL {}",
                    service_account_impersonation_url
                );

                let iam_generate_token_request = client
                    .post(service_account_impersonation_url)
                    .header(reqwest::header::AUTHORIZATION, sts_token.header_value())
                    .header(reqwest::header::CONTENT_TYPE, "application/json");

                let iam_generate_body = IamCredentialsGenerateAccessToken {
                    scope: external_account.scopes.clone().join(" "),
                };

                let iam_generate_token_response = iam_generate_token_request
                    .json(&iam_generate_body)
                    .send()
                    .await?;
                if iam_generate_token_response.status().is_success() {
                    let iam_generate_resp_body = iam_generate_token_response
                        .json::<IamCredentialsTokenResponse>()
                        .await?;

                    Ok(Token {
                        token: iam_generate_resp_body.access_token,
                        ..sts_token
                    })
                } else {
                    let status = iam_generate_token_response.status();
                    let err_body = iam_generate_token_response.text().await?;
                    let err_text = format!("Unable to receive subject using external impersonation url: {}. HTTP: {} {}", &external_account.token_url, status, err_body);
                    Err(crate::error::ErrorKind::ExternalCredsSourceError(err_text).into())
                }
            } else {
                Ok(sts_token)
            }
        } else {
            let status = sts_http_response.status();
            let err_body = sts_http_response.text().await?;
            let err_text = format!(
                "Unable to receive subject using external url: {}. HTTP: {} {}",
                &external_account.token_url, status, err_body
            );
            Err(crate::error::ErrorKind::ExternalCredsSourceError(err_text).into())
        }
    }
}

mod impersonate_account {
    use crate::token_source::credentials::{
        jwt, oauth2, IamCredentialsGenerateAccessToken, IamCredentialsTokenResponse,
        ServiceAccountImpersonation, ServiceAccountImpersonationSourceCredentials,
    };
    use crate::{Token, GCP_DEFAULT_SCOPES};
    use tracing::*;

    pub async fn token(
        impersonate_account: &ServiceAccountImpersonation,
    ) -> crate::error::Result<Token> {
        let initial_token = match &impersonate_account.source_credentials {
            ServiceAccountImpersonationSourceCredentials::ServiceAccount(sa) => {
                jwt::token(sa).await
            }
            ServiceAccountImpersonationSourceCredentials::User(user) => oauth2::token(user).await,
        }?;

        debug!(
            "Using impersonation URL {}",
            impersonate_account.service_account_impersonation_url
        );

        let client = reqwest::Client::new();

        let iam_generate_body = IamCredentialsGenerateAccessToken {
            scope: GCP_DEFAULT_SCOPES.clone().join(" "),
        };

        let iam_generate_token_response = client
            .post(&impersonate_account.service_account_impersonation_url)
            .header(reqwest::header::AUTHORIZATION, initial_token.header_value())
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .json(&iam_generate_body)
            .send()
            .await?;

        if iam_generate_token_response.status().is_success() {
            let iam_generate_resp_body = iam_generate_token_response
                .json::<IamCredentialsTokenResponse>()
                .await?;

            Ok(Token {
                token: iam_generate_resp_body.access_token,
                ..initial_token
            })
        } else {
            let status = iam_generate_token_response.status();
            let err_body = iam_generate_token_response.text().await?;
            let err_text = format!(
                "Unable to receive subject using impersonation url: {}. HTTP: {} {}",
                &impersonate_account.service_account_impersonation_url, status, err_body
            );
            Err(crate::error::ErrorKind::ExternalCredsSourceError(err_text).into())
        }
    }
}
