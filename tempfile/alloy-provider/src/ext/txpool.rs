//! This modules extends the Ethereum JSON-RPC provider with the `txpool` namespace.
use crate::Provider;
use alloy_network::{Ethereum, Network};
use alloy_primitives::Address;
use alloy_rpc_types_txpool::{TxpoolContent, TxpoolContentFrom, TxpoolInspect, TxpoolStatus};
use alloy_transport::TransportResult;

/// Txpool namespace rpc interface.
#[allow(unused, unreachable_pub)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
pub trait TxPoolApi<N: Network = Ethereum>: Send + Sync {
    /// Returns the content of the transaction pool.
    ///
    /// Lists the exact details of all the transactions currently pending for inclusion in the next
    /// block(s), as well as the ones that are being scheduled for future execution only.
    ///
    /// See [here](https://geth.ethereum.org/docs/rpc/ns-txpool#txpool_content) for more details
    async fn txpool_content(&self) -> TransportResult<TxpoolContent<N::TransactionResponse>>;

    /// Returns the content of the transaction pool filtered by a specific address.
    ///
    /// See [here](https://geth.ethereum.org/docs/rpc/ns-txpool#txpool_contentFrom) for more details
    async fn txpool_content_from(
        &self,
        from: Address,
    ) -> TransportResult<TxpoolContentFrom<N::TransactionResponse>>;

    /// Returns a textual summary of each transaction in the pool.
    ///
    /// Lists a textual summary of all the transactions currently pending for inclusion in the next
    /// block(s), as well as the ones that are being scheduled for future execution only.
    /// This is a method specifically tailored to developers to quickly see the
    /// transactions in the pool and find any potential issues.
    ///
    /// See [here](https://geth.ethereum.org/docs/rpc/ns-txpool#txpool_inspect) for more details
    async fn txpool_inspect(&self) -> TransportResult<TxpoolInspect>;

    /// Returns the current status of the transaction pool.
    ///
    /// i.e the number of transactions currently pending for inclusion in the next block(s), as well
    /// as the ones that are being scheduled for future execution only.
    ///
    /// See [here](https://geth.ethereum.org/docs/rpc/ns-txpool#txpool_status) for more details
    async fn txpool_status(&self) -> TransportResult<TxpoolStatus>;
}

#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
impl<P, N> TxPoolApi<N> for P
where
    P: Provider<N>,
    N: Network,
{
    async fn txpool_content(&self) -> TransportResult<TxpoolContent<N::TransactionResponse>> {
        self.client().request_noparams("txpool_content").await
    }

    async fn txpool_content_from(
        &self,
        from: Address,
    ) -> TransportResult<TxpoolContentFrom<N::TransactionResponse>> {
        self.client().request("txpool_contentFrom", (from,)).await
    }

    async fn txpool_inspect(&self) -> TransportResult<TxpoolInspect> {
        self.client().request_noparams("txpool_inspect").await
    }

    async fn txpool_status(&self) -> TransportResult<TxpoolStatus> {
        self.client().request_noparams("txpool_status").await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ext::test::async_ci_only, ProviderBuilder};
    use alloy_node_bindings::{utils::run_with_tempdir, Geth};

    #[tokio::test]
    async fn test_txpool_content() {
        async_ci_only(|| async move {
            run_with_tempdir("geth-test-", |temp_dir| async move {
                let geth = Geth::new().disable_discovery().data_dir(temp_dir).spawn();
                let provider = ProviderBuilder::new().on_http(geth.endpoint_url());
                let content = provider.txpool_content().await.unwrap();
                assert_eq!(content, TxpoolContent::default());
            })
            .await;
        })
        .await;
    }

    #[tokio::test]
    async fn test_txpool_content_from() {
        async_ci_only(|| async move {
            run_with_tempdir("geth-test-", |temp_dir| async move {
                let geth = Geth::new().disable_discovery().data_dir(temp_dir).spawn();
                let provider = ProviderBuilder::new().on_http(geth.endpoint_url());
                let content = provider.txpool_content_from(Address::default()).await.unwrap();
                assert_eq!(content, TxpoolContentFrom::default());
            })
            .await;
        })
        .await;
    }

    #[tokio::test]
    async fn test_txpool_inspect() {
        async_ci_only(|| async move {
            run_with_tempdir("geth-test-", |temp_dir| async move {
                let geth = Geth::new().disable_discovery().data_dir(temp_dir).spawn();
                let provider = ProviderBuilder::new().on_http(geth.endpoint_url());
                let content = provider.txpool_inspect().await.unwrap();
                assert_eq!(content, TxpoolInspect::default());
            })
            .await;
        })
        .await;
    }

    #[tokio::test]
    async fn test_txpool_status() {
        async_ci_only(|| async move {
            run_with_tempdir("geth-test-", |temp_dir| async move {
                let geth = Geth::new().disable_discovery().data_dir(temp_dir).spawn();
                let provider = ProviderBuilder::new().on_http(geth.endpoint_url());
                let content = provider.txpool_status().await.unwrap();
                assert_eq!(content, TxpoolStatus::default());
            })
            .await;
        })
        .await;
    }
}
