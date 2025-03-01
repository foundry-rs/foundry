//! This module extends the Ethereum JSON-RPC provider with the Trace namespace's RPC methods.
use crate::{Provider, RpcWithBlock};
use alloy_eips::BlockId;
use alloy_network::Network;
use alloy_primitives::TxHash;
use alloy_rpc_types_eth::Index;
use alloy_rpc_types_trace::{
    filter::TraceFilter,
    parity::{LocalizedTransactionTrace, TraceResults, TraceResultsWithTransactionHash, TraceType},
};
use alloy_transport::TransportResult;

/// List of trace calls for use with [`TraceApi::trace_call_many`]
pub type TraceCallList<'a, N> = &'a [(<N as Network>::TransactionRequest, &'a [TraceType])];

/// Trace namespace rpc interface that gives access to several non-standard RPC methods.
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
pub trait TraceApi<N>: Send + Sync
where
    N: Network,
{
    /// Executes the given transaction and returns a number of possible traces.
    ///
    /// # Note
    ///
    /// Not all nodes support this call.
    fn trace_call<'a, 'b>(
        &self,
        request: &'a N::TransactionRequest,
        trace_type: &'b [TraceType],
    ) -> RpcWithBlock<(&'a N::TransactionRequest, &'b [TraceType]), TraceResults>;

    /// Traces multiple transactions on top of the same block, i.e. transaction `n` will be executed
    /// on top of the given block with all `n - 1` transaction applied first.
    ///
    /// Allows tracing dependent transactions.
    ///
    /// # Note
    ///
    /// Not all nodes support this call.
    fn trace_call_many<'a>(
        &self,
        request: TraceCallList<'a, N>,
    ) -> RpcWithBlock<(TraceCallList<'a, N>,), Vec<TraceResults>>;

    /// Parity trace transaction.
    async fn trace_transaction(
        &self,
        hash: TxHash,
    ) -> TransportResult<Vec<LocalizedTransactionTrace>>;

    /// Traces of the transaction on the given positions
    ///
    /// # Note
    ///
    /// This function accepts single index and build list with it under the hood because
    /// trace_get method accepts list of indices but limits this list to len == 1.
    async fn trace_get(
        &self,
        hash: TxHash,
        index: usize,
    ) -> TransportResult<LocalizedTransactionTrace>;

    /// Trace the given raw transaction.
    async fn trace_raw_transaction(
        &self,
        data: &[u8],
        trace_type: &[TraceType],
    ) -> TransportResult<TraceResults>;

    /// Traces matching given filter.
    async fn trace_filter(
        &self,
        tracer: &TraceFilter,
    ) -> TransportResult<Vec<LocalizedTransactionTrace>>;

    /// Trace all transactions in the given block.
    ///
    /// # Note
    ///
    /// Not all nodes support this call.
    async fn trace_block(&self, block: BlockId) -> TransportResult<Vec<LocalizedTransactionTrace>>;

    /// Replays a transaction.
    async fn trace_replay_transaction(
        &self,
        hash: TxHash,
        trace_types: &[TraceType],
    ) -> TransportResult<TraceResults>;

    /// Replays all transactions in the given block.
    async fn trace_replay_block_transactions(
        &self,
        block: BlockId,
        trace_types: &[TraceType],
    ) -> TransportResult<Vec<TraceResultsWithTransactionHash>>;
}

#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
impl<N, P> TraceApi<N> for P
where
    N: Network,
    P: Provider<N>,
{
    fn trace_call<'a, 'b>(
        &self,
        request: &'a <N as Network>::TransactionRequest,
        trace_types: &'b [TraceType],
    ) -> RpcWithBlock<(&'a <N as Network>::TransactionRequest, &'b [TraceType]), TraceResults> {
        self.client().request("trace_call", (request, trace_types)).into()
    }

    fn trace_call_many<'a>(
        &self,
        request: TraceCallList<'a, N>,
    ) -> RpcWithBlock<(TraceCallList<'a, N>,), Vec<TraceResults>> {
        self.client().request("trace_callMany", (request,)).into()
    }

    async fn trace_transaction(
        &self,
        hash: TxHash,
    ) -> TransportResult<Vec<LocalizedTransactionTrace>> {
        self.client().request("trace_transaction", (hash,)).await
    }

    async fn trace_get(
        &self,
        hash: TxHash,
        index: usize,
    ) -> TransportResult<LocalizedTransactionTrace> {
        // We are using `[index]` because API accepts a list, but only supports a single index
        self.client().request("trace_get", (hash, (Index::from(index),))).await
    }

    async fn trace_raw_transaction(
        &self,
        data: &[u8],
        trace_types: &[TraceType],
    ) -> TransportResult<TraceResults> {
        self.client().request("trace_rawTransaction", (data, trace_types)).await
    }

    async fn trace_filter(
        &self,
        tracer: &TraceFilter,
    ) -> TransportResult<Vec<LocalizedTransactionTrace>> {
        self.client().request("trace_filter", (tracer,)).await
    }

    async fn trace_block(&self, block: BlockId) -> TransportResult<Vec<LocalizedTransactionTrace>> {
        self.client().request("trace_block", (block,)).await
    }

    async fn trace_replay_transaction(
        &self,
        hash: TxHash,
        trace_types: &[TraceType],
    ) -> TransportResult<TraceResults> {
        self.client().request("trace_replayTransaction", (hash, trace_types)).await
    }

    async fn trace_replay_block_transactions(
        &self,
        block: BlockId,
        trace_types: &[TraceType],
    ) -> TransportResult<Vec<TraceResultsWithTransactionHash>> {
        self.client().request("trace_replayBlockTransactions", (block, trace_types)).await
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::{ext::test::async_ci_only, ProviderBuilder};
    use alloy_eips::BlockNumberOrTag;
    use alloy_network::TransactionBuilder;
    use alloy_node_bindings::{utils::run_with_tempdir, Reth};
    use alloy_primitives::address;
    use alloy_rpc_types_eth::TransactionRequest;

    #[tokio::test]
    async fn trace_block() {
        let provider = ProviderBuilder::new().on_anvil();
        let traces = provider.trace_block(BlockId::Number(BlockNumberOrTag::Latest)).await.unwrap();
        assert_eq!(traces.len(), 0);
    }

    #[tokio::test]
    #[cfg_attr(windows, ignore)]
    async fn trace_call() {
        async_ci_only(|| async move {
            run_with_tempdir("reth-test-", |temp_dir| async move {
                let reth = Reth::new().dev().disable_discovery().data_dir(temp_dir).spawn();
                let provider = ProviderBuilder::new().on_http(reth.endpoint_url());

                let tx = TransactionRequest::default()
                    .with_from(address!("0000000000000000000000000000000000000123"))
                    .with_to(address!("0000000000000000000000000000000000000456"));

                let result = provider.trace_call(&tx, &[TraceType::Trace]).await;
                assert!(result.is_ok());

                let traces = result.unwrap();
                assert_eq!(
                    serde_json::to_string_pretty(&traces).unwrap().trim(),
                    r#"
{
  "output": "0x",
  "stateDiff": null,
  "trace": [
    {
      "type": "call",
      "action": {
        "from": "0x0000000000000000000000000000000000000123",
        "callType": "call",
        "gas": "0x2fa9e78",
        "input": "0x",
        "to": "0x0000000000000000000000000000000000000456",
        "value": "0x0"
      },
      "result": {
        "gasUsed": "0x0",
        "output": "0x"
      },
      "subtraces": 0,
      "traceAddress": []
    }
  ],
  "vmTrace": null
}
"#
                    .trim(),
                );
            })
            .await;
        })
        .await;
    }

    #[tokio::test]
    #[cfg_attr(windows, ignore)]
    async fn trace_call_many() {
        async_ci_only(|| async move {
            run_with_tempdir("reth-test-", |temp_dir| async move {
                let reth = Reth::new().dev().disable_discovery().data_dir(temp_dir).spawn();
                let provider = ProviderBuilder::new().on_http(reth.endpoint_url());

                let tx1 = TransactionRequest::default()
                    .with_from(address!("0000000000000000000000000000000000000123"))
                    .with_to(address!("0000000000000000000000000000000000000456"));

                let tx2 = TransactionRequest::default()
                    .with_from(address!("0000000000000000000000000000000000000456"))
                    .with_to(address!("0000000000000000000000000000000000000789"));

                let result = provider
                    .trace_call_many(&[(tx1, &[TraceType::Trace]), (tx2, &[TraceType::Trace])])
                    .await;
                assert!(result.is_ok());

                let traces = result.unwrap();
                assert_eq!(
                    serde_json::to_string_pretty(&traces).unwrap().trim(),
                    r#"
[
  {
    "output": "0x",
    "stateDiff": null,
    "trace": [
      {
        "type": "call",
        "action": {
          "from": "0x0000000000000000000000000000000000000123",
          "callType": "call",
          "gas": "0x2fa9e78",
          "input": "0x",
          "to": "0x0000000000000000000000000000000000000456",
          "value": "0x0"
        },
        "result": {
          "gasUsed": "0x0",
          "output": "0x"
        },
        "subtraces": 0,
        "traceAddress": []
      }
    ],
    "vmTrace": null
  },
  {
    "output": "0x",
    "stateDiff": null,
    "trace": [
      {
        "type": "call",
        "action": {
          "from": "0x0000000000000000000000000000000000000456",
          "callType": "call",
          "gas": "0x2fa9e78",
          "input": "0x",
          "to": "0x0000000000000000000000000000000000000789",
          "value": "0x0"
        },
        "result": {
          "gasUsed": "0x0",
          "output": "0x"
        },
        "subtraces": 0,
        "traceAddress": []
      }
    ],
    "vmTrace": null
  }
]
"#
                    .trim()
                );
            })
            .await;
        })
        .await;
    }
}
