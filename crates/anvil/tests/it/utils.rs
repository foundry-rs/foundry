use alloy_network::{Ethereum, EthereumSigner};
use foundry_common::provider::{
    get_http_provider, ProviderBuilder, RetryProvider, RetryProviderWithSigner,
};

pub fn http_provider(http_endpoint: &str) -> RetryProvider {
    get_http_provider(http_endpoint)
}

pub fn http_provider_with_signer(
    http_endpoint: &str,
    signer: EthereumSigner,
) -> RetryProviderWithSigner {
    ProviderBuilder::new(http_endpoint)
        .build_with_signer(signer)
        .expect("failed to build Alloy HTTP provider with signer")
}

pub fn ws_provider_with_signer(
    ws_endpoint: &str,
    signer: EthereumSigner,
) -> RetryProviderWithSigner {
    ProviderBuilder::new(ws_endpoint)
        .build_with_signer(signer)
        .expect("failed to build Alloy WS provider with signer")
}

/// Currently required to get around <https://github.com/alloy-rs/alloy/issues/296>
pub async fn connect_pubsub(conn_str: &str) -> RootProvider<BoxTransport> {
    alloy_provider::ProviderBuilder::new().on_builtin(conn_str).await.unwrap()
}

use alloy_provider::{
    fillers::{ChainIdFiller, FillProvider, GasFiller, JoinFill, NonceFiller, SignerFiller},
    Identity, RootProvider,
};
use alloy_transport::BoxTransport;
type PubsubSigner = FillProvider<
    JoinFill<
        JoinFill<JoinFill<JoinFill<Identity, GasFiller>, NonceFiller>, ChainIdFiller>,
        SignerFiller<EthereumSigner>,
    >,
    RootProvider<BoxTransport>,
    BoxTransport,
    Ethereum,
>;
pub async fn connect_pubsub_with_signer(conn_str: &str, signer: EthereumSigner) -> PubsubSigner {
    alloy_provider::ProviderBuilder::new()
        .with_recommended_fillers()
        .signer(signer)
        .on_builtin(conn_str)
        .await
        .unwrap()
}

pub async fn ipc_provider_with_signer(
    ipc_endpoint: &str,
    signer: EthereumSigner,
) -> RetryProviderWithSigner {
    ProviderBuilder::new(ipc_endpoint)
        .build_with_signer(signer)
        .expect("failed to build Alloy IPC provider with signer")
}
