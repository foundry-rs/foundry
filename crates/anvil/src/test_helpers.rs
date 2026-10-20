//! Helpers for tests that run a local Anvil node.

use crate::{NodeConfig, NodeHandle, eth::EthApi, spawn};
use foundry_common::provider::RetryProvider;
use foundry_primitives::FoundryNetwork;

/// A node spawned with [`NodeConfig::test`] and an HTTP provider connected to it.
pub struct TestNode {
    /// The node's in-process API.
    pub api: EthApi<FoundryNetwork>,
    /// The node handle, which must remain alive while the test uses the node.
    pub handle: NodeHandle,
    /// An HTTP provider connected to the node.
    pub provider: RetryProvider,
}

impl TestNode {
    /// Spawns a test node on a free port and constructs its HTTP provider.
    pub async fn spawn() -> Self {
        let (api, handle) = spawn(NodeConfig::test()).await;
        let provider = handle.http_provider();
        Self { api, handle, provider }
    }
}
