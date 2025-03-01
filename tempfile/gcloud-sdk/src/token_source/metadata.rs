use url::form_urlencoded::Serializer;

use async_trait::async_trait;
use hyper::http::uri::PathAndQuery;
use secret_vault_value::SecretValue;
use std::convert::TryFrom;
use std::str::FromStr;
use tracing::*;

use crate::token_source::gce::gce_metadata_client::GceMetadataClient;
use crate::token_source::{BoxSource, Source, Token, TokenResponse};

#[derive(Debug)]
pub struct Metadata {
    account: String,
    scopes: Vec<String>,
    client: GceMetadataClient,
}

impl Metadata {
    pub fn new(scopes: impl Into<Vec<String>>) -> Self {
        Self::with_account(scopes, "default".to_string())
    }

    pub fn with_account(scopes: impl Into<Vec<String>>, account: String) -> Self {
        Self {
            account,
            scopes: scopes.into(),
            client: GceMetadataClient::new(),
        }
    }

    pub async fn init(&mut self) -> bool {
        self.client.init().await
    }

    fn uri_suffix(&self) -> String {
        let query = if self.scopes.is_empty() {
            String::new()
        } else {
            Serializer::new(String::new())
                .append_pair("scopes", &self.scopes.join(","))
                .finish()
        };
        format!("instance/service-accounts/{}/token?{}", self.account, query)
    }

    pub async fn detect_google_project_id(&self) -> Option<String> {
        match PathAndQuery::from_str("/computeMetadata/v1/project/project-id") {
            Ok(url) if self.client.is_available() => {
                trace!("Receiving Project ID token from Metadata Server");
                self.client
                    .get(url)
                    .await
                    .ok()
                    .map(|project_id| project_id.trim().to_string())
            }
            Ok(_) => None,
            Err(e) => {
                error!("Internal URL format error: '{}'", e);
                None
            }
        }
    }

    pub async fn id_token(&self, audience: &str) -> crate::error::Result<SecretValue> {
        let url = PathAndQuery::from_str(
            format!(
                "/computeMetadata/v1/instance/service-accounts/{}/identity?audience={}",
                self.account, audience
            )
            .as_str(),
        )?;
        trace!(
            "Receiving a new ID token from Metadata Server using '{}'",
            url
        );
        let resp = self.client.get(url).await?;
        Ok(SecretValue::from(resp))
    }
}

impl From<Metadata> for BoxSource {
    fn from(v: Metadata) -> Self {
        Box::new(v)
    }
}

#[async_trait]
impl Source for Metadata {
    async fn token(&self) -> crate::error::Result<Token> {
        let url =
            PathAndQuery::from_str(format!("/computeMetadata/v1/{}", self.uri_suffix()).as_str())?;
        trace!("Receiving a new token from Metadata Server using '{}'", url);

        let resp_str = self.client.get(url).await?;
        let resp = TokenResponse::try_from(resp_str.as_str())?;
        Token::try_from(resp)
    }
}

pub async fn from_metadata(
    scopes: &[String],
    account: String,
) -> crate::error::Result<Option<Metadata>> {
    let mut metadata = Metadata::with_account(scopes, account);

    if metadata.init().await {
        Ok(Some(metadata))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_metadata_uri_suffix() {
        let m = Metadata::new(Vec::new());
        assert_eq!(m.uri_suffix(), "instance/service-accounts/default/token?");

        let m = Metadata::new(crate::GCP_DEFAULT_SCOPES.clone());

        assert_eq!(
            m.uri_suffix(),
            "instance/service-accounts/default/token?scopes=https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fcloud-platform"
        );

        let m = Metadata::new(vec!["scope1".into(), "scope2".into()]);
        assert_eq!(
            m.uri_suffix(),
            "instance/service-accounts/default/token?scopes=scope1%2Cscope2",
        );
    }
}
