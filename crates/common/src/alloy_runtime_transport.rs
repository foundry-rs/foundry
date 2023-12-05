//! Runtime transport that connects on first request, which can take either of an HTTP,
//! WebSocket, or IPC transport.
use std::{sync::Arc, time::Duration};

use alloy_json_rpc::{RequestPacket, ResponsePacket};
use alloy_providers::provider::Provider;
use alloy_pubsub::{PubSubConnect, PubSubFrontend};
use alloy_rpc_client::{ClientBuilder, RpcClient};
use alloy_transport::{TransportError, TransportFut};
use alloy_transport_http::Http;
use alloy_transport_ws::WsConnect;
use thiserror::Error;
use tokio::sync::RwLock;
use tower::Service;
use url::Url;

/// An enum representing the different transports that can be used to connect to a runtime.
#[derive(Debug)]
pub enum InnerTransport {
    /// HTTP transport
    Http(Http<reqwest::Client>),
    Ws(PubSubFrontend),
    // TODO: IPC
    Ipc,
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
}

#[derive(Clone, Debug, Error)]
pub struct RuntimeTransport {
    inner: Arc<RwLock<Option<InnerTransport>>>,
    url: Url,
    max_retry: u32,
    timeout_retry: u32,
    timeout: Duration,
    compute_units_per_second: u64,
    jwt: Option<String>,
    headers: Vec<String>,
}

impl ::core::fmt::Display for RuntimeTransport {
    fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
        write!(f, "RuntimeTransport {}", self.url)
    }
}

impl RuntimeTransport {
    /// Connect to the runtime transport, depending on the URL scheme.
    async fn connect(&self) -> Result<InnerTransport, RuntimeTransportError> {
        match self.url.scheme() {
            "http" | "https" => Ok(InnerTransport::Http(Http::new(self.url.clone()))),
            "ws" | "wss" => {
                // TODO: Auth
                let ws = WsConnect { url: self.url.to_string(), auth: None }
                    .into_service()
                    .await
                    .unwrap();
                Ok(InnerTransport::Ws(ws))
            }
            // TODO: IPC once it's merged
            _ => Err(RuntimeTransportError::BadScheme(self.url.scheme().to_string())),
        }
    }

    /// Send a request
    async fn request(&self, req: RequestPacket) -> TransportFut<'static> {
        if self.inner.read().await.is_none() {
            let mut w = self.inner.write().await;
            *w = Some(
                self.connect()
                    .await
                    .map_err(|e| TransportError::Other(e.to_string()))
                    .unwrap(),
            )
        }
        match self.url.scheme() {
            _ => todo!()
        }
    }
}

impl Service<RequestPacket> for RuntimeTransport {
    type Response = ResponsePacket;
    type Error = TransportError;
    type Future = TransportFut<'static>;

    #[inline]
    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        todo!()
    }

    #[inline]
    fn call(&mut self, req: RequestPacket) -> Self::Future {
        todo!()
    }
}

impl Service<RequestPacket> for &RuntimeTransport {
    type Response = ResponsePacket;
    type Error = TransportError;
    type Future = TransportFut<'static>;

    #[inline]
    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        todo!()
    }

    #[inline]
    fn call(&mut self, req: RequestPacket) -> Self::Future {
        todo!()
    }
}
