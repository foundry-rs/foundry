//! IPC handling

use crate::PubSubRpcHandler;
use parity_tokio_ipc::Endpoint;

/// An IPC connection for anvil
pub struct IpcEndpoint<Handler: PubSubRpcHandler> {
    /// the handler for the websocket connection
    handler: Handler,
    /// The endpoint we listen for incoming transactions
    endpoint: Endpoint,
}

async fn on_connection() {}
