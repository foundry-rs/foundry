use crate::{server::SharedModule, state_dump::SerializableState, types::ReorgOptions};
use alloy_consensus::{Blob, TxEnvelope};
use alloy_dyn_abi::TypedData;
use alloy_eips::{BlockId, BlockNumberOrTag, eip7910::EthConfig};
use alloy_primitives::{Address, B256, Bytes, U256};
use alloy_rpc_types::{
    anvil::{Forking, Metadata, MineOptions, NodeInfo},
    debug::ExecutionWitness,
    trace::{
        filter::TraceFilter,
        geth::{GethDebugTracingCallOptions, GethDebugTracingOptions, GethTrace, TraceResult},
        otterscan::{
            BlockDetails, ContractCreator, InternalOperation, OtsBlockTransactions, TraceEntry,
            TransactionsWithReceipts,
        },
        parity::{
            LocalizedTransactionTrace, TraceResults, TraceResultsWithTransactionHash, TraceType,
        },
    },
    txpool::TxpoolStatus,
};
use alloy_rpc_types_eth::{
    AccessListResult, Account, Block, EIP1186AccountProofResponse, FeeHistory, FillTransaction,
    Header, Index, Transaction, TransactionReceipt, TransactionRequest,
    simulate::{SimulatePayload, SimulatedBlock},
    state::{EvmOverrides, StateOverride},
};
use alloy_serde::WithOtherFields;
use eyre::Result;
use jsonrpsee::core::params::ArrayParams;
use parking_lot::RwLock;
use serde::de::DeserializeOwned;
use std::{collections::HashSet, sync::Arc};
use tokio::sync::oneshot;

/// In-process access to the node's RPC handlers.
///
/// Every call runs the same handler the HTTP and WebSocket servers run, without a transport.
#[derive(Clone, Debug)]
pub struct EthApi {
    /// The RPC module of the current node. A relaunch replaces it.
    module: SharedModule,
    /// The instance id, shared with the `anvil_*` namespace, which rotates it on `anvil_reset`.
    instance_id: Arc<RwLock<B256>>,
    /// Keeps the node running while the API is alive.
    _node: Arc<oneshot::Sender<()>>,
}

impl EthApi {
    /// Creates the API over the node's RPC module.
    pub(crate) const fn new(
        module: SharedModule,
        instance_id: Arc<RwLock<B256>>,
        node: Arc<oneshot::Sender<()>>,
    ) -> Self {
        Self { module, instance_id, _node: node }
    }

    /// Calls an RPC method with positional parameters.
    pub async fn request<R>(&self, method: &str, params: ArrayParams) -> Result<R>
    where
        R: DeserializeOwned + Clone,
    {
        let module = self.module.read().clone();
        Ok(module.call(method, params).await?)
    }

    /// Returns the names of every RPC method the node serves.
    #[doc(hidden)]
    pub fn method_names(&self) -> Vec<String> {
        let mut names: Vec<String> =
            self.module.read().method_names().map(str::to_string).collect();
        names.sort_unstable();
        names
    }

    /// Returns the unique identifier of this node instance. It changes on `anvil_reset`.
    pub fn instance_id(&self) -> B256 {
        *self.instance_id.read()
    }

    /// Returns the current block number.
    pub async fn block_number(&self) -> Result<U256> {
        self.request("eth_blockNumber", ArrayParams::new()).await
    }

    /// Returns the chain id.
    pub async fn chain_id(&self) -> Result<u64> {
        Ok(self.request::<U256>("eth_chainId", ArrayParams::new()).await?.to())
    }

    /// Returns the balance of an account.
    pub async fn balance(&self, address: Address, block: Option<BlockId>) -> Result<U256> {
        self.request("eth_getBalance", params![address, block.unwrap_or_default()]).await
    }

    /// Returns the code of an account.
    pub async fn get_code(&self, address: Address, block: Option<BlockId>) -> Result<Bytes> {
        self.request("eth_getCode", params![address, block.unwrap_or_default()]).await
    }

    /// Returns the account at the given block.
    pub async fn get_account(&self, address: Address, block: Option<BlockId>) -> Result<Account> {
        self.request("eth_getAccount", params![address, block.unwrap_or_default()]).await
    }

    /// Returns the nonce of an account.
    pub async fn transaction_count(
        &self,
        address: Address,
        block: Option<BlockId>,
    ) -> Result<U256> {
        self.request("eth_getTransactionCount", params![address, block.unwrap_or_default()]).await
    }

    /// Returns the receipt of a transaction.
    pub async fn transaction_receipt(&self, hash: B256) -> Result<Option<TransactionReceipt>> {
        self.request("eth_getTransactionReceipt", params![hash]).await
    }

    /// Returns the transaction at the given block and index.
    pub async fn transaction_by_block_number_and_index(
        &self,
        block: BlockNumberOrTag,
        index: Index,
    ) -> Result<Option<Transaction>> {
        self.request("eth_getTransactionByBlockNumberAndIndex", params![block, index]).await
    }

    /// Signs and sends a transaction from a dev or impersonated account.
    pub async fn send_transaction(
        &self,
        request: WithOtherFields<TransactionRequest>,
    ) -> Result<B256> {
        self.request("eth_sendTransaction", params![request]).await
    }

    /// Returns the pool status.
    pub async fn txpool_status(&self) -> Result<TxpoolStatus> {
        self.request("txpool_status", ArrayParams::new()).await
    }

    /// Mines one block.
    pub async fn mine_one(&self) -> Result<()> {
        self.anvil_mine(Some(U256::ONE), None).await
    }

    /// Mines the given number of blocks, with an optional timestamp interval between them.
    pub async fn anvil_mine(&self, blocks: Option<U256>, interval: Option<U256>) -> Result<()> {
        self.request("anvil_mine", params![blocks, interval]).await
    }

    /// Enables or disables automine.
    pub async fn anvil_set_auto_mine(&self, enabled: bool) -> Result<()> {
        self.request("anvil_setAutomine", params![enabled]).await
    }

    /// Impersonates an account.
    pub async fn anvil_impersonate_account(&self, address: Address) -> Result<()> {
        self.request("anvil_impersonateAccount", params![address]).await
    }

    /// Returns the state of the chain as gzipped JSON.
    pub async fn anvil_dump_state(
        &self,
        preserve_historical_states: Option<bool>,
    ) -> Result<Bytes> {
        self.request("anvil_dumpState", params![preserve_historical_states]).await
    }

    /// Returns the state of the chain decoded from `anvil_dumpState`.
    pub async fn serialized_state(
        &self,
        preserve_historical_states: bool,
    ) -> Result<SerializableState> {
        let buf = self.anvil_dump_state(Some(preserve_historical_states)).await?;
        SerializableState::decode(&buf)
    }

    /// Applies a state dump on top of the current state.
    pub async fn anvil_load_state(&self, buf: Bytes) -> Result<bool> {
        self.request("anvil_loadState", params![buf]).await
    }

    /// Resets the chain to genesis, or to the fork block when forking.
    pub async fn anvil_reset(&self, forking: Option<Forking>) -> Result<()> {
        self.request("anvil_reset", params![forking]).await
    }

    /// Sets the TIP-20 balance of an account. Only Tempo serves it.
    pub async fn anvil_deal_tip20(
        &self,
        address: Address,
        token_address: Address,
        balance: U256,
    ) -> Result<()> {
        self.request("anvil_dealTIP20", params![address, token_address, balance]).await
    }

    /// Sets the chain id. The node relaunches with its state and height.
    pub async fn anvil_set_chain_id(&self, chain_id: u64) -> Result<()> {
        self.request("anvil_setChainId", params![chain_id]).await
    }

    /// Sets the balance of an account.
    pub async fn anvil_set_balance(&self, address: Address, balance: U256) -> Result<()> {
        self.request("anvil_setBalance", params![address, balance]).await
    }

    /// Sets the nonce of an account.
    pub async fn anvil_set_nonce(&self, address: Address, nonce: U256) -> Result<()> {
        self.request("anvil_setNonce", params![address, nonce]).await
    }

    /// Sets the code of an account.
    pub async fn anvil_set_code(&self, address: Address, code: Bytes) -> Result<()> {
        self.request("anvil_setCode", params![address, code]).await
    }

    /// Sets one storage slot of an account.
    pub async fn anvil_set_storage_at(
        &self,
        address: Address,
        slot: U256,
        value: B256,
    ) -> Result<bool> {
        self.request("anvil_setStorageAt", params![address, slot, value]).await
    }

    /// Removes a transaction from the pool.
    pub async fn anvil_drop_transaction(&self, hash: B256) -> Result<Option<B256>> {
        self.request("anvil_dropTransaction", params![hash]).await
    }

    /// Sets the base fee of the next block.
    pub async fn anvil_set_next_block_base_fee_per_gas(&self, base_fee: U256) -> Result<()> {
        self.request("anvil_setNextBlockBaseFeePerGas", params![base_fee]).await
    }

    /// Sets the prevrandao of the next block.
    pub async fn anvil_set_next_block_prevrandao(&self, prev_randao: B256) -> Result<()> {
        self.request("anvil_setNextBlockPrevRandao", params![prev_randao]).await
    }

    /// Sets the exact timestamp of the next block.
    pub async fn evm_set_next_block_timestamp(&self, seconds: u64) -> Result<()> {
        self.request("anvil_setNextBlockTimestamp", params![seconds]).await
    }

    /// Snapshots the chain. Returns the snapshot id.
    pub async fn evm_snapshot(&self) -> Result<U256> {
        self.request("anvil_snapshot", ArrayParams::new()).await
    }

    /// Reverts the chain to a snapshot.
    pub async fn evm_revert(&self, id: U256) -> Result<bool> {
        self.request("anvil_revert", params![id]).await
    }

    /// Returns the dev accounts.
    pub async fn accounts(&self) -> Result<Vec<Address>> {
        self.request("eth_accounts", ArrayParams::new()).await
    }

    /// Returns the gas price.
    pub async fn gas_price(&self) -> Result<u128> {
        Ok(self.request::<U256>("eth_gasPrice", ArrayParams::new()).await?.to())
    }

    /// Returns the base fee of the next block.
    pub async fn base_fee(&self) -> Result<Option<U256>> {
        self.request("eth_baseFee", ArrayParams::new()).await
    }

    /// Returns the gas limit of the latest block.
    pub async fn gas_limit(&self) -> Result<U256> {
        Ok(U256::from(self.anvil_node_info().await?.environment.gas_limit))
    }

    /// Returns the receipts of a block.
    pub async fn block_receipts(&self, block: BlockId) -> Result<Option<Vec<TransactionReceipt>>> {
        self.request("eth_getBlockReceipts", params![block]).await
    }

    /// Returns the block with the given number, with transaction hashes.
    pub async fn block_by_number(&self, number: BlockNumberOrTag) -> Result<Option<Block>> {
        self.request("eth_getBlockByNumber", params![number, false]).await
    }

    /// Returns the block with the given number, with full transactions.
    pub async fn block_by_number_full(&self, number: BlockNumberOrTag) -> Result<Option<Block>> {
        self.request("eth_getBlockByNumber", params![number, true]).await
    }

    /// Returns the storage value of an account at a slot.
    pub async fn storage_at(
        &self,
        address: Address,
        index: U256,
        block: Option<BlockId>,
    ) -> Result<B256> {
        self.request("eth_getStorageAt", params![address, index, block.unwrap_or_default()]).await
    }

    /// Returns the fee history.
    pub async fn fee_history(
        &self,
        block_count: U256,
        newest_block: BlockNumberOrTag,
        reward_percentiles: Vec<f64>,
    ) -> Result<FeeHistory> {
        self.request("eth_feeHistory", params![block_count, newest_block, reward_percentiles]).await
    }

    /// Executes a call in the given block and returns its output.
    pub async fn call(
        &self,
        request: WithOtherFields<TransactionRequest>,
        block: Option<BlockId>,
        overrides: EvmOverrides,
    ) -> Result<Bytes> {
        let EvmOverrides { state, block: block_overrides } = overrides;
        self.request(
            "eth_call",
            params![request, block.unwrap_or_default(), state, block_overrides],
        )
        .await
    }

    /// Estimates the gas of a call.
    pub async fn estimate_gas(
        &self,
        request: WithOtherFields<TransactionRequest>,
        block: Option<BlockId>,
        overrides: EvmOverrides,
    ) -> Result<U256> {
        let EvmOverrides { state, block: block_overrides } = overrides;
        self.request("eth_estimateGas", params![request, block, state, block_overrides]).await
    }

    /// Sends a signed transaction.
    pub async fn send_raw_transaction(&self, tx: Bytes) -> Result<B256> {
        self.request("eth_sendRawTransaction", params![tx]).await
    }

    /// Sends a signed transaction and waits for its receipt.
    pub async fn send_raw_transaction_sync(
        &self,
        tx: Bytes,
        timeout_ms: Option<u64>,
    ) -> Result<TransactionReceipt> {
        self.request("eth_sendRawTransactionSync", params![tx, timeout_ms]).await
    }

    /// Signs and sends a transaction from a dev account, and waits for its receipt.
    pub async fn send_transaction_sync(
        &self,
        request: WithOtherFields<TransactionRequest>,
    ) -> Result<TransactionReceipt> {
        self.request("eth_sendTransactionSync", params![request]).await
    }

    /// Fills the missing fields of a transaction request.
    pub async fn fill_transaction(
        &self,
        request: WithOtherFields<TransactionRequest>,
    ) -> Result<FillTransaction<TxEnvelope>> {
        self.request("eth_fillTransaction", params![request]).await
    }

    /// Returns the chain id as a decimal string.
    pub async fn network_id(&self) -> Result<Option<String>> {
        self.request("net_version", ArrayParams::new()).await.map(Some)
    }

    /// Returns the bytecode with the given hash.
    pub async fn debug_code_by_hash(
        &self,
        hash: B256,
        block: Option<BlockId>,
    ) -> Result<Option<Bytes>> {
        self.request("debug_codeByHash", params![hash, block]).await
    }

    /// Returns the execution witness of a block.
    pub async fn debug_execution_witness(
        &self,
        block: BlockNumberOrTag,
    ) -> Result<ExecutionWitness> {
        self.request("debug_executionWitness", params![block]).await
    }

    /// Returns the EIP-2718 encoded transaction with the given hash, mined or pending.
    pub async fn raw_transaction(&self, hash: B256) -> Result<Option<Bytes>> {
        self.request("debug_getRawTransaction", params![hash]).await
    }

    /// Returns the EIP-2718 encoded receipts of a block.
    pub async fn raw_receipts(&self, block: BlockId) -> Result<Vec<Bytes>> {
        self.request("debug_getRawReceipts", params![block]).await
    }

    /// Returns the chain configuration.
    pub async fn config(&self) -> Result<EthConfig> {
        self.request("eth_config", ArrayParams::new()).await
    }

    /// Replays a mined transaction and returns its traces.
    pub async fn trace_replay_transaction(
        &self,
        hash: B256,
        trace_types: HashSet<TraceType>,
    ) -> Result<Option<TraceResults>> {
        self.request("trace_replayTransaction", params![hash, trace_types]).await
    }

    /// Traces a mined transaction with a geth tracer.
    pub async fn debug_trace_transaction(
        &self,
        hash: B256,
        opts: GethDebugTracingOptions,
    ) -> Result<GethTrace> {
        self.request("debug_traceTransaction", params![hash, opts]).await
    }

    /// Traces a call with a geth tracer.
    pub async fn debug_trace_call(
        &self,
        request: WithOtherFields<TransactionRequest>,
        block: Option<BlockId>,
        opts: GethDebugTracingCallOptions,
    ) -> Result<GethTrace> {
        self.request("debug_traceCall", params![request, block, opts]).await
    }

    /// Traces the transactions of a block by number with a geth tracer.
    pub async fn debug_trace_block_by_number(
        &self,
        number: BlockNumberOrTag,
        opts: GethDebugTracingOptions,
    ) -> Result<Vec<TraceResult>> {
        self.request("debug_traceBlockByNumber", params![number, opts]).await
    }

    /// Traces the transactions of a block by hash with a geth tracer.
    pub async fn debug_trace_block_by_hash(
        &self,
        hash: B256,
        opts: GethDebugTracingOptions,
    ) -> Result<Vec<TraceResult>> {
        self.request("debug_traceBlockByHash", params![hash, opts]).await
    }

    /// Replays the transactions of a block and returns their traces.
    pub async fn trace_replay_block_transactions(
        &self,
        block: BlockNumberOrTag,
        trace_types: HashSet<TraceType>,
    ) -> Result<Option<Vec<TraceResultsWithTransactionHash>>> {
        self.request("trace_replayBlockTransactions", params![block, trace_types]).await
    }

    /// Returns the traces matching a filter.
    pub async fn trace_filter(
        &self,
        filter: TraceFilter,
    ) -> Result<Vec<LocalizedTransactionTrace>> {
        self.request("trace_filter", params![filter]).await
    }

    /// Returns the state root of the latest block.
    pub async fn state_root(&self) -> Result<Option<B256>> {
        let block = self.block_by_number(BlockNumberOrTag::Latest).await?;
        Ok(block.map(|block| block.header.state_root))
    }

    /// Executes a call and returns its traces.
    pub async fn trace_call(
        &self,
        request: WithOtherFields<TransactionRequest>,
        trace_types: HashSet<TraceType>,
        block: Option<BlockId>,
    ) -> Result<TraceResults> {
        self.request("trace_call", params![request, trace_types, block]).await
    }

    /// Executes calls on top of each other and returns their traces.
    pub async fn trace_call_many(
        &self,
        calls: Vec<(WithOtherFields<TransactionRequest>, HashSet<TraceType>)>,
        block: Option<BlockId>,
    ) -> Result<Vec<TraceResults>> {
        self.request("trace_callMany", params![calls, block]).await
    }

    /// Signs a transaction with a dev account and returns the encoded transaction.
    pub async fn sign_transaction(
        &self,
        request: WithOtherFields<TransactionRequest>,
    ) -> Result<String> {
        self.request("eth_signTransaction", params![request]).await
    }

    /// Signs typed data with a dev account.
    pub async fn sign_typed_data_v4(&self, address: Address, data: &TypedData) -> Result<String> {
        self.request("eth_signTypedData_v4", params![address, data]).await
    }

    /// Returns the Merkle proof of an account and its storage slots.
    pub async fn get_proof(
        &self,
        address: Address,
        keys: Vec<B256>,
        block: Option<BlockId>,
    ) -> Result<EIP1186AccountProofResponse> {
        self.request("eth_getProof", params![address, keys, block]).await
    }

    /// Simulates blocks of calls on top of a block.
    pub async fn simulate_v1(
        &self,
        payload: SimulatePayload,
        block: Option<BlockId>,
    ) -> Result<Vec<SimulatedBlock<Block>>> {
        self.request("eth_simulateV1", params![payload, block]).await
    }

    /// Returns the pool blobs of a transaction.
    pub async fn anvil_get_blob_by_tx_hash(&self, hash: B256) -> Result<Option<Vec<Blob>>> {
        let encoded: Option<Vec<String>> =
            self.request("anvil_getBlobsByTransactionHash", params![hash]).await?;
        let Some(encoded) = encoded else { return Ok(None) };
        // Decode in place: a blob is 128 KiB, so moving one through the stack is a cost, and a
        // few of them more than a test thread's stack holds.
        let mut blobs = Vec::with_capacity(encoded.len());
        for hex in &encoded {
            blobs.push(Blob::ZERO);
            let blob = blobs.last_mut().expect("just pushed");
            alloy_primitives::hex::decode_to_slice(hex, blob.as_mut_slice())?;
        }
        Ok(Some(blobs))
    }

    /// Returns the pool blob with the given versioned hash.
    pub async fn anvil_get_blob_by_versioned_hash(&self, hash: B256) -> Result<Option<Box<Blob>>> {
        let blob: Option<String> = self.request("anvil_getBlobByHash", params![hash]).await?;
        blob.as_deref().map(decode_blob).transpose()
    }

    /// Attributes the transactions carrying `signature` to `address`.
    pub async fn anvil_impersonate_signature(
        &self,
        signature: Bytes,
        address: Address,
    ) -> Result<()> {
        self.request("anvil_impersonateSignature", params![signature, address]).await
    }

    /// Returns the interval mining period in seconds, if interval mining is enabled.
    pub async fn anvil_get_interval_mining(&self) -> Result<Option<u64>> {
        self.request("anvil_getIntervalMining", ArrayParams::new()).await
    }

    /// Returns the block access list of a block by hash.
    pub async fn block_access_list_by_hash(&self, hash: B256) -> Result<Option<serde_json::Value>> {
        self.request("eth_getBlockAccessListByBlockHash", params![hash]).await
    }

    /// Returns the block access list of a block by number.
    pub async fn block_access_list_by_number(
        &self,
        number: BlockNumberOrTag,
    ) -> Result<Option<serde_json::Value>> {
        self.request("eth_getBlockAccessListByBlockNumber", params![number]).await
    }

    /// Returns a block by hash, with transaction hashes.
    pub async fn block_by_hash(&self, hash: B256) -> Result<Option<Block>> {
        self.request("eth_getBlockByHash", params![hash, false]).await
    }

    /// Returns the transaction count of a block.
    pub async fn block_transaction_count_by_number(
        &self,
        number: BlockNumberOrTag,
    ) -> Result<Option<U256>> {
        self.request("eth_getBlockTransactionCountByNumber", params![number]).await
    }

    /// Returns a transaction by hash, mined or pending.
    pub async fn transaction_by_hash(&self, hash: B256) -> Result<Option<Transaction>> {
        self.request("eth_getTransactionByHash", params![hash]).await
    }

    /// Returns a block header by number.
    pub async fn erigon_get_header_by_number(
        &self,
        number: BlockNumberOrTag,
    ) -> Result<Option<Block>> {
        self.request("erigon_getHeaderByNumber", params![number]).await
    }

    /// Returns the Otterscan API level.
    pub async fn ots_get_api_level(&self) -> Result<u64> {
        self.request("ots_getApiLevel", ArrayParams::new()).await
    }

    /// Returns the internal ETH transfers of a transaction.
    pub async fn ots_get_internal_operations(&self, hash: B256) -> Result<Vec<InternalOperation>> {
        self.request("ots_getInternalOperations", params![hash]).await
    }

    /// Returns whether an address has code at a block.
    pub async fn ots_has_code(&self, address: Address, number: BlockNumberOrTag) -> Result<bool> {
        self.request("ots_hasCode", params![address, number]).await
    }

    /// Returns the call trace of a transaction.
    pub async fn ots_trace_transaction(&self, hash: B256) -> Result<Vec<TraceEntry>> {
        self.request("ots_traceTransaction", params![hash]).await
    }

    /// Returns the revert data of a transaction.
    pub async fn ots_get_transaction_error(&self, hash: B256) -> Result<Bytes> {
        self.request("ots_getTransactionError", params![hash]).await
    }

    /// Returns a block with its issuance and fees.
    pub async fn ots_get_block_details(&self, number: BlockNumberOrTag) -> Result<BlockDetails> {
        self.request("ots_getBlockDetails", params![number]).await
    }

    /// Returns a block with its issuance and fees, by hash.
    pub async fn ots_get_block_details_by_hash(&self, hash: B256) -> Result<BlockDetails> {
        self.request("ots_getBlockDetailsByHash", params![hash]).await
    }

    /// Returns a page of a block's transactions with their receipts.
    pub async fn ots_get_block_transactions(
        &self,
        number: u64,
        page: usize,
        page_size: usize,
    ) -> Result<OtsBlockTransactions<Transaction, Header>> {
        self.request("ots_getBlockTransactions", params![number, page, page_size]).await
    }

    /// Returns the transactions of an address before a block.
    pub async fn ots_search_transactions_before(
        &self,
        address: Address,
        number: u64,
        page_size: usize,
    ) -> Result<TransactionsWithReceipts<Transaction>> {
        self.request("ots_searchTransactionsBefore", params![address, number, page_size]).await
    }

    /// Returns the transactions of an address after a block.
    pub async fn ots_search_transactions_after(
        &self,
        address: Address,
        number: u64,
        page_size: usize,
    ) -> Result<TransactionsWithReceipts<Transaction>> {
        self.request("ots_searchTransactionsAfter", params![address, number, page_size]).await
    }

    /// Returns the hash of the transaction an address sent with the given nonce.
    pub async fn ots_get_transaction_by_sender_and_nonce(
        &self,
        address: Address,
        nonce: U256,
    ) -> Result<Option<B256>> {
        self.request("ots_getTransactionBySenderAndNonce", params![address, nonce.to::<u64>()])
            .await
    }

    /// Returns the creator of a contract.
    pub async fn ots_get_contract_creator(
        &self,
        address: Address,
    ) -> Result<Option<ContractCreator>> {
        self.request("ots_getContractCreator", params![address]).await
    }

    /// Creates the access list of a call.
    pub async fn create_access_list(
        &self,
        request: WithOtherFields<TransactionRequest>,
        block: Option<BlockId>,
        state_override: Option<StateOverride>,
    ) -> Result<AccessListResult> {
        self.request("eth_createAccessList", params![request, block, state_override]).await
    }

    /// Mines a block, with optional timestamp and block count.
    pub async fn evm_mine(&self, opts: Option<MineOptions>) -> Result<String> {
        self.request("evm_mine", params![opts]).await
    }

    /// Sets the timestamp of the next block and shifts time by the difference.
    pub async fn evm_set_time(&self, timestamp: u64) -> Result<u64> {
        self.request("evm_setTime", params![timestamp]).await
    }

    /// Moves time forward.
    pub async fn evm_increase_time(&self, seconds: U256) -> Result<i64> {
        self.request("evm_increaseTime", params![seconds]).await
    }

    /// Sets the interval between the timestamps of consecutive blocks.
    pub async fn evm_set_block_timestamp_interval(&self, seconds: u64) -> Result<()> {
        self.request("anvil_setBlockTimestampInterval", params![seconds]).await
    }

    /// Removes the block timestamp interval.
    pub async fn evm_remove_block_timestamp_interval(&self) -> Result<bool> {
        self.request("anvil_removeBlockTimestampInterval", ArrayParams::new()).await
    }

    /// Sets the block gas limit.
    pub async fn evm_set_block_gas_limit(&self, gas_limit: U256) -> Result<bool> {
        self.request("evm_setBlockGasLimit", params![gas_limit]).await
    }

    /// Returns whether automine is on.
    pub async fn anvil_get_auto_mine(&self) -> Result<bool> {
        self.request("anvil_getAutomine", ArrayParams::new()).await
    }

    /// Mines a block every `secs` seconds.
    pub async fn anvil_set_interval_mining(&self, secs: u64) -> Result<()> {
        self.request("evm_setIntervalMining", params![secs]).await
    }

    /// Stops impersonating an account.
    pub async fn anvil_stop_impersonating_account(&self, address: Address) -> Result<()> {
        self.request("anvil_stopImpersonatingAccount", params![address]).await
    }

    /// Impersonates every sender.
    pub async fn anvil_auto_impersonate_account(&self, enabled: bool) -> Result<()> {
        self.request("anvil_autoImpersonateAccount", params![enabled]).await
    }

    /// Returns the node info.
    pub async fn anvil_node_info(&self) -> Result<NodeInfo> {
        self.request("anvil_nodeInfo", ArrayParams::new()).await
    }

    /// Returns the node metadata.
    pub async fn anvil_metadata(&self) -> Result<Metadata> {
        self.request("anvil_metadata", ArrayParams::new()).await
    }

    /// Reorganizes the chain.
    pub async fn anvil_reorg(&self, options: ReorgOptions) -> Result<()> {
        self.request("anvil_reorg", params![options]).await
    }

    /// Rolls the chain back by `depth` blocks.
    pub async fn anvil_rollback(&self, depth: Option<u64>) -> Result<()> {
        self.request("anvil_rollback", params![depth]).await
    }

    /// Sets the minimum gas price before London.
    pub async fn anvil_set_min_gas_price(&self, gas_price: U256) -> Result<()> {
        self.request("anvil_setMinGasPrice", params![gas_price]).await
    }

    /// Sets the parent beacon block root of the next block.
    pub async fn anvil_set_next_block_parent_beacon_block_root(&self, root: B256) -> Result<()> {
        self.request("anvil_setNextBlockParentBeaconBlockRoot", params![root]).await
    }

    /// Removes every pool transaction of a sender.
    pub async fn anvil_remove_pool_transactions(&self, address: Address) -> Result<()> {
        self.request("anvil_removePoolTransactions", params![address]).await
    }

    /// Adds to the balance of an account.
    pub async fn anvil_add_balance(&self, address: Address, balance: U256) -> Result<()> {
        self.request("anvil_addBalance", params![address, balance]).await
    }

    /// Sets the coinbase of the following blocks.
    pub async fn anvil_set_coinbase(&self, address: Address) -> Result<()> {
        self.request("anvil_setCoinbase", params![address]).await
    }

    /// Replaces the fork endpoint.
    pub async fn anvil_set_rpc_url(&self, url: String) -> Result<()> {
        self.request("anvil_setRpcUrl", params![url]).await
    }

    /// Sets the ERC-20 balance of an account.
    pub async fn anvil_deal_erc20(
        &self,
        address: Address,
        token: Address,
        balance: U256,
    ) -> Result<()> {
        self.request("anvil_dealERC20", params![address, token, balance]).await
    }

    /// Sets the ERC-20 allowance of a spender.
    pub async fn anvil_set_erc20_allowance(
        &self,
        owner: Address,
        spender: Address,
        token: Address,
        amount: U256,
    ) -> Result<()> {
        self.request("anvil_setERC20Allowance", params![owner, spender, token, amount]).await
    }

    /// Sets the token an account pays fees with. Tempo only.
    pub async fn anvil_set_fee_token(&self, user: Address, token: Address) -> Result<()> {
        self.request("anvil_setFeeToken", params![user, token]).await
    }

    /// Sets the token a validator receives fees in. Tempo only.
    pub async fn anvil_set_validator_fee_token(
        &self,
        validator: Address,
        token: Address,
    ) -> Result<()> {
        self.request("anvil_setValidatorFeeToken", params![validator, token]).await
    }

    /// Adds Fee AMM liquidity for a token pair. Tempo only.
    pub async fn anvil_set_fee_amm_liquidity(
        &self,
        user_token: Address,
        validator_token: Address,
        amount: U256,
    ) -> Result<()> {
        self.request("anvil_setFeeAmmLiquidity", params![user_token, validator_token, amount]).await
    }

    /// Returns the blob base fee of the next block.
    pub async fn blob_base_fee(&self) -> Result<U256> {
        self.request("eth_blobBaseFee", ArrayParams::new()).await
    }

    /// Returns the number of transactions in the block with the given hash.
    pub async fn block_transaction_count_by_hash(&self, hash: B256) -> Result<Option<U256>> {
        self.request("eth_getBlockTransactionCountByHash", params![hash]).await
    }
}

/// Builds positional RPC parameters.
macro_rules! params {
    ($($param:expr),* $(,)?) => {{
        let mut params = ArrayParams::new();
        $(params.insert($param)?;)*
        params
    }};
}
use params;

/// Decodes a hex blob on the heap: a blob is 128 KiB, and deserializing one through serde puts
/// several copies on the stack.
pub(crate) fn decode_blob(hex: &str) -> Result<Box<Blob>> {
    let mut blob = Box::<Blob>::default();
    alloy_primitives::hex::decode_to_slice(hex, blob.as_mut_slice())?;
    Ok(blob)
}

/// Runs `work` on a thread with a large stack and returns its result.
pub(crate) async fn on_large_stack<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> T {
    let (tx, rx) = oneshot::channel();
    std::thread::Builder::new()
        .stack_size(LARGE_STACK_SIZE)
        .spawn(move || {
            let _ = tx.send(work());
        })
        .expect("spawn a thread");
    rx.await.expect("the worker thread finished")
}

/// The stack size of the threads [`on_large_stack`] spawns.
const LARGE_STACK_SIZE: usize = 32 * 1024 * 1024;
