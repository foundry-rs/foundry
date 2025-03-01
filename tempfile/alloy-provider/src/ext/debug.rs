//! This module extends the Ethereum JSON-RPC provider with the Debug namespace's RPC methods.
use crate::Provider;
use alloy_json_rpc::RpcRecv;
use alloy_network::Network;
use alloy_primitives::{hex, Bytes, TxHash, B256};
use alloy_rpc_types_debug::ExecutionWitness;
use alloy_rpc_types_eth::{
    BadBlock, BlockId, BlockNumberOrTag, Bundle, StateContext, TransactionRequest,
};
use alloy_rpc_types_trace::geth::{
    BlockTraceResult, CallFrame, GethDebugTracingCallOptions, GethDebugTracingOptions, GethTrace,
    TraceResult,
};
use alloy_transport::TransportResult;

/// Debug namespace rpc interface that gives access to several non-standard RPC methods.
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
pub trait DebugApi<N>: Send + Sync {
    /// Returns an RLP-encoded header.
    async fn debug_get_raw_header(&self, block: BlockId) -> TransportResult<Bytes>;

    /// Retrieves and returns the RLP encoded block by number, hash or tag.
    async fn debug_get_raw_block(&self, block: BlockId) -> TransportResult<Bytes>;

    /// Returns an EIP-2718 binary-encoded transaction.
    async fn debug_get_raw_transaction(&self, hash: TxHash) -> TransportResult<Bytes>;

    /// Returns an array of EIP-2718 binary-encoded receipts.
    async fn debug_get_raw_receipts(&self, block: BlockId) -> TransportResult<Vec<Bytes>>;

    /// Returns an array of recent bad blocks that the client has seen on the network.
    async fn debug_get_bad_blocks(&self) -> TransportResult<Vec<BadBlock>>;

    /// Returns the structured logs created during the execution of EVM between two blocks
    /// (excluding start) as a JSON object.
    async fn debug_trace_chain(
        &self,
        start_exclusive: BlockNumberOrTag,
        end_inclusive: BlockNumberOrTag,
    ) -> TransportResult<Vec<BlockTraceResult>>;

    /// The debug_traceBlock method will return a full stack trace of all invoked opcodes of all
    /// transaction that were included in this block.
    ///
    /// This expects an RLP-encoded block.
    ///
    /// # Note
    ///
    /// The parent of this block must be present, or it will fail.
    async fn debug_trace_block(
        &self,
        rlp_block: &[u8],
        trace_options: GethDebugTracingOptions,
    ) -> TransportResult<Vec<TraceResult>>;

    /// Reruns the transaction specified by the hash and returns the trace.
    ///
    /// It will replay any prior transactions to achieve the same state the transaction was executed
    /// in.
    ///
    /// [GethDebugTracingOptions] can be used to specify the trace options.
    ///
    /// # Note
    ///
    /// Not all nodes support this call.
    async fn debug_trace_transaction(
        &self,
        hash: TxHash,
        trace_options: GethDebugTracingOptions,
    ) -> TransportResult<GethTrace>;

    /// Reruns the transaction specified by the hash and returns the trace in a specified format.
    ///
    /// This method allows for the trace to be returned as a type that implements `RpcRecv` and
    /// `serde::de::DeserializeOwned`.
    ///
    /// [GethDebugTracingOptions] can be used to specify the trace options.
    ///
    /// # Note
    ///
    /// Not all nodes support this call.
    async fn debug_trace_transaction_as<R>(
        &self,
        hash: TxHash,
        trace_options: GethDebugTracingOptions,
    ) -> TransportResult<R>
    where
        R: RpcRecv + serde::de::DeserializeOwned;

    /// Reruns the transaction specified by the hash and returns the trace as a JSON object.
    ///
    /// This method provides the trace in a JSON format, which can be useful for further processing
    /// or inspection.
    ///
    /// [GethDebugTracingOptions] can be used to specify the trace options.
    ///
    /// # Note
    ///
    /// Not all nodes support this call.
    async fn debug_trace_transaction_js(
        &self,
        hash: TxHash,
        trace_options: GethDebugTracingOptions,
    ) -> TransportResult<serde_json::Value>;

    /// Reruns the transaction specified by the hash and returns the trace as a call frame.
    ///
    /// This method provides the trace in the form of a `CallFrame`, which can be useful for
    /// analyzing the call stack and execution details.
    ///
    /// [GethDebugTracingOptions] can be used to specify the trace options.
    ///
    /// # Note
    ///
    /// Not all nodes support this call.
    async fn debug_trace_transaction_call(
        &self,
        hash: TxHash,
        trace_options: GethDebugTracingOptions,
    ) -> TransportResult<CallFrame>;

    /// Reruns the transaction specified by the hash and returns the trace in a specified format.
    ///
    /// This method allows for the trace to be returned as a type that implements `RpcRecv` and
    /// `serde::de::DeserializeOwned`.
    ///
    /// [GethDebugTracingOptions] can be used to specify the trace options.
    ///
    /// # Note
    ///
    /// Not all nodes support this call.
    async fn debug_trace_call_as<R>(
        &self,
        tx: TransactionRequest,
        block: BlockId,
        trace_options: GethDebugTracingCallOptions,
    ) -> TransportResult<R>
    where
        R: RpcRecv + serde::de::DeserializeOwned;

    /// Reruns the transaction specified by the hash and returns the trace as a JSON object.
    ///
    /// This method provides the trace in a JSON format, which can be useful for further processing
    /// or inspection.
    ///
    /// [GethDebugTracingOptions] can be used to specify the trace options.
    ///
    /// # Note
    ///
    /// Not all nodes support this call.
    async fn debug_trace_call_js(
        &self,
        tx: TransactionRequest,
        block: BlockId,
        trace_options: GethDebugTracingCallOptions,
    ) -> TransportResult<serde_json::Value>;

    /// Reruns the transaction specified by the hash and returns the trace as a call frame.
    ///
    /// This method provides the trace in the form of a `CallFrame`, which can be useful for
    /// analyzing the call stack and execution details.
    ///
    /// [GethDebugTracingOptions] can be used to specify the trace options.
    ///
    /// # Note
    ///
    /// Not all nodes support this call.
    async fn debug_trace_call_callframe(
        &self,
        tx: TransactionRequest,
        block: BlockId,
        trace_options: GethDebugTracingCallOptions,
    ) -> TransportResult<CallFrame>;

    /// Return a full stack trace of all invoked opcodes of all transaction that were included in
    /// this block.
    ///
    /// The parent of the block must be present or it will fail.
    ///
    /// [GethDebugTracingOptions] can be used to specify the trace options.
    ///
    /// # Note
    ///
    /// Not all nodes support this call.
    async fn debug_trace_block_by_hash(
        &self,
        block: B256,
        trace_options: GethDebugTracingOptions,
    ) -> TransportResult<Vec<TraceResult>>;

    /// Same as `debug_trace_block_by_hash` but block is specified by number.
    ///
    /// [GethDebugTracingOptions] can be used to specify the trace options.
    ///
    /// # Note
    ///
    /// Not all nodes support this call.
    async fn debug_trace_block_by_number(
        &self,
        block: BlockNumberOrTag,
        trace_options: GethDebugTracingOptions,
    ) -> TransportResult<Vec<TraceResult>>;

    /// Executes the given transaction without publishing it like `eth_call` and returns the trace
    /// of the execution.
    ///
    /// The transaction will be executed in the context of the given block number or tag.
    /// The state its run on is the state of the previous block.
    ///
    /// [GethDebugTracingOptions] can be used to specify the trace options.
    ///
    /// # Note
    ///
    ///
    /// Not all nodes support this call.
    async fn debug_trace_call(
        &self,
        tx: TransactionRequest,
        block: BlockId,
        trace_options: GethDebugTracingCallOptions,
    ) -> TransportResult<GethTrace>;

    /// Same as `debug_trace_call` but it used to run and trace multiple transactions at once.
    ///
    /// [GethDebugTracingOptions] can be used to specify the trace options.
    ///
    /// # Note
    ///
    /// Not all nodes support this call.
    async fn debug_trace_call_many(
        &self,
        bundles: Vec<Bundle>,
        state_context: StateContext,
        trace_options: GethDebugTracingCallOptions,
    ) -> TransportResult<Vec<GethTrace>>;

    /// The `debug_executionWitness` method allows for re-execution of a block with the purpose of
    /// generating an execution witness. The witness comprises of a map of all hashed trie nodes to
    /// their preimages that were required during the execution of the block, including during
    /// state root recomputation.
    ///
    /// The first argument is the block number or block hash.
    ///
    /// # Note
    ///
    /// Not all nodes support this call.
    async fn debug_execution_witness(
        &self,
        block: BlockNumberOrTag,
    ) -> TransportResult<ExecutionWitness>;
}

#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
impl<N, P> DebugApi<N> for P
where
    N: Network,
    P: Provider<N>,
{
    async fn debug_get_raw_header(&self, block: BlockId) -> TransportResult<Bytes> {
        self.client().request("debug_getRawHeader", (block,)).await
    }

    async fn debug_get_raw_block(&self, block: BlockId) -> TransportResult<Bytes> {
        self.client().request("debug_getRawBlock", (block,)).await
    }

    async fn debug_get_raw_transaction(&self, hash: TxHash) -> TransportResult<Bytes> {
        self.client().request("debug_getRawTransaction", (hash,)).await
    }

    async fn debug_get_raw_receipts(&self, block: BlockId) -> TransportResult<Vec<Bytes>> {
        self.client().request("debug_getRawReceipts", (block,)).await
    }

    async fn debug_get_bad_blocks(&self) -> TransportResult<Vec<BadBlock>> {
        self.client().request_noparams("debug_getBadBlocks").await
    }

    async fn debug_trace_chain(
        &self,
        start_exclusive: BlockNumberOrTag,
        end_inclusive: BlockNumberOrTag,
    ) -> TransportResult<Vec<BlockTraceResult>> {
        self.client().request("debug_traceChain", (start_exclusive, end_inclusive)).await
    }

    async fn debug_trace_block(
        &self,
        rlp_block: &[u8],
        trace_options: GethDebugTracingOptions,
    ) -> TransportResult<Vec<TraceResult>> {
        let rlp_block = hex::encode_prefixed(rlp_block);
        self.client().request("debug_traceBlock", (rlp_block, trace_options)).await
    }

    async fn debug_trace_transaction(
        &self,
        hash: TxHash,
        trace_options: GethDebugTracingOptions,
    ) -> TransportResult<GethTrace> {
        self.client().request("debug_traceTransaction", (hash, trace_options)).await
    }

    async fn debug_trace_transaction_as<R>(
        &self,
        hash: TxHash,
        trace_options: GethDebugTracingOptions,
    ) -> TransportResult<R>
    where
        R: RpcRecv,
    {
        self.client().request("debug_traceTransaction", (hash, trace_options)).await
    }

    async fn debug_trace_transaction_js(
        &self,
        hash: TxHash,
        trace_options: GethDebugTracingOptions,
    ) -> TransportResult<serde_json::Value> {
        self.debug_trace_transaction_as::<serde_json::Value>(hash, trace_options).await
    }

    async fn debug_trace_transaction_call(
        &self,
        hash: TxHash,
        trace_options: GethDebugTracingOptions,
    ) -> TransportResult<CallFrame> {
        self.debug_trace_transaction_as::<CallFrame>(hash, trace_options).await
    }

    async fn debug_trace_call_as<R>(
        &self,
        tx: TransactionRequest,
        block: BlockId,
        trace_options: GethDebugTracingCallOptions,
    ) -> TransportResult<R>
    where
        R: RpcRecv,
    {
        self.client().request("debug_traceCall", (tx, block, trace_options)).await
    }

    async fn debug_trace_call_js(
        &self,
        tx: TransactionRequest,
        block: BlockId,
        trace_options: GethDebugTracingCallOptions,
    ) -> TransportResult<serde_json::Value> {
        self.debug_trace_call_as::<serde_json::Value>(tx, block, trace_options).await
    }

    async fn debug_trace_call_callframe(
        &self,
        tx: TransactionRequest,
        block: BlockId,
        trace_options: GethDebugTracingCallOptions,
    ) -> TransportResult<CallFrame> {
        self.debug_trace_call_as::<CallFrame>(tx, block, trace_options).await
    }

    async fn debug_trace_block_by_hash(
        &self,
        block: B256,
        trace_options: GethDebugTracingOptions,
    ) -> TransportResult<Vec<TraceResult>> {
        self.client().request("debug_traceBlockByHash", (block, trace_options)).await
    }

    async fn debug_trace_block_by_number(
        &self,
        block: BlockNumberOrTag,
        trace_options: GethDebugTracingOptions,
    ) -> TransportResult<Vec<TraceResult>> {
        self.client().request("debug_traceBlockByNumber", (block, trace_options)).await
    }

    async fn debug_trace_call(
        &self,
        tx: TransactionRequest,
        block: BlockId,
        trace_options: GethDebugTracingCallOptions,
    ) -> TransportResult<GethTrace> {
        self.client().request("debug_traceCall", (tx, block, trace_options)).await
    }

    async fn debug_trace_call_many(
        &self,
        bundles: Vec<Bundle>,
        state_context: StateContext,
        trace_options: GethDebugTracingCallOptions,
    ) -> TransportResult<Vec<GethTrace>> {
        self.client().request("debug_traceCallMany", (bundles, state_context, trace_options)).await
    }

    async fn debug_execution_witness(
        &self,
        block: BlockNumberOrTag,
    ) -> TransportResult<ExecutionWitness> {
        self.client().request("debug_executionWitness", block).await
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::{ext::test::async_ci_only, ProviderBuilder, WalletProvider};
    use alloy_network::TransactionBuilder;
    use alloy_node_bindings::{utils::run_with_tempdir, Geth, Reth};
    use alloy_primitives::{address, U256};

    #[tokio::test]
    async fn test_debug_trace_transaction() {
        async_ci_only(|| async move {
            let provider = ProviderBuilder::new().on_anvil_with_wallet();
            let from = provider.default_signer_address();

            let gas_price = provider.get_gas_price().await.unwrap();
            let tx = TransactionRequest::default()
                .from(from)
                .to(address!("deadbeef00000000deadbeef00000000deadbeef"))
                .value(U256::from(100))
                .max_fee_per_gas(gas_price + 1)
                .max_priority_fee_per_gas(gas_price + 1);
            let pending = provider.send_transaction(tx).await.unwrap();
            let receipt = pending.get_receipt().await.unwrap();

            let hash = receipt.transaction_hash;
            let trace_options = GethDebugTracingOptions::default();

            let trace = provider.debug_trace_transaction(hash, trace_options).await.unwrap();

            if let GethTrace::Default(trace) = trace {
                assert_eq!(trace.gas, 21000)
            }
        })
        .await;
    }

    #[tokio::test]
    async fn test_debug_trace_call() {
        async_ci_only(|| async move {
            let provider = ProviderBuilder::new().on_anvil_with_wallet();
            let from = provider.default_signer_address();
            let gas_price = provider.get_gas_price().await.unwrap();
            let tx = TransactionRequest::default()
                .from(from)
                .with_input("0xdeadbeef")
                .max_fee_per_gas(gas_price + 1)
                .max_priority_fee_per_gas(gas_price + 1);

            let trace = provider
                .debug_trace_call(
                    tx,
                    BlockNumberOrTag::Latest.into(),
                    GethDebugTracingCallOptions::default(),
                )
                .await
                .unwrap();

            if let GethTrace::Default(trace) = trace {
                assert!(!trace.struct_logs.is_empty());
            }
        })
        .await;
    }

    #[tokio::test]
    async fn call_debug_get_raw_header() {
        async_ci_only(|| async move {
            run_with_tempdir("geth-test-", |temp_dir| async move {
                let geth = Geth::new().disable_discovery().data_dir(temp_dir).spawn();
                let provider = ProviderBuilder::new().on_http(geth.endpoint_url());

                let rlp_header = provider
                    .debug_get_raw_header(BlockId::Number(BlockNumberOrTag::Latest))
                    .await
                    .expect("debug_getRawHeader call should succeed");

                assert!(!rlp_header.is_empty());
            })
            .await;
        })
        .await;
    }

    #[tokio::test]
    async fn call_debug_get_raw_block() {
        async_ci_only(|| async move {
            run_with_tempdir("geth-test-", |temp_dir| async move {
                let geth = Geth::new().disable_discovery().data_dir(temp_dir).spawn();
                let provider = ProviderBuilder::new().on_http(geth.endpoint_url());

                let rlp_block = provider
                    .debug_get_raw_block(BlockId::Number(BlockNumberOrTag::Latest))
                    .await
                    .expect("debug_getRawBlock call should succeed");

                assert!(!rlp_block.is_empty());
            })
            .await;
        })
        .await;
    }

    #[tokio::test]
    async fn call_debug_get_raw_receipts() {
        async_ci_only(|| async move {
            run_with_tempdir("geth-test-", |temp_dir| async move {
                let geth = Geth::new().disable_discovery().data_dir(temp_dir).spawn();
                let provider = ProviderBuilder::new().on_http(geth.endpoint_url());

                let result = provider
                    .debug_get_raw_receipts(BlockId::Number(BlockNumberOrTag::Latest))
                    .await;
                assert!(result.is_ok());
            })
            .await;
        })
        .await;
    }

    #[tokio::test]
    async fn call_debug_get_bad_blocks() {
        async_ci_only(|| async move {
            run_with_tempdir("geth-test-", |temp_dir| async move {
                let geth = Geth::new().disable_discovery().data_dir(temp_dir).spawn();
                let provider = ProviderBuilder::new().on_http(geth.endpoint_url());

                let result = provider.debug_get_bad_blocks().await;
                assert!(result.is_ok());
            })
            .await;
        })
        .await;
    }

    #[tokio::test]
    #[cfg_attr(windows, ignore)]
    async fn debug_trace_call_many() {
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

                let bundles = vec![Bundle { transactions: vec![tx1, tx2], block_override: None }];
                let state_context = StateContext::default();
                let trace_options = GethDebugTracingCallOptions::default();
                let result =
                    provider.debug_trace_call_many(bundles, state_context, trace_options).await;
                assert!(result.is_ok());

                let traces = result.unwrap();
                assert_eq!(
                    serde_json::to_string_pretty(&traces).unwrap().trim(),
                    r#"
[
  [
    {
      "failed": false,
      "gas": 21000,
      "returnValue": "",
      "structLogs": []
    },
    {
      "failed": false,
      "gas": 21000,
      "returnValue": "",
      "structLogs": []
    }
  ]
]
"#
                    .trim(),
                );
            })
            .await;
        })
        .await;
    }
}
