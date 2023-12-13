//! Runtime transport that connects on first request, which can take either of an HTTP,
//! WebSocket, or IPC transport and supports retries based on CUPS logic.
use crate::{ALCHEMY_FREE_TIER_CUPS, REQUEST_TIMEOUT};
use super::policy::{RateLimitRetryPolicy, RetryPolicy};
use alloy_json_rpc::{RequestPacket, ResponsePacket};
use alloy_pubsub::{PubSubConnect, PubSubFrontend};
use alloy_transport::{
    Authorization, BoxTransport, TransportError, TransportErrorKind, TransportFut,
};
use alloy_transport_http::Http;
use alloy_transport_ipc::IpcConnect;
use alloy_transport_ws::WsConnect;
use ethers_providers::{JwtAuth, JwtKey};
use reqwest::header::{HeaderName, HeaderValue};
use std::{
    path::PathBuf,
    str::FromStr,
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc,
    },
};
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
    // TODO: IPC
    /// IPC transport
    Ipc(PubSubFrontend),
}

/// Error type for the runtime transport.
#[derive(Error, Debug)]
pub enum RuntimeTransportError {
    /// Internal transport error
    #[error(transparent)]
    TransportError(TransportError),

    /// Failed to lock the transport
    #[error("Failed to lock the transport")]
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

    /// Invalid construction of Http provider
    #[error(transparent)]
    HttpConstructionError(#[from] reqwest::Error),

    /// Invalid JWT
    #[error("Invalid JWT: {0}")]
    InvalidJwt(String),
}

/// A runtime transport is a custom [alloy_transport::Transport] that only connects when the *first*
/// request is made. When the first request is made, it will connect to the runtime using either an
/// HTTP WebSocket, or IPC transport depending on the URL used.
/// It also supports retries for rate-limiting and timeout-related errors.
#[derive(Clone, Debug, Error)]
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
    /// The current amount of requests being retried.
    requests_enqueued: Arc<AtomicU32>,
    /// The policy that will be used to determine whether to retry a request or not.
    policy: RateLimitRetryPolicy,
    /// Max amount of retries.
    max_rate_limit_retries: u32,
    /// Max amount of timeout retries
    max_timeout_retries: u32,
    /// Initial backoff
    initial_backoff: u64,
    /// Compute units per second
    compute_units_per_second: u64,
}

/// A builder for [RuntimeTransport].
pub struct RuntimeTransportBuilder {
    url: Url,
    headers: Vec<String>,
    jwt: Option<String>,
    timeout: std::time::Duration,
    max_rate_limit_retries: u32,
    max_timeout_retries: u32,
    initial_backoff: u64,
    compute_units_per_second: u64,
}

impl RuntimeTransportBuilder {
    /// Create a new builder with the given URL.
    pub fn new(url: Url) -> Self {
        Self {
            url,
            headers: vec![],
            jwt: None,
            timeout: REQUEST_TIMEOUT,
            max_rate_limit_retries: 8,
            max_timeout_retries: 8,
            initial_backoff: 800,
            compute_units_per_second: ALCHEMY_FREE_TIER_CUPS,
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

    /// Set the max rate-limit retries for the transport.
    pub fn with_max_rate_limit_retries(mut self, max_retries: u32) -> Self {
        self.max_rate_limit_retries = max_retries;
        self
    }

    /// Set the max timeout retries for the transport.
    pub fn with_max_timeout_retries(mut self, max_retries: u32) -> Self {
        self.max_timeout_retries = max_retries;
        self
    }

    /// Set the initial backoff for the transport.
    pub fn with_initial_backoff(mut self, initial_backoff: u64) -> Self {
        self.initial_backoff = initial_backoff;
        self
    }

    /// Set the compute units per second for the transport.
    pub fn with_cups(mut self, cups: u64) -> Self {
        self.compute_units_per_second = cups;
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
            requests_enqueued: Arc::new(AtomicU32::new(0)),
            policy: RateLimitRetryPolicy::default(),
            max_rate_limit_retries: self.max_rate_limit_retries,
            max_timeout_retries: self.max_timeout_retries,
            initial_backoff: self.initial_backoff,
            compute_units_per_second: self.compute_units_per_second,
        }
    }
}

impl ::core::fmt::Display for RuntimeTransport {
    fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
        write!(f, "RuntimeTransport {}", self.url)
    }
}

impl RuntimeTransport {
    /// Create a new [RuntimeTransport].
    pub fn new(
        url: Url,
        headers: Vec<String>,
        jwt: Option<String>,
        timeout: std::time::Duration,
        max_rate_limit_retries: u32,
        max_timeout_retries: u32,
        compute_units_per_second: u64,
        initial_backoff: u64,
    ) -> Self {
        Self {
            inner: Arc::new(RwLock::new(None)),
            url,
            headers,
            jwt,
            timeout,
            max_rate_limit_retries,
            max_timeout_retries,
            requests_enqueued: Arc::new(AtomicU32::new(0)),
            policy: RateLimitRetryPolicy::default(),
            initial_backoff,
            compute_units_per_second,
        }
    }

    /// Connects the underlying transport, depending on the URL scheme.
    pub async fn connect(&self) -> Result<InnerTransport, RuntimeTransportError> {
        match self.url.scheme() {
            "http" | "https" => self.connect_http().await,
            "ws" | "wss" => self.connect_ws().await,
            "file" => self.connect_ipc().await,
            _ => Err(RuntimeTransportError::BadScheme(self.url.scheme().to_string())),
        }
    }

    /// Connects to an HTTP [alloy_transport_http::Http] transport.
    async fn connect_http(&self) -> Result<InnerTransport, RuntimeTransportError> {
        let mut client_builder = reqwest::Client::builder().timeout(self.timeout);
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
        for header in self.headers.iter() {
            let make_err = || RuntimeTransportError::BadHeader(header.to_string());

            let (key, val) = header.split_once(':').ok_or_else(make_err)?;

            headers.insert(
                HeaderName::from_str(key.trim()).map_err(|_| make_err())?,
                HeaderValue::from_str(val.trim()).map_err(|_| make_err())?,
            );
        }

        client_builder = client_builder.default_headers(headers);

        let client =
            client_builder.build().map_err(RuntimeTransportError::HttpConstructionError)?;

        // todo: retry tower layer
        Ok(InnerTransport::Http(Http::with_client(client, self.url.clone())))
    }

    /// Connects to a WS transport.
    async fn connect_ws(&self) -> Result<InnerTransport, RuntimeTransportError> {
        let auth = self.jwt.as_ref().and_then(|jwt| build_auth(jwt.clone()).ok());
        let ws = WsConnect { url: self.url.to_string(), auth }
            .into_service()
            .await
            .map_err(RuntimeTransportError::TransportError)?;
        Ok(InnerTransport::Ws(ws))
    }

    /// Connects to an IPC transport.
    async fn connect_ipc(&self) -> Result<InnerTransport, RuntimeTransportError> {
        let path = url_to_file_path(&self.url)
            .map_err(|_| RuntimeTransportError::BadPath(self.url.to_string()))?;
        let ipc_connector: IpcConnect<PathBuf> = path.into();
        let ipc =
            ipc_connector.into_service().await.map_err(RuntimeTransportError::TransportError)?;
        Ok(InnerTransport::Ipc(ipc))
    }

    /// Sends a request using the underlying transport.
    /// If this is the first request, it will connect to the appropiate transport depending on the
    /// URL scheme. When sending the request, retries will be automatically handled depending
    /// on the parameters set on the [RuntimeTransport].
    /// For sending the actual request, this action is delegated down to the
    /// underlying transport through Tower's call. See tower's [tower::Service] trait for more
    /// information.
    pub fn request(&self, req: RequestPacket) -> TransportFut<'static> {
        let this = self.clone();
        Box::pin(async move {
            if this.inner.read().await.is_none() {
                let mut inner = this.inner.write().await;
                *inner = Some(this.connect().await.map_err(TransportErrorKind::custom)?)
            }

            let mut inner = this.inner.write().await;
            // SAFETY: We just checked that the inner transport exists.
            let inner_mut = inner.as_mut().expect("We should have an inner transport.");

            let ahead_in_queue = this.requests_enqueued.fetch_add(1, Ordering::SeqCst) as u64;

            let mut rate_limit_retry_number: u32 = 0;
            let mut timeout_retries: u32 = 0;

            loop {
                let err;
                let fut = match inner_mut {
                    InnerTransport::Http(http) => http.call(req.clone()),
                    InnerTransport::Ws(ws) => ws.call(req.clone()),
                    InnerTransport::Ipc(ipc) => ipc.call(req.clone()),
                }
                .await;

                match fut {
                    Ok(res) => {
                        this.requests_enqueued.fetch_sub(1, Ordering::SeqCst);
                        return Ok(res)
                    }
                    Err(e) => err = e,
                }

                let err = TransportError::from(err);
                let should_retry = this.policy.should_retry(&err);
                if should_retry {
                    rate_limit_retry_number += 1;
                    if rate_limit_retry_number > this.max_rate_limit_retries {
                        return Err(TransportErrorKind::custom_str("Max retries exceeded"))
                    }

                    let current_queued_reqs = this.requests_enqueued.load(Ordering::SeqCst) as u64;

                    // try to extract the requested backoff from the error or compute the next
                    // backoff based on retry count
                    let mut next_backoff = this.policy.backoff_hint(&err).unwrap_or_else(|| {
                        std::time::Duration::from_millis(this.initial_backoff as u64)
                    });

                    // requests are usually weighted and can vary from 10 CU to several 100 CU,
                    // cheaper requests are more common some example alchemy
                    // weights:
                    // - `eth_getStorageAt`: 17
                    // - `eth_getBlockByNumber`: 16
                    // - `eth_newFilter`: 20
                    //
                    // (coming from forking mode) assuming here that storage request will be the
                    // driver for Rate limits we choose `17` as the average cost
                    // of any request
                    const AVG_COST: u64 = 17u64;
                    let seconds_to_wait_for_compute_budget = compute_unit_offset_in_secs(
                        AVG_COST,
                        this.compute_units_per_second,
                        current_queued_reqs,
                        ahead_in_queue,
                    );
                    next_backoff +=
                        std::time::Duration::from_secs(seconds_to_wait_for_compute_budget);

                    tokio::time::sleep(next_backoff).await;
                } else {
                    if timeout_retries < this.max_timeout_retries {
                        timeout_retries += 1;
                        continue;
                    }

                    this.requests_enqueued.fetch_sub(1, Ordering::SeqCst);
                    return Err(TransportErrorKind::custom_str("Max retries exceeded"))
                }
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

impl Service<RequestPacket> for &RuntimeTransport {
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
    let jwt = hex::decode(jwt)?;
    let secret = JwtKey::from_slice(&jwt).map_err(|err| eyre::eyre!("Invalid JWT: {}", err))?;
    let auth = JwtAuth::new(secret, None, None);
    let token = auth.generate_token()?;

    // Essentially unrolled ethers-rs new_with_auth to accomodate the custom timeout
    let auth = Authorization::Bearer(token);

    Ok(auth)
}

#[cfg(windows)]
fn url_to_file_path(url: &Url) -> Result<PathBuf, ()> {
    const PREFIX: &str = "file:///pipe/";

    let url_str = url.as_str();

    if url_str.starts_with(PREFIX) {
        let pipe_name = &url_str[PREFIX.len()..];
        let pipe_path = format!(r"\\.\pipe\{}", pipe_name);
        return Ok(PathBuf::from(pipe_path));
    }

    url.to_file_path()
}

#[cfg(not(windows))]
fn url_to_file_path(url: &Url) -> Result<PathBuf, ()> {
    url.to_file_path()
}

/// Calculates an offset in seconds by taking into account the number of currently queued requests,
/// number of requests that were ahead in the queue when the request was first issued, the average
/// cost a weighted request (heuristic), and the number of available compute units per seconds.
///
/// Returns the number of seconds (the unit the remote endpoint measures compute budget) a request
/// is supposed to wait to not get rate limited. The budget per second is
/// `compute_units_per_second`, assuming an average cost of `avg_cost` this allows (in theory)
/// `compute_units_per_second / avg_cost` requests per seconds without getting rate limited.
/// By taking into account the number of concurrent request and the position in queue when the
/// request was first issued and determine the number of seconds a request is supposed to wait, if
/// at all
fn compute_unit_offset_in_secs(
    avg_cost: u64,
    compute_units_per_second: u64,
    current_queued_requests: u64,
    ahead_in_queue: u64,
) -> u64 {
    let request_capacity_per_second = compute_units_per_second.saturating_div(avg_cost);
    if current_queued_requests > request_capacity_per_second {
        current_queued_requests.min(ahead_in_queue).saturating_div(request_capacity_per_second)
    } else {
        0
    }
}