//! Wrap different providers

use std::{fmt::Debug, path::PathBuf, str::FromStr, sync::Arc, time::Duration};

use async_trait::async_trait;
use ethers_core::types::U256;
use ethers_providers::{Authorization, ConnectionDetails, Http, HttpClientError, HttpRateLimitRetryPolicy, Ipc, JsonRpcClient, JsonRpcError, JwtAuth, JwtKey, ProviderError, PubsubClient, RetryClient, RetryClientBuilder, RetryPolicy, RpcError, Ws};
use reqwest::{
    header::{HeaderName, HeaderValue},
    Url,
};
use serde::{de::DeserializeOwned, Serialize};
use thiserror::Error;
use tokio::sync::RwLock;

/// Enum representing a the client types supported by the runtime provider
#[derive(Debug)]
enum InnerClient {
    /// HTTP client
    Http(RetryClient<Http>),
    /// WebSocket client
    Ws(Ws),
    /// IPC client
    Ipc(Ipc),
}

/// Error type for the runtime provider
#[derive(Error, Debug)]
pub enum RuntimeClientError {
    /// Internal provider error
    #[error(transparent)]
    ProviderError(ProviderError),

    /// Failed to lock the client
    #[error("Failed to lock the client")]
    LockError,

    /// Invalid URL scheme
    #[error("URL scheme is not supported: {0}")]
    BadScheme(String),

    /// Invalid HTTP header
    #[error("Invalid HTTP header: {0}")]
    BadHeader(String),

    /// Invalid file path
    #[error("Invalid IPC file path: {0}")]
    BadPath(String),
}

impl RpcError for RuntimeClientError {
    fn as_error_response(&self) -> Option<&JsonRpcError> {
        match self {
            RuntimeClientError::ProviderError(err) => err.as_error_response(),
            _ => None,
        }
    }

    fn as_serde_error(&self) -> Option<&serde_json::Error> {
        match self {
            RuntimeClientError::ProviderError(e) => e.as_serde_error(),
            _ => None,
        }
    }
}

impl From<RuntimeClientError> for ProviderError {
    fn from(src: RuntimeClientError) -> Self {
        match src {
            RuntimeClientError::ProviderError(err) => err,
            _ => ProviderError::JsonRpcClientError(Box::new(src)),
        }
    }
}

/// A provider that connects on first request allowing handling of different provider types at
/// runtime
#[derive(Clone, Debug, Error)]
pub struct RuntimeClient {
    client: Arc<RwLock<Option<InnerClient>>>,
    url: Url,
    max_retry: u32,
    timeout_retry: u32,
    initial_backoff: u64,
    timeout: Duration,
    /// available CUPS
    compute_units_per_second: u64,
    jwt: Option<String>,
    headers: Vec<String>,
    retry_policy: Arc<RwLock<Option<Box<dyn RetryPolicy<HttpClientError>>>>>
}

/// Builder for RuntimeClient
pub struct RuntimeClientBuilder {
    url: Url,
    max_retry: u32,
    timeout_retry: u32,
    initial_backoff: u64,
    timeout: Duration,
    /// available CUPS
    compute_units_per_second: u64,
    jwt: Option<String>,
    headers: Vec<String>,
}

impl ::core::fmt::Display for RuntimeClient {
    fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
        write!(f, "RuntimeClient")
    }
}

fn build_auth(jwt: String) -> eyre::Result<Authorization> {
    // Decode jwt from hex, then generate claims (iat with current timestamp)
    let jwt = hex::decode(jwt)?;
    let secret = JwtKey::from_slice(&jwt).map_err(|err| eyre::eyre!("Invalid JWT: {}", err))?;
    let auth = JwtAuth::new(secret, None, None);
    let token = auth.generate_token()?;

    // Essentially unrolled ethers-rs new_with_auth to accomodate the custom timeout
    let auth = Authorization::Bearer(token);

    Ok(auth)
}

impl RuntimeClient {
    async fn connect(&self) -> Result<InnerClient, RuntimeClientError> {
        match self.url.scheme() {
            "http" | "https" => {
                let mut client_builder = reqwest::Client::builder().timeout(self.timeout);
                let mut headers = reqwest::header::HeaderMap::new();

                if let Some(jwt) = self.jwt.as_ref() {
                    let auth = build_auth(jwt.clone()).map_err(|err| {
                        RuntimeClientError::ProviderError(ProviderError::CustomError(
                            err.to_string(),
                        ))
                    })?;

                    let mut auth_value: HeaderValue = HeaderValue::from_str(&auth.to_string())
                        .expect("Header should be valid string");
                    auth_value.set_sensitive(true);

                    headers.insert(reqwest::header::AUTHORIZATION, auth_value);
                };

                for header in self.headers.iter() {
                    let make_err = || RuntimeClientError::BadHeader(header.to_string());

                    let (key, val) = header.split_once(':').ok_or_else(make_err)?;

                    headers.insert(
                        HeaderName::from_str(key.trim()).map_err(|_| make_err())?,
                        HeaderValue::from_str(val.trim()).map_err(|_| make_err())?,
                    );
                }

                client_builder = client_builder.default_headers(headers);

                let client = client_builder
                    .build()
                    .map_err(|e| RuntimeClientError::ProviderError(e.into()))?;
                let provider = Http::new_with_client(self.url.clone(), client);

                let policy = match self.get_retry_policy().await {
                    Some(policy) => policy,
                    None => Box::new(HttpRateLimitRetryPolicy)
                };

                #[allow(clippy::box_default)]
                let provider = RetryClientBuilder::default()
                    .initial_backoff(Duration::from_millis(self.initial_backoff))
                    .rate_limit_retries(self.max_retry)
                    .timeout_retries(self.timeout_retry)
                    .compute_units_per_second(self.compute_units_per_second)
                    .build(provider, policy);

                Ok(InnerClient::Http(provider))
            }
            "ws" | "wss" => {
                let auth: Option<Authorization> =
                    self.jwt.as_ref().and_then(|jwt| build_auth(jwt.clone()).ok());
                let connection_details = ConnectionDetails::new(self.url.as_str(), auth);

                let client =
                    Ws::connect_with_reconnects(connection_details, self.max_retry as usize)
                        .await
                        .map_err(|e| RuntimeClientError::ProviderError(e.into()))?;

                Ok(InnerClient::Ws(client))
            }
            "file" => {
                let path = url_to_file_path(&self.url)
                    .map_err(|_| RuntimeClientError::BadPath(self.url.to_string()))?;

                let client = Ipc::connect(path)
                    .await
                    .map_err(|e| RuntimeClientError::ProviderError(e.into()))?;

                Ok(InnerClient::Ipc(client))
            }
            _ => Err(RuntimeClientError::BadScheme(self.url.to_string())),
        }
    }

    async fn get_retry_policy(&self) -> Option<Box<dyn RetryPolicy<HttpClientError>>> {
        self.retry_policy.write().await.take()
    }
}

impl RuntimeClientBuilder {
    /// Create new RuntimeClientBuilder
    pub fn new(
        url: Url,
        max_retry: u32,
        timeout_retry: u32,
        initial_backoff: u64,
        timeout: Duration,
        compute_units_per_second: u64,
    ) -> Self {
        Self {
            url,
            max_retry,
            timeout,
            timeout_retry,
            initial_backoff,
            compute_units_per_second,
            jwt: None,
            headers: vec![],
        }
    }

    /// Set jwt to use with RuntimeClient
    pub fn with_jwt(mut self, jwt: Option<String>) -> Self {
        self.jwt = jwt;
        self
    }

    /// Set http headers to use with RuntimeClient
    /// Only works with http/https schemas
    pub fn with_headers(mut self, headers: Vec<String>) -> Self {
        self.headers = headers;
        self
    }

    /// Builds RuntimeClient instance
    pub fn build(self) -> RuntimeClient {
        RuntimeClient {
            client: Arc::new(RwLock::new(None)),
            url: self.url,
            max_retry: self.max_retry,
            timeout_retry: self.timeout_retry,
            initial_backoff: self.initial_backoff,
            timeout: self.timeout,
            compute_units_per_second: self.compute_units_per_second,
            jwt: self.jwt,
            headers: self.headers,
            retry_policy: Arc::new(RwLock::new(None)),
        }
    }

    /// Build a RuntimeClient with a custom retry policy
    pub fn build_with_retry_policy(self, policy: Box<dyn RetryPolicy<HttpClientError>>) -> RuntimeClient {
        RuntimeClient {
            client: Arc::new(RwLock::new(None)),
            url: self.url,
            max_retry: self.max_retry,
            timeout_retry: self.timeout_retry,
            initial_backoff: self.initial_backoff,
            timeout: self.timeout,
            compute_units_per_second: self.compute_units_per_second,
            jwt: self.jwt,
            headers: self.headers,
            retry_policy: Arc::new(RwLock::new(Some(policy)))
        }
    }
}

#[cfg(windows)]
fn url_to_file_path(url: &Url) -> Result<PathBuf, ()> {
    const PREFIX: &str = "file:///pipe/";

    let url_str = url.as_str();

    if url_str.starts_with(PREFIX) {
        let pipe_name = &url_str[PREFIX.len()..];
        let pipe_path = format!(r"\\.\pipe\{}", pipe_name);
        return Ok(PathBuf::from(pipe_path))
    }

    url.to_file_path()
}

#[cfg(not(windows))]
fn url_to_file_path(url: &Url) -> Result<PathBuf, ()> {
    url.to_file_path()
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl JsonRpcClient for RuntimeClient {
    type Error = RuntimeClientError;

    #[allow(implied_bounds_entailment)]
    async fn request<T, R>(&self, method: &str, params: T) -> Result<R, Self::Error>
    where
        T: Debug + Serialize + Send + Sync,
        R: DeserializeOwned + Send,
    {
        if self.client.read().await.is_none() {
            let mut w = self.client.write().await;
            *w = Some(
                self.connect().await.map_err(|e| RuntimeClientError::ProviderError(e.into()))?,
            );
        }

        let res = match self.client.read().await.as_ref().unwrap() {
            InnerClient::Http(http) => RetryClient::request(http, method, params)
                .await
                .map_err(|e| RuntimeClientError::ProviderError(e.into())),
            InnerClient::Ws(ws) => JsonRpcClient::request(ws, method, params)
                .await
                .map_err(|e| RuntimeClientError::ProviderError(e.into())),
            InnerClient::Ipc(ipc) => JsonRpcClient::request(ipc, method, params)
                .await
                .map_err(|e| RuntimeClientError::ProviderError(e.into())),
        }?;
        Ok(res)
    }
}

// We can also implement [`PubsubClient`] for our dynamic provider.
impl PubsubClient for RuntimeClient {
    // Since both `Ws` and `Ipc`'s `NotificationStream` associated type is the same,
    // we can simply return one of them.
    type NotificationStream = <Ws as PubsubClient>::NotificationStream;

    fn subscribe<T: Into<U256>>(&self, id: T) -> Result<Self::NotificationStream, Self::Error> {
        match self.client.try_read().map_err(|_| RuntimeClientError::LockError)?.as_ref().unwrap() {
            InnerClient::Http(_) => {
                Err(RuntimeClientError::ProviderError(ProviderError::UnsupportedRPC))
            }
            InnerClient::Ws(client) => Ok(PubsubClient::subscribe(client, id)
                .map_err(|e| RuntimeClientError::ProviderError(e.into()))?),
            InnerClient::Ipc(client) => Ok(PubsubClient::subscribe(client, id)
                .map_err(|e| RuntimeClientError::ProviderError(e.into()))?),
        }
    }

    fn unsubscribe<T: Into<U256>>(&self, id: T) -> Result<(), Self::Error> {
        match self.client.try_read().map_err(|_| (RuntimeClientError::LockError))?.as_ref().unwrap()
        {
            InnerClient::Http(_) => {
                Err(RuntimeClientError::ProviderError(ProviderError::UnsupportedRPC))
            }
            InnerClient::Ws(client) => Ok(PubsubClient::unsubscribe(client, id)
                .map_err(|e| RuntimeClientError::ProviderError(e.into()))?),
            InnerClient::Ipc(client) => Ok(PubsubClient::unsubscribe(client, id)
                .map_err(|e| RuntimeClientError::ProviderError(e.into()))?),
        }
    }
}
