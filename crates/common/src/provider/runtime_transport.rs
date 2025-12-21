//! Runtime transport that connects on first request, which can take either of an HTTP,
//! WebSocket, or IPC transport. Retries are handled by a client layer (e.g.,
//! `RetryBackoffLayer`) when used.

use crate::{DEFAULT_USER_AGENT, REQUEST_TIMEOUT};
use alloy_json_rpc::{RequestPacket, ResponsePacket};
use alloy_pubsub::{PubSubConnect, PubSubFrontend};
use alloy_rpc_types::engine::{Claims, JwtSecret};
use alloy_transport::{
    Authorization, BoxTransport, TransportError, TransportErrorKind, TransportFut,
};
use alloy_transport_http::Http;
use alloy_transport_ipc::IpcConnect;
use alloy_transport_ws::WsConnect;
use reqwest::header::{HeaderName, HeaderValue};
use std::{fmt, path::PathBuf, str::FromStr, sync::Arc};
use thiserror::Error;
use tokio::sync::RwLock;
use tower::Service;
use url::Url;

/// An enum representing the different transports that can be used to connect to a runtime.
/// Only meant to be used internally by [RuntimeTransport].
#[derive(Clone, Debug)]
pub enum InnerTransport {
    /// HTTP transport
    Http(Http<reqwest::Client>),
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
}

/// A builder for [RuntimeTransport].
#[derive(Debug)]
pub struct RuntimeTransportBuilder {
    url: Url,
    headers: Vec<String>,
    jwt: Option<String>,
    timeout: std::time::Duration,
    accept_invalid_certs: bool,
}

impl RuntimeTransportBuilder {
    /// Create a new builder with the given URL.
    pub fn new(url: Url) -> Self {
        Self {
            url,
            headers: vec![],
            jwt: None,
            timeout: REQUEST_TIMEOUT,
            accept_invalid_certs: false,
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
    pub fn with_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Set whether to accept invalid certificates.
    pub fn accept_invalid_certs(mut self, accept_invalid_certs: bool) -> Self {
        self.accept_invalid_certs = accept_invalid_certs;
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
        }
    }
}

impl fmt::Display for RuntimeTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RuntimeTransport {}", self.url)
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

    /// Creates a new reqwest client from this transport.
    pub fn reqwest_client(&self) -> Result<reqwest::Client, RuntimeTransportError> {
        let mut client_builder = reqwest::Client::builder()
            .timeout(self.timeout)
            .tls_built_in_root_certs(self.url.scheme() == "https")
            .danger_accept_invalid_certs(self.accept_invalid_certs);
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
            let make_err = || RuntimeTransportError::BadHeader(header.to_string());

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

        client_builder = client_builder.default_headers(headers);

        Ok(client_builder.build()?)
    }

    /// Connects to an HTTP [alloy_transport_http::Http] transport.
    fn connect_http(&self) -> Result<InnerTransport, RuntimeTransportError> {
        let client = self.reqwest_client()?;
        Ok(InnerTransport::Http(Http::with_client(client, self.url.clone())))
    }

    /// Connects to a WS transport.
    async fn connect_ws(&self) -> Result<InnerTransport, RuntimeTransportError> {
        let auth = self.jwt.as_ref().and_then(|jwt| build_auth(jwt.clone()).ok());
        let mut ws = WsConnect::new(self.url.to_string());
        if let Some(auth) = auth {
            ws = ws.with_auth(auth);
        };
        let service = ws
            .into_service()
            .await
            .map_err(|e| RuntimeTransportError::TransportError(e, self.url.to_string()))?;
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
                InnerTransport::Http(mut http) => http.call(req),
                InnerTransport::Ws(mut ws) => ws.call(req),
                InnerTransport::Ipc(mut ipc) => ipc.call(req),
            }
            .await
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
