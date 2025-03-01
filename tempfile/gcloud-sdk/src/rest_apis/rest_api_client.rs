use crate::{GoogleAuthTokenGenerator, TokenSourceType, GCP_DEFAULT_SCOPES};
use async_trait::async_trait;
use hyper::Uri;
use reqwest::{IntoUrl, Method, Request, RequestBuilder};
use std::sync::Arc;

#[derive(Clone)]
pub struct GoogleRestApi {
    pub client: reqwest::Client,
    pub token_generator: Arc<GoogleAuthTokenGenerator>,
}

impl GoogleRestApi {
    pub async fn new() -> crate::error::Result<Self> {
        Self::with_token_source(TokenSourceType::Default, GCP_DEFAULT_SCOPES.clone()).await
    }

    pub async fn with_token_source(
        token_source_type: TokenSourceType,
        token_scopes: Vec<String>,
    ) -> crate::error::Result<Self> {
        let client = reqwest::Client::new();
        Self::with_client_token_source(client, token_source_type, token_scopes).await
    }

    pub async fn with_client_token_source(
        client: reqwest::Client,
        token_source_type: TokenSourceType,
        token_scopes: Vec<String>,
    ) -> crate::error::Result<Self> {
        let token_generator =
            GoogleAuthTokenGenerator::new(token_source_type, token_scopes).await?;

        Ok(Self {
            client,
            token_generator: Arc::new(token_generator),
        })
    }

    pub async fn with_google_token<'a>(
        &self,
        request: RequestBuilder,
    ) -> crate::error::Result<RequestBuilder> {
        let token = self.token_generator.create_token().await?;
        Ok(request.header(reqwest::header::AUTHORIZATION, token.header_value()))
    }

    pub async fn get<U: IntoUrl>(&self, url: U) -> crate::error::Result<RequestBuilder> {
        self.with_google_token(self.client.request(Method::GET, url))
            .await
    }
    pub async fn post<U: IntoUrl>(&self, url: U) -> crate::error::Result<RequestBuilder> {
        self.with_google_token(self.client.request(Method::POST, url))
            .await
    }
    pub async fn put<U: IntoUrl>(&self, url: U) -> crate::error::Result<RequestBuilder> {
        self.with_google_token(self.client.request(Method::PUT, url))
            .await
    }
    pub async fn patch<U: IntoUrl>(&self, url: U) -> crate::error::Result<RequestBuilder> {
        self.with_google_token(self.client.request(Method::PATCH, url))
            .await
    }
    pub async fn delete<U: IntoUrl>(&self, url: U) -> crate::error::Result<RequestBuilder> {
        self.with_google_token(self.client.request(Method::DELETE, url))
            .await
    }
    pub async fn head<U: IntoUrl>(&self, url: U) -> crate::error::Result<RequestBuilder> {
        self.with_google_token(self.client.request(Method::HEAD, url))
            .await
    }
}

pub fn create_hyper_uri_with_params<'p, PT, TS>(url_str: &str, params: &'p PT) -> Uri
where
    PT: std::iter::IntoIterator<Item = (&'p str, Option<&'p TS>)> + Clone,
    TS: std::string::ToString + 'p,
{
    let url_query_params: Vec<(String, String)> = params
        .clone()
        .into_iter()
        .filter_map(|(k, vo)| vo.map(|v| (k.to_string(), v.to_string())))
        .collect();

    let url: url::Url = url::Url::parse_with_params(url_str, url_query_params)
        .unwrap()
        .as_str()
        .parse()
        .unwrap();

    url.as_str().parse().unwrap()
}
