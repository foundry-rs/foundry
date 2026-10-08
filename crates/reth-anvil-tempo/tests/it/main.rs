//! Integration coverage for the external Tempo network implementation.
use reth_anvil::{EthApi, NodeConfig, NodeHandle, launch, network::ethereum::Ethereum};
use reth_anvil_tempo::Tempo;

mod tempo;
mod tempo_canary;

mod utils {
    pub use foundry_common::provider::get_http_provider as http_provider;
}

async fn spawn(mut config: NodeConfig) -> (EthApi, NodeHandle) {
    config
        .resolve_networks_for(&[
            foundry_evm_networks::NetworkVariant::Ethereum,
            foundry_evm_networks::NetworkVariant::Tempo,
        ])
        .await
        .unwrap();
    if config.networks.is_tempo() {
        launch::<Tempo>(config).await.unwrap()
    } else {
        launch::<Ethereum>(config).await.unwrap()
    }
}
