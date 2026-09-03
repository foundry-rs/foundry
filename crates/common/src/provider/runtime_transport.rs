//! Runtime transport that connects on first request, which can take either of an HTTP,
//! WebSocket, or IPC transport. Retries are handled by a client layer (e.g.,
//! `RetryBackoffLayer`) when used.

use crate::{
    DEFAULT_USER_AGENT, REQUEST_TIMEOUT,
    provider::{
        mpp::transport::{LazyMppHttpTransport, lazy_mpp_ws_connect},
        redact_url,
    },
};
use alloy_json_rpc::{RequestPacket, ResponsePacket};
use alloy_pubsub::{PubSubConnect, PubSubFrontend};
use alloy_rpc_types_engine::{Claims, JwtSecret};
use alloy_transport::{
    Authorization, BoxTransport, TransportError, TransportErrorKind, TransportFut,
    utils::guess_local_url,
};
use alloy_transport_ipc::IpcConnect;
use alloy_transport_ws::WsConnect;
use regex::{Captures, Regex};
use reqwest::header::{HeaderName, HeaderValue};
use std::{
    error::Error as StdError,
    fmt,
    path::PathBuf,
    str::FromStr,
    sync::{Arc, LazyLock},
};
use thiserror::Error;
use tokio::sync::RwLock;
use tower::Service;
use url::Url;

/// Known MPP-enabled RPC host suffixes.
///
/// Endpoints matching these patterns always use the MPP WebSocket transport,
/// regardless of whether local MPP keys have been discovered.
const KNOWN_MPP_HOSTS: &[&str] = &[".mpp.tempo.xyz", ".mpp.moderato.tempo.xyz"];

static HTTP_URL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)https?://[^\s<>"']+"#).expect("valid URL regex"));

/// An enum representing the different transports that can be used to connect to a runtime.
/// Only meant to be used internally by [RuntimeTransport].
#[derive(Clone, Debug)]
pub enum InnerTransport {
    /// HTTP transport with lazy MPP 402 handling.
    ///
    /// For known Tempo endpoints, the MPP layer additionally runs the
    /// `wallet.tempo.xyz` device-code flow on a 402 when no local access key
    /// is configured (see [`crate::tempo::ensure_access_key`]).
    Http(LazyMppHttpTransport),
    /// WebSocket transport
    Ws(PubSubFrontend),
    /// IPC transport
    Ipc(PubSubFrontend),
}

/// Error type for the runtime transport.
#[derive(Error, Debug)]
pub enum RuntimeTransportError {
    /// Internal transport error
    #[error("Internal transport error: {0} with {1}")]
    TransportError(TransportError, String),

    /// Invalid URL scheme
    #[error("URL scheme is not supported: {0}")]
    BadScheme(String),

    /// Invalid HTTP header
    #[error("Invalid HTTP header: {0}")]
    BadHeader(String),

    /// Invalid file path
    #[error("Invalid IPC file path: {0}")]
    BadPath(String),

    /// Invalid construction of Http provider
    #[error(transparent)]
    HttpConstructionError(#[from] reqwest::Error),

    /// Invalid JWT
    #[error("Invalid JWT: {0}")]
    InvalidJwt(String),
}

/// Runtime transport that only connects on first request.
///
/// A runtime transport is a custom [`alloy_transport::Transport`] that only connects when the
/// *first* request is made. When the first request is made, it will connect to the runtime using
/// either an HTTP WebSocket, or IPC transport depending on the URL used.
/// Retries for rate-limiting and timeout-related errors are handled by an external
/// client layer (e.g., `RetryBackoffLayer`) when configured.
#[derive(Clone, Debug)]
pub struct RuntimeTransport {
    /// The inner actual transport used.
    inner: Arc<RwLock<Option<InnerTransport>>>,
    /// The URL to connect to.
    url: Url,
    /// The headers to use for requests.
    headers: Vec<String>,
    /// The JWT to use for requests.
    jwt: Option<String>,
    /// The timeout for requests.
    timeout: std::time::Duration,
    /// Whether to accept invalid certificates.
    accept_invalid_certs: bool,
    /// Whether to disable automatic proxy detection.
    no_proxy: bool,
}

/// A builder for [RuntimeTransport].
#[derive(Debug)]
pub struct RuntimeTransportBuilder {
    url: Url,
    headers: Vec<String>,
    jwt: Option<String>,
    timeout: std::time::Duration,
    accept_invalid_certs: bool,
    no_proxy: bool,
}

impl RuntimeTransportBuilder {
    /// Create a new builder with the given URL.
    pub const fn new(url: Url) -> Self {
        Self {
            url,
            headers: vec![],
            jwt: None,
            timeout: REQUEST_TIMEOUT,
            accept_invalid_certs: false,
            no_proxy: false,
        }
    }

    /// Set the URL for the transport.
    pub fn with_headers(mut self, headers: Vec<String>) -> Self {
        self.headers = headers;
        self
    }

    /// Set the JWT for the transport.
    pub fn with_jwt(mut self, jwt: Option<String>) -> Self {
        self.jwt = jwt;
        self
    }

    /// Set the timeout for the transport.
    pub const fn with_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Set whether to accept invalid certificates.
    pub const fn accept_invalid_certs(mut self, accept_invalid_certs: bool) -> Self {
        self.accept_invalid_certs = accept_invalid_certs;
        self
    }

    /// Set whether to disable automatic proxy detection.
    ///
    /// This can help in sandboxed environments (e.g., Cursor IDE sandbox, macOS App Sandbox)
    /// where system proxy detection via SCDynamicStore causes crashes.
    pub const fn no_proxy(mut self, no_proxy: bool) -> Self {
        self.no_proxy = no_proxy;
        self
    }

    /// Builds the [RuntimeTransport] and returns it in a disconnected state.
    /// The runtime transport will then connect when the first request happens.
    pub fn build(self) -> RuntimeTransport {
        RuntimeTransport {
            inner: Arc::new(RwLock::new(None)),
            url: self.url,
            headers: self.headers,
            jwt: self.jwt,
            timeout: self.timeout,
            accept_invalid_certs: self.accept_invalid_certs,
            no_proxy: self.no_proxy,
        }
    }
}

impl fmt::Display for RuntimeTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RuntimeTransport {}", redact_url(self.url.as_str()))
    }
}

impl RuntimeTransport {
    /// Connects the underlying transport, depending on the URL scheme.
    pub async fn connect(&self) -> Result<InnerTransport, RuntimeTransportError> {
        match self.url.scheme() {
            "http" | "https" => self.connect_http(),
            "ws" | "wss" => self.connect_ws().await,
            "file" => self.connect_ipc().await,
            _ => Err(RuntimeTransportError::BadScheme(self.url.scheme().to_string())),
        }
    }

    fn reqwest_headers(&self) -> Result<reqwest::header::HeaderMap, RuntimeTransportError> {
        let mut headers = reqwest::header::HeaderMap::new();

        // If there's a JWT, add it to the headers if we can decode it.
        if let Some(jwt) = self.jwt.clone() {
            let auth =
                build_auth(jwt).map_err(|e| RuntimeTransportError::InvalidJwt(e.to_string()))?;

            let mut auth_value: HeaderValue =
                HeaderValue::from_str(&auth.to_string()).expect("Header should be valid string");
            auth_value.set_sensitive(true);

            headers.insert(reqwest::header::AUTHORIZATION, auth_value);
        };

        // Add any custom headers.
        for header in &self.headers {
            let make_err = || RuntimeTransportError::BadHeader(header.clone());

            let (key, val) = header.split_once(':').ok_or_else(make_err)?;

            headers.insert(
                HeaderName::from_str(key.trim()).map_err(|_| make_err())?,
                HeaderValue::from_str(val.trim()).map_err(|_| make_err())?,
            );
        }

        if !headers.contains_key(reqwest::header::USER_AGENT) {
            headers.insert(
                reqwest::header::USER_AGENT,
                HeaderValue::from_str(DEFAULT_USER_AGENT)
                    .expect("User-Agent should be valid string"),
            );
        }

        // If MPP_API_KEY is set, attach it as x-api-key for gated MPP proxies.
        // Does not override an explicit x-api-key header from the user.
        if !headers.contains_key(HeaderName::from_static("x-api-key"))
            && let Ok(api_key) = std::env::var("MPP_API_KEY")
        {
            let api_key = api_key.trim();
            if !api_key.is_empty() {
                let mut value = HeaderValue::from_str(api_key)
                    .map_err(|_| RuntimeTransportError::BadHeader("MPP_API_KEY".to_string()))?;
                value.set_sensitive(true);
                headers.insert(HeaderName::from_static("x-api-key"), value);
            }
        }

        Ok(headers)
    }

    fn reqwest_client_with_headers(
        &self,
        headers: reqwest::header::HeaderMap,
    ) -> Result<reqwest::Client, RuntimeTransportError> {
        let mut client_builder = reqwest::Client::builder()
            .timeout(self.timeout)
            .danger_accept_invalid_certs(self.accept_invalid_certs);

        // Disable automatic proxy detection if requested. This helps in sandboxed environments
        // (e.g., Cursor IDE sandbox, macOS App Sandbox) where system proxy detection via
        // SCDynamicStore causes crashes. See: https://github.com/foundry-rs/foundry/issues/12733
        if self.no_proxy || guess_local_url(self.url.as_str()) {
            client_builder = client_builder.no_proxy();
        }

        client_builder = client_builder.default_headers(headers);

        Ok(client_builder.build()?)
    }

    /// Creates a new reqwest client from this transport.
    pub fn reqwest_client(&self) -> Result<reqwest::Client, RuntimeTransportError> {
        self.reqwest_client_with_headers(self.reqwest_headers()?)
    }

    /// Connects to an HTTP transport with lazy MPP 402 handling.
    fn connect_http(&self) -> Result<InnerTransport, RuntimeTransportError> {
        let headers = self.reqwest_headers()?;
        let client = self.reqwest_client_with_headers(headers.clone())?;
        Ok(InnerTransport::Http(LazyMppHttpTransport::lazy(client, self.url.clone(), headers)))
    }

    /// Connects to a WS transport.
    ///
    /// Uses the canonical Alloy MPP WebSocket transport when the endpoint is a
    /// known MPP service.
    /// Otherwise falls back to alloy's plain [`WsConnect`] with zero overhead.
    async fn connect_ws(&self) -> Result<InnerTransport, RuntimeTransportError> {
        let auth = self.jwt.as_ref().and_then(|jwt| build_auth(jwt.clone()).ok());

        let service = if is_known_mpp_endpoint(&self.url) {
            let mut ws = lazy_mpp_ws_connect(&self.url);
            if let Some(auth) = auth {
                ws = ws.with_auth(auth);
            }
            ws.into_service().await.map_err(|e| {
                RuntimeTransportError::TransportError(e, redact_url(self.url.as_str()))
            })?
        } else {
            let mut ws = WsConnect::new(self.url.to_string());
            if let Some(auth) = auth {
                ws = ws.with_auth(auth);
            }
            ws.into_service().await.map_err(|e| {
                RuntimeTransportError::TransportError(e, redact_url(self.url.as_str()))
            })?
        };

        Ok(InnerTransport::Ws(service))
    }

    /// Connects to an IPC transport.
    async fn connect_ipc(&self) -> Result<InnerTransport, RuntimeTransportError> {
        let path = url_to_file_path(&self.url)
            .map_err(|_| RuntimeTransportError::BadPath(self.url.to_string()))?;
        let ipc_connector = IpcConnect::new(path.clone());
        let ipc = ipc_connector.into_service().await.map_err(|e| {
            RuntimeTransportError::TransportError(e, path.clone().display().to_string())
        })?;
        Ok(InnerTransport::Ipc(ipc))
    }

    /// Sends a request using the underlying transport.
    /// If this is the first request, it will connect to the appropriate transport depending on the
    /// URL scheme. Retries are performed by an external client layer (e.g., `RetryBackoffLayer`),
    /// if such a layer is configured by the caller.
    /// For sending the actual request, this action is delegated down to the
    /// underlying transport through Tower's [tower::Service::call]. See tower's [tower::Service]
    /// trait for more information.
    pub fn request(&self, req: RequestPacket) -> TransportFut<'static> {
        let this = self.clone();
        Box::pin(async move {
            let mut inner = this.inner.read().await;
            if inner.is_none() {
                drop(inner);
                {
                    let mut inner_mut = this.inner.write().await;
                    if inner_mut.is_none() {
                        *inner_mut =
                            Some(this.connect().await.map_err(TransportErrorKind::custom)?);
                    }
                }
                inner = this.inner.read().await;
            }

            // SAFETY: We just checked that the inner transport exists.
            match inner.clone().expect("must've been initialized") {
                InnerTransport::Http(mut http) => http
                    .call(req)
                    .await
                    .map_err(|error| redact_http_transport_error(error, &this.url)),
                InnerTransport::Ws(mut ws) => ws.call(req).await,
                InnerTransport::Ipc(mut ipc) => ipc.call(req).await,
            }
        })
    }

    /// Convert this transport into a boxed trait object.
    pub fn boxed(self) -> BoxTransport
    where
        Self: Sized + Clone + Send + Sync + 'static,
    {
        BoxTransport::new(self)
    }
}

/// Returns `true` if `url` points to a known MPP-enabled RPC service.
fn is_known_mpp_endpoint(url: &Url) -> bool {
    url.host_str().is_some_and(|host| KNOWN_MPP_HOSTS.iter().any(|suffix| host.ends_with(suffix)))
}

fn redact_http_transport_error(error: TransportError, endpoint: &Url) -> TransportError {
    let alloy_json_rpc::RpcError::Transport(TransportErrorKind::Custom(source)) = &error else {
        return error;
    };
    let safe_endpoint = redact_url(endpoint.as_str());

    let mut message = String::new();
    let mut error: Option<&(dyn StdError + 'static)> = Some(source.as_ref());
    while let Some(source) = error {
        if !message.is_empty() {
            message.push_str(": ");
        }
        message.push_str(&source.to_string());
        error = source.source();
    }
    let message = HTTP_URL_RE.replace_all(&message, |captures: &Captures<'_>| {
        let candidate = &captures[0];
        let Ok(url) = Url::parse(candidate) else { return candidate.to_owned() };
        if url.host() == endpoint.host()
            && url.port_or_known_default() == endpoint.port_or_known_default()
        {
            safe_endpoint.clone()
        } else {
            candidate.to_owned()
        }
    });
    TransportErrorKind::custom_str(&message)
}

impl tower::Service<RequestPacket> for RuntimeTransport {
    type Response = ResponsePacket;
    type Error = TransportError;
    type Future = TransportFut<'static>;

    #[inline]
    fn poll_ready(
        &mut self,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        std::task::Poll::Ready(Ok(()))
    }

    #[inline]
    fn call(&mut self, req: RequestPacket) -> Self::Future {
        self.request(req)
    }
}

impl tower::Service<RequestPacket> for &RuntimeTransport {
    type Response = ResponsePacket;
    type Error = TransportError;
    type Future = TransportFut<'static>;

    #[inline]
    fn poll_ready(
        &mut self,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        std::task::Poll::Ready(Ok(()))
    }

    #[inline]
    fn call(&mut self, req: RequestPacket) -> Self::Future {
        self.request(req)
    }
}

fn build_auth(jwt: String) -> eyre::Result<Authorization> {
    // Decode jwt from hex, then generate claims (iat with current timestamp)
    let secret = JwtSecret::from_hex(jwt)?;
    let claims = Claims::default();
    let token = secret.encode(&claims)?;

    let auth = Authorization::Bearer(token);

    Ok(auth)
}

#[cfg(windows)]
fn url_to_file_path(url: &Url) -> Result<PathBuf, ()> {
    const PREFIX: &str = "file:///pipe/";

    let url_str = url.as_str();

    if let Some(pipe_name) = url_str.strip_prefix(PREFIX) {
        let pipe_path = format!(r"\\.\pipe\{pipe_name}");
        return Ok(PathBuf::from(pipe_path));
    }

    url.to_file_path()
}

#[cfg(not(windows))]
fn url_to_file_path(url: &Url) -> Result<PathBuf, ()> {
    url.to_file_path()
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::HeaderMap;
    use std::io;

    #[derive(Debug, Error)]
    #[error("request to https://example.com/private-api-key failed")]
    struct ProviderError {
        #[source]
        source: io::Error,
    }

    #[test]
    fn http_transport_errors_preserve_provider_guidance() {
        let endpoint =
            Url::parse("https://user:password@example.com/private-api-key?token=secret").unwrap();
        let error = TransportErrorKind::custom(ProviderError {
            source: io::Error::other(
                "Authorize an access key with:\n  cast tempo login --no-browser",
            ),
        });

        let report = redact_http_transport_error(error, &endpoint).to_string();

        assert!(report.contains("https://example.com/"));
        assert!(report.contains("cast tempo login --no-browser"));
        assert!(!report.contains("password"));
        assert!(!report.contains("private-api-key"));
        assert!(!report.contains("secret"));
    }

    #[test]
    fn http_transport_errors_redact_endpoint_paths() {
        let endpoint =
            Url::parse("https://user:password@example.com/private-api-key?token=secret").unwrap();
        let error = TransportErrorKind::custom_str(concat!(
            "request to https://example.com/private-api-key failed: connection refused\n\n",
            "Authorize an access key with:\n  cast tempo login"
        ));

        let error = redact_http_transport_error(error, &endpoint);
        let report = error.to_string();

        assert!(report.contains("https://example.com/"));
        assert!(!report.contains("password"));
        assert!(!report.contains("private-api-key"));
        assert!(!report.contains("secret"));
        assert!(report.to_lowercase().contains("connection refused"));
        assert!(report.contains("cast tempo login"));
    }

    #[test]
    fn http_transport_errors_redact_normalized_endpoint_variants() {
        let endpoint =
            Url::parse("https://user:password@example.com/private-api-key?token=secret").unwrap();
        let error = TransportErrorKind::custom_str(
            "request to https://USER:normalized@example.com:443/different%2Fpath?key=other failed",
        );

        let report = redact_http_transport_error(error, &endpoint).to_string();

        assert!(report.contains("https://example.com/"));
        assert!(!report.contains("normalized"));
        assert!(!report.contains("different"));
        assert!(!report.contains("other"));
    }

    #[tokio::test]
    async fn websocket_error_redacts_url_credentials() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let url = Url::parse(&format!(
            "ws://user:password@{address}/private-api-key?token=secret#fragment"
        ))
        .unwrap();
        let transport = RuntimeTransportBuilder::new(url).build();

        let error = transport.connect_ws().await.unwrap_err().to_string();

        assert!(error.contains(&format!("ws://{address}/")));
        assert!(!error.contains("user"));
        assert!(!error.contains("password"));
        assert!(!error.contains("private-api-key"));
        assert!(!error.contains("secret"));
    }

    #[tokio::test]
    async fn test_user_agent_header() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = Url::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();

        let http_handler = axum::routing::get(|actual_headers: HeaderMap| {
            let user_agent = HeaderName::from_str("User-Agent").unwrap();
            assert_eq!(actual_headers[user_agent], HeaderValue::from_str("test-agent").unwrap());

            async { "" }
        });

        let server_task = tokio::spawn(async move {
            axum::serve(listener, http_handler.into_make_service()).await.unwrap()
        });

        let transport = RuntimeTransportBuilder::new(url.clone())
            .with_headers(vec!["User-Agent: test-agent".to_string()])
            .build();
        let inner = transport.connect_http().unwrap();

        match inner {
            InnerTransport::Http(http) => {
                let _ = http.client().get(url).send().await.unwrap();

                // assert inside http_handler
            }
            _ => unreachable!(),
        }

        server_task.abort();
    }
}
