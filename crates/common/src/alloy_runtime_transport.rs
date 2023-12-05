//! Runtime transport that connects on first request, which can take either of an HTTP,
//! WebSocket, or IPC transport.
use std::sync::Arc;
use alloy_json_rpc::{RequestPacket, ResponsePacket};
use alloy_pubsub::{PubSubConnect, PubSubFrontend};
use alloy_transport::{TransportError, TransportFut};
use alloy_transport_http::Http;
use alloy_transport_ws::WsConnect;
use thiserror::Error;
use tokio::sync::RwLock;
use tower::Service;
use url::Url;

/// An enum representing the different transports that can be used to connect to a runtime.
#[derive(Clone, Debug)]
pub enum InnerTransport {
    /// HTTP transport
    Http(Http<reqwest::Client>),
    /// WebSocket transport
    Ws(PubSubFrontend),
    // TODO: IPC
    /// IPC transport
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

/// A runtime transport that connects on first request, which can take either of an HTTP,
/// WebSocket, or IPC transport depending on the URL used.
#[derive(Clone, Debug, Error)]
pub struct RuntimeTransport {
    /// The inner actual transport used.
    inner: Arc<RwLock<Option<InnerTransport>>>,
    /// The URL to connect to.
    url: Url,
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
    pub fn request(&self, req: RequestPacket) -> TransportFut<'static> {
        let this = self.clone();
        Box::pin(async move {
            if this.inner.read().await.is_none() {
                let mut w = this.inner.write().await;
                *w = Some(this.connect().await.unwrap())
            }

            let mut inner = this.inner.write().await;
            let inner_mut = inner.as_mut().expect("boom");

            match inner_mut {
                InnerTransport::Http(_) => todo!(), // http.request(req).await,
                InnerTransport::Ws(ws) => ws.send_packet(req).await,
                InnerTransport::Ipc => todo!(),
            }
        })
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
