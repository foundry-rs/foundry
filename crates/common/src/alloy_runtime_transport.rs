//! Runtime transport that connects on first request, which can take either of an HTTP,
//! WebSocket, or IPC transport.
use std::{sync::Arc, time::Duration};

use alloy_providers::provider::Provider;
use alloy_pubsub::PubSubFrontend;
use alloy_rpc_client::{ClientBuilder, RpcClient};
use alloy_transport::TransportError;
use alloy_transport_http::Http;
use alloy_transport_ws::WsConnect;
use thiserror::Error;
use tokio::sync::RwLock;
use url::Url;

/// An enum representing the different transports that can be used to connect to a runtime.
#[derive(Debug)]
pub enum InnerTransport {
    /// HTTP transport
    Http(RpcClient<Http<reqwest::Client>>),
    Ws(RpcClient<PubSubFrontend>),
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
    async fn connect(&self) -> Result<InnerTransport, RuntimeTransportError> {
        match self.url.scheme() {
            "http" | "https" => {
                Ok(InnerTransport::Http(ClientBuilder::default().reqwest_http(self.url.to_owned())))
            }
            "ws" | "wss" => Ok(InnerTransport::Ws(
                ClientBuilder::default()
                    .ws(WsConnect { url: self.url.to_string(), auth: None })
                    .await
                    .unwrap(),
            )),
            // TODO: IPC once it's merged
            _ => Err(RuntimeTransportError::BadScheme(self.url.scheme().to_string())),
        }
    }
}
