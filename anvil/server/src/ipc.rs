//! IPC handling

use crate::PubSubRpcHandler;
use anvil_rpc::{request::Request, response::Response};
use futures::stream::StreamExt;
use parity_tokio_ipc::Endpoint;
use tokio_serde::{formats::Json, Framed};
use tracing::{error, trace};
use crate::pubsub::PubSubConnection;

/// An IPC connection for anvil
///
/// A Future that listens for incoming connections and spawns new connections
pub struct IpcEndpoint<Handler> {
    /// the handler for the websocket connection
    handler: Handler,
    /// The endpoint we listen for incoming transactions
    endpoint: Endpoint,
    // TODO add shutdown
}

impl<Handler: PubSubRpcHandler> IpcEndpoint<Handler> {
    /// Creates a new endpoint with the given handler
    pub fn new(handler: Handler, endpoint: Endpoint) -> Self {
        Self { handler, endpoint }
    }

    /// Start listening for incoming connections
    pub async fn start(self) {
        let IpcEndpoint { handler, endpoint } = self;
        trace!(target: "ipc",  endpoint=?endpoint.path(), "starting ipc server" );

        let mut connections = match endpoint.incoming() {
            Ok(connections) => connections,
            Err(err) => {
                error!(target: "ipc",  ?err, "Failed to create ipc listener");
                return
            }
        };

        while let Some(Ok(stream)) = connections.next().await {
            trace!(target: "ipc", "successful incoming IPC connection");

            let framed: Framed<_, Request, Response, _> =
                Framed::new(stream, Json::<Request, Response>::default());

            // TOOD need to convert the stream into a Sink+Stream

            // PubSubConnection::new(framed, handler.clone()).await

        }
    }
}
