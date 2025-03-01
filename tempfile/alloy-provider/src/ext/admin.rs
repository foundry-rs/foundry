//! This module extends the Ethereum JSON-RPC provider with the Admin namespace's RPC methods.
use crate::Provider;
use alloy_network::Network;
use alloy_rpc_types_admin::{NodeInfo, PeerInfo};
use alloy_transport::TransportResult;

/// Admin namespace rpc interface that gives access to several non-standard RPC methods.
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
pub trait AdminApi<N>: Send + Sync {
    /// Requests adding the given peer, returning a boolean representing
    /// whether or not the peer was accepted for tracking.
    async fn add_peer(&self, record: &str) -> TransportResult<bool>;

    /// Requests adding the given peer as a trusted peer, which the node will
    /// always connect to even when its peer slots are full.
    async fn add_trusted_peer(&self, record: &str) -> TransportResult<bool>;

    /// Requests to remove the given peer, returning true if the enode was successfully parsed and
    /// the peer was removed.
    async fn remove_peer(&self, record: &str) -> TransportResult<bool>;

    /// Requests to remove the given peer, returning a boolean representing whether or not the
    /// enode url passed was validated. A return value of `true` does not necessarily mean that the
    /// peer was disconnected.
    async fn remove_trusted_peer(&self, record: &str) -> TransportResult<bool>;

    /// Returns the list of peers currently connected to the node.
    async fn peers(&self) -> TransportResult<Vec<PeerInfo>>;

    /// Returns general information about the node as well as information about the running p2p
    /// protocols (e.g. `eth`, `snap`).
    async fn node_info(&self) -> TransportResult<NodeInfo>;

    /// Subscribe to events received by peers over the network.
    #[cfg(feature = "pubsub")]
    async fn subscribe_peer_events(
        &self,
    ) -> TransportResult<alloy_pubsub::Subscription<alloy_rpc_types_admin::PeerEvent>>;
}

#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
impl<N, P> AdminApi<N> for P
where
    N: Network,
    P: Provider<N>,
{
    async fn add_peer(&self, record: &str) -> TransportResult<bool> {
        self.client().request("admin_addPeer", (record,)).await
    }

    async fn add_trusted_peer(&self, record: &str) -> TransportResult<bool> {
        self.client().request("admin_addTrustedPeer", (record,)).await
    }

    async fn remove_peer(&self, record: &str) -> TransportResult<bool> {
        self.client().request("admin_removePeer", (record,)).await
    }

    async fn remove_trusted_peer(&self, record: &str) -> TransportResult<bool> {
        self.client().request("admin_removeTrustedPeer", (record,)).await
    }

    async fn peers(&self) -> TransportResult<Vec<PeerInfo>> {
        self.client().request_noparams("admin_peers").await
    }

    async fn node_info(&self) -> TransportResult<NodeInfo> {
        self.client().request_noparams("admin_nodeInfo").await
    }

    #[cfg(feature = "pubsub")]
    async fn subscribe_peer_events(
        &self,
    ) -> TransportResult<alloy_pubsub::Subscription<alloy_rpc_types_admin::PeerEvent>> {
        self.root().pubsub_frontend()?;
        let mut call = self.client().request_noparams("admin_peerEvents_subscribe");
        call.set_is_subscription();
        let id = call.await?;
        self.root().get_subscription(id).await
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::{ext::test::async_ci_only, ProviderBuilder};
    use alloy_node_bindings::{utils::run_with_tempdir, Geth};

    #[tokio::test]
    async fn node_info() {
        async_ci_only(|| async move {
            run_with_tempdir("geth-test-", |temp_dir| async move {
                let geth = Geth::new().disable_discovery().data_dir(temp_dir).spawn();
                let provider = ProviderBuilder::new().on_http(geth.endpoint_url());
                let node_info = provider.node_info().await.unwrap();
                assert!(node_info.enode.starts_with("enode://"));
            })
            .await;
        })
        .await;
    }

    #[tokio::test]
    async fn admin_peers() {
        async_ci_only(|| async move {
            run_with_tempdir("geth-test-1", |temp_dir_1| async move {
                run_with_tempdir("geth-test-2", |temp_dir_2| async move {
                    let geth1 = Geth::new().disable_discovery().data_dir(&temp_dir_1).spawn();
                    let mut geth2 =
                        Geth::new().disable_discovery().port(0u16).data_dir(&temp_dir_2).spawn();

                    let provider1 = ProviderBuilder::new().on_http(geth1.endpoint_url());
                    let provider2 = ProviderBuilder::new().on_http(geth2.endpoint_url());
                    let node1_info = provider1.node_info().await.unwrap();
                    let node1_id = node1_info.id;
                    let node1_enode = node1_info.enode;

                    let added = provider2.add_peer(&node1_enode).await.unwrap();
                    assert!(added);
                    geth2.wait_to_add_peer(&node1_id).unwrap();
                    let peers = provider2.peers().await.unwrap();
                    assert_eq!(peers[0].enode, node1_enode);
                })
                .await;
            })
            .await;
        })
        .await;
    }
}
