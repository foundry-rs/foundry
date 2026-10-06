//! The `reth-anvil` CLI: a local Ethereum development node built on the reth SDK.

use reth_anvil::RethAnvilConfig;
use reth_ethereum::tasks::{RuntimeBuilder, RuntimeConfig};

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let runtime = RuntimeBuilder::new(RuntimeConfig::default()).build()?;
    let node = reth_anvil::launch(RethAnvilConfig::default(), runtime).await?;
    node.node_exit_future.await
}
