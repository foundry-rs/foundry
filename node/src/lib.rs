mod config;

use crate::eth::EthApi;
pub use config::NodeConfig;
use tokio::task::JoinHandle;

// mod node;
// pub use node::Node;

mod service;

/// axum RPC server implementations
pub mod server;

pub mod eth;

/// Creates the node and runs the server
///
/// Returns the [EthApi] that can be used to interact with the node and the [JoinHandle] of the
/// task.
// TODO add example
pub fn spawn(_config: NodeConfig) -> (EthApi, JoinHandle<()>) {
    todo!()
}
