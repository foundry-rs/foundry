pub mod api;
pub mod otterscan;
pub mod sign;
pub use api::EthApi;

pub mod backend;

pub mod error;

pub mod fees;
pub(crate) mod macros;
pub mod miner;
pub mod pool;
pub mod util;

// Create a new node and start the server
// Returns an instance of EthApi and NodeHandle
pub async fn spawn(mut config: Config) -> Result<(EthApi, NodeHandle)> {
    // Create a new node with the provided configuration
    let node = Node::new(config.clone()).await?;
    let node_handle = node.handle();

    // Create a new server with the node
    let server = Server::new(node);
    let addr = server.addr();

    // Start the server in a separate task
    tokio::spawn(server);

    // Create a new EthApi instance with the node handle and server address
    let api = EthApi::new(node_handle.clone(), addr);

    Ok((api, node_handle))
}

// Handle incoming RPC requests
// This function is responsible for processing RPC calls and returning appropriate responses
pub async fn handle(
    mut handler: PubSubEthRpcHandler,
    mut rx: mpsc::Receiver<PubSubRpcHandler>,
) -> Result<()> {
    // Process incoming messages until the channel is closed
    while let Some(msg) = rx.recv().await {
        // Handle the received message
        handler.handle_message(msg).await?;
    }

    Ok(())
}
