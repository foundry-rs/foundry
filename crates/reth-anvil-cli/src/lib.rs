//! The anvil binary and runtime selection of its installed network extensions.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]

use eyre::Result;
use foundry_evm_networks::NetworkVariant;

pub mod args;
pub mod cmd;
pub mod opts;

pub use reth_anvil::*;

#[cfg(feature = "tempo")]
pub use reth_anvil_tempo::{TempoConfig, TempoConfigExt};

/// Starts an anvil node with its configured network extension.
pub async fn spawn(config: NodeConfig) -> (EthApi, NodeHandle) {
    try_spawn(config).await.expect("failed to spawn node")
}

/// Resolves the network once, then launches its concrete SDK implementation.
pub async fn try_spawn(mut config: NodeConfig) -> Result<(EthApi, NodeHandle)> {
    let supported = [
        NetworkVariant::Ethereum,
        #[cfg(feature = "tempo")]
        NetworkVariant::Tempo,
    ];
    config.resolve_networks_for(&supported).await?;
    match config.networks.resolved_network().unwrap_or_default() {
        NetworkVariant::Ethereum => launch::<network::ethereum::Ethereum>(config).await,
        #[cfg(feature = "tempo")]
        NetworkVariant::Tempo => launch::<reth_anvil_tempo::Tempo>(config).await,
        #[allow(unreachable_patterns)]
        network => eyre::bail!("the {network:?} network is not supported yet"),
    }
}
