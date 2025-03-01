//! This module extends the Ethereum JSON-RPC provider with the Net namespace's RPC methods.
use crate::Provider;
use alloy_network::Network;
use alloy_transport::TransportResult;

/// Net namespace rpc interface that provides access to network information of the node.
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
pub trait NetApi<N>: Send + Sync {
    /// Returns a `bool` indicating whether or not the node is listening for network connections.
    async fn net_listening(&self) -> TransportResult<bool>;
    /// Returns the number of peers connected to the node.
    async fn net_peer_count(&self) -> TransportResult<u64>;
    /// Returns the network ID (e.g. 1 for mainnet).
    async fn net_version(&self) -> TransportResult<u64>;
}

#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
impl<N, P> NetApi<N> for P
where
    N: Network,
    P: Provider<N>,
{
    async fn net_listening(&self) -> TransportResult<bool> {
        self.client().request_noparams("net_listening").await
    }

    async fn net_peer_count(&self) -> TransportResult<u64> {
        self.client().request_noparams("net_peerCount").map_resp(crate::utils::convert_u64).await
    }

    async fn net_version(&self) -> TransportResult<u64> {
        self.client().request_noparams("net_version").map_resp(crate::utils::convert_u64).await
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::{ext::test::async_ci_only, ProviderBuilder};
    use alloy_node_bindings::{utils::run_with_tempdir, Geth};

    #[tokio::test]
    async fn call_net_version() {
        async_ci_only(|| async move {
            run_with_tempdir("geth-test-", |temp_dir| async move {
                let geth = Geth::new().disable_discovery().data_dir(temp_dir).spawn();
                let provider = ProviderBuilder::new().on_http(geth.endpoint_url());

                let version =
                    provider.net_version().await.expect("net_version call should succeed");
                assert_eq!(version, 1);
            })
            .await;
        })
        .await;
    }

    #[tokio::test]
    async fn call_net_peer_count() {
        async_ci_only(|| async move {
            run_with_tempdir("geth-test-", |temp_dir| async move {
                let geth = Geth::new().disable_discovery().data_dir(temp_dir).spawn();
                let provider = ProviderBuilder::new().on_http(geth.endpoint_url());

                let count =
                    provider.net_peer_count().await.expect("net_peerCount call should succeed");
                assert_eq!(count, 0);
            })
            .await;
        })
        .await;
    }

    #[tokio::test]
    async fn call_net_listening() {
        async_ci_only(|| async move {
            run_with_tempdir("geth-test-", |temp_dir| async move {
                let geth = Geth::new().disable_discovery().data_dir(temp_dir).spawn();
                let provider = ProviderBuilder::new().on_http(geth.endpoint_url());

                let listening =
                    provider.net_listening().await.expect("net_listening call should succeed");
                assert!(listening);
            })
            .await;
        })
        .await;
    }
}
