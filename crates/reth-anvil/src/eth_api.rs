use crate::{server::SharedModule, types::ReorgOptions};
use alloy_consensus::TxEnvelope;
use alloy_eips::{BlockId, BlockNumberOrTag, eip7910::EthConfig};
use alloy_primitives::{Address, B256, Bytes, U256};
use alloy_rpc_types::{
    anvil::{Forking, Metadata, MineOptions, NodeInfo},
    debug::ExecutionWitness,
    trace::parity::{TraceResults, TraceType},
    txpool::TxpoolStatus,
};
use alloy_rpc_types_eth::{
    Account, Block, FeeHistory, FillTransaction, Index, Transaction, TransactionReceipt,
    TransactionRequest, state::StateOverride,
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
    pub async fn call<R>(&self, method: &str, params: ArrayParams) -> Result<R>
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
        self.call("eth_blockNumber", ArrayParams::new()).await
    }

    /// Returns the chain id.
    pub async fn chain_id(&self) -> Result<u64> {
        Ok(self.call::<U256>("eth_chainId", ArrayParams::new()).await?.to())
    }

    /// Returns the balance of an account.
    pub async fn balance(&self, address: Address, block: Option<BlockId>) -> Result<U256> {
        self.call("eth_getBalance", params![address, block.unwrap_or_default()]).await
    }

    /// Returns the code of an account.
    pub async fn get_code(&self, address: Address, block: Option<BlockId>) -> Result<Bytes> {
        self.call("eth_getCode", params![address, block.unwrap_or_default()]).await
    }

    /// Returns the account at the given block.
    pub async fn get_account(&self, address: Address, block: Option<BlockId>) -> Result<Account> {
        self.call("eth_getAccount", params![address, block.unwrap_or_default()]).await
    }

    /// Returns the nonce of an account.
    pub async fn transaction_count(
        &self,
        address: Address,
        block: Option<BlockId>,
    ) -> Result<U256> {
        self.call("eth_getTransactionCount", params![address, block.unwrap_or_default()]).await
    }

    /// Returns the receipt of a transaction.
    pub async fn transaction_receipt(&self, hash: B256) -> Result<Option<TransactionReceipt>> {
        self.call("eth_getTransactionReceipt", params![hash]).await
    }

    /// Returns the transaction at the given block and index.
    pub async fn transaction_by_block_number_and_index(
        &self,
        block: BlockNumberOrTag,
        index: Index,
    ) -> Result<Option<Transaction>> {
        self.call("eth_getTransactionByBlockNumberAndIndex", params![block, index]).await
    }

    /// Signs and sends a transaction from a dev or impersonated account.
    pub async fn send_transaction(
        &self,
        request: WithOtherFields<TransactionRequest>,
    ) -> Result<B256> {
        self.call("eth_sendTransaction", params![request]).await
    }

    /// Returns the pool status.
    pub async fn txpool_status(&self) -> Result<TxpoolStatus> {
        self.call("txpool_status", ArrayParams::new()).await
    }

    /// Mines one block.
    pub async fn mine_one(&self) -> Result<()> {
        self.anvil_mine(Some(U256::ONE), None).await
    }

    /// Mines the given number of blocks, with an optional timestamp interval between them.
    pub async fn anvil_mine(&self, blocks: Option<U256>, interval: Option<U256>) -> Result<()> {
        self.call("anvil_mine", params![blocks, interval]).await
    }

    /// Enables or disables automine.
    pub async fn anvil_set_auto_mine(&self, enabled: bool) -> Result<()> {
        self.call("anvil_setAutomine", params![enabled]).await
    }

    /// Impersonates an account.
    pub async fn anvil_impersonate_account(&self, address: Address) -> Result<()> {
        self.call("anvil_impersonateAccount", params![address]).await
    }

    /// Returns the state of the chain as gzipped JSON.
    pub async fn anvil_dump_state(
        &self,
        preserve_historical_states: Option<bool>,
    ) -> Result<Bytes> {
        self.call("anvil_dumpState", params![preserve_historical_states]).await
    }

    /// Applies a state dump on top of the current state.
    pub async fn anvil_load_state(&self, buf: Bytes) -> Result<bool> {
        self.call("anvil_loadState", params![buf]).await
    }

    /// Resets the chain to genesis, or to the fork block when forking.
    pub async fn anvil_reset(&self, forking: Option<Forking>) -> Result<()> {
        self.call("anvil_reset", params![forking]).await
    }

    /// Sets the TIP-20 balance of an account. Only Tempo serves it.
    pub async fn anvil_deal_tip20(
        &self,
        address: Address,
        token_address: Address,
        balance: U256,
    ) -> Result<()> {
        self.call("anvil_dealTIP20", params![address, token_address, balance]).await
    }

    /// Sets the chain id. The node relaunches with its state and height.
    pub async fn anvil_set_chain_id(&self, chain_id: u64) -> Result<()> {
        self.call("anvil_setChainId", params![chain_id]).await
    }

    /// Sets the balance of an account.
    pub async fn anvil_set_balance(&self, address: Address, balance: U256) -> Result<()> {
        self.call("anvil_setBalance", params![address, balance]).await
    }

    /// Sets the nonce of an account.
    pub async fn anvil_set_nonce(&self, address: Address, nonce: U256) -> Result<()> {
        self.call("anvil_setNonce", params![address, nonce]).await
    }

    /// Sets the code of an account.
    pub async fn anvil_set_code(&self, address: Address, code: Bytes) -> Result<()> {
        self.call("anvil_setCode", params![address, code]).await
    }

    /// Sets one storage slot of an account.
    pub async fn anvil_set_storage_at(
        &self,
        address: Address,
        slot: U256,
        value: B256,
    ) -> Result<bool> {
        self.call("anvil_setStorageAt", params![address, slot, value]).await
    }

    /// Removes a transaction from the pool.
    pub async fn anvil_drop_transaction(&self, hash: B256) -> Result<Option<B256>> {
        self.call("anvil_dropTransaction", params![hash]).await
    }

    /// Sets the base fee of the next block.
    pub async fn anvil_set_next_block_base_fee_per_gas(&self, base_fee: U256) -> Result<()> {
        self.call("anvil_setNextBlockBaseFeePerGas", params![base_fee]).await
    }

    /// Sets the prevrandao of the next block.
    pub async fn anvil_set_next_block_prevrandao(&self, prev_randao: B256) -> Result<()> {
        self.call("anvil_setNextBlockPrevRandao", params![prev_randao]).await
    }

    /// Sets the exact timestamp of the next block.
    pub async fn evm_set_next_block_timestamp(&self, seconds: u64) -> Result<()> {
        self.call("anvil_setNextBlockTimestamp", params![seconds]).await
    }

    /// Snapshots the chain. Returns the snapshot id.
    pub async fn evm_snapshot(&self) -> Result<U256> {
        self.call("anvil_snapshot", ArrayParams::new()).await
    }

    /// Reverts the chain to a snapshot.
    pub async fn evm_revert(&self, id: U256) -> Result<bool> {
        self.call("anvil_revert", params![id]).await
    }

    /// Returns the dev accounts.
    pub async fn accounts(&self) -> Result<Vec<Address>> {
        self.call("eth_accounts", ArrayParams::new()).await
    }

    /// Returns the gas price.
    pub async fn gas_price(&self) -> Result<u128> {
        Ok(self.call::<U256>("eth_gasPrice", ArrayParams::new()).await?.to())
    }

    /// Returns the base fee of the next block.
    pub async fn base_fee(&self) -> Result<Option<U256>> {
        let block = self.block_by_number(BlockNumberOrTag::Pending).await?;
        Ok(block.and_then(|block| block.header.base_fee_per_gas).map(U256::from))
    }

    /// Returns the gas limit of the latest block.
    pub async fn gas_limit(&self) -> Result<U256> {
        let block = self.block_by_number(BlockNumberOrTag::Latest).await?;
        Ok(U256::from(block.map(|block| block.header.gas_limit).unwrap_or_default()))
    }

    /// Returns the block with the given number, with transaction hashes.
    pub async fn block_by_number(&self, number: BlockNumberOrTag) -> Result<Option<Block>> {
        self.call("eth_getBlockByNumber", params![number, false]).await
    }

    /// Returns the block with the given number, with full transactions.
    pub async fn block_by_number_full(&self, number: BlockNumberOrTag) -> Result<Option<Block>> {
        self.call("eth_getBlockByNumber", params![number, true]).await
    }

    /// Returns the storage value of an account at a slot.
    pub async fn storage_at(
        &self,
        address: Address,
        index: U256,
        block: Option<BlockId>,
    ) -> Result<B256> {
        self.call("eth_getStorageAt", params![address, index, block.unwrap_or_default()]).await
    }

    /// Returns the fee history.
    pub async fn fee_history(
        &self,
        block_count: U256,
        newest_block: BlockNumberOrTag,
        reward_percentiles: Vec<f64>,
    ) -> Result<FeeHistory> {
        self.call("eth_feeHistory", params![block_count, newest_block, reward_percentiles]).await
    }

    /// Estimates the gas of a call.
    pub async fn estimate_gas(
        &self,
        request: WithOtherFields<TransactionRequest>,
        block: Option<BlockId>,
        state_overrides: Option<StateOverride>,
    ) -> Result<U256> {
        self.call("eth_estimateGas", params![request, block.unwrap_or_default(), state_overrides])
            .await
    }

    /// Sends a signed transaction.
    pub async fn send_raw_transaction(&self, tx: Bytes) -> Result<B256> {
        self.call("eth_sendRawTransaction", params![tx]).await
    }

    /// Sends a signed transaction and waits for its receipt.
    pub async fn send_raw_transaction_sync(
        &self,
        tx: Bytes,
        timeout_ms: Option<u64>,
    ) -> Result<TransactionReceipt> {
        self.call("eth_sendRawTransactionSync", params![tx, timeout_ms]).await
    }

    /// Signs and sends a transaction from a dev account, and waits for its receipt.
    pub async fn send_transaction_sync(
        &self,
        request: WithOtherFields<TransactionRequest>,
    ) -> Result<TransactionReceipt> {
        self.call("eth_sendTransactionSync", params![request]).await
    }

    /// Fills the missing fields of a transaction request.
    pub async fn fill_transaction(
        &self,
        request: WithOtherFields<TransactionRequest>,
    ) -> Result<FillTransaction<TxEnvelope>> {
        self.call("eth_fillTransaction", params![request]).await
    }

    /// Returns the chain id as a decimal string.
    pub async fn network_id(&self) -> Result<Option<String>> {
        self.call("net_version", ArrayParams::new()).await.map(Some)
    }

    /// Returns the bytecode with the given hash.
    pub async fn debug_code_by_hash(
        &self,
        hash: B256,
        block: Option<BlockId>,
    ) -> Result<Option<Bytes>> {
        self.call("debug_codeByHash", params![hash, block]).await
    }

    /// Returns the execution witness of a block.
    pub async fn debug_execution_witness(
        &self,
        block: BlockNumberOrTag,
    ) -> Result<ExecutionWitness> {
        self.call("debug_executionWitness", params![block]).await
    }

    /// Returns the chain configuration.
    pub async fn config(&self) -> Result<EthConfig> {
        self.call("eth_config", ArrayParams::new()).await
    }

    /// Replays a mined transaction and returns its traces.
    pub async fn trace_replay_transaction(
        &self,
        hash: B256,
        trace_types: HashSet<TraceType>,
    ) -> Result<Option<TraceResults>> {
        self.call("trace_replayTransaction", params![hash, trace_types]).await
    }

    /// Mines a block, with optional timestamp and block count.
    pub async fn evm_mine(&self, opts: Option<MineOptions>) -> Result<String> {
        self.call("evm_mine", params![opts]).await
    }

    /// Sets the timestamp of the next block and shifts time by the difference.
    pub async fn evm_set_time(&self, timestamp: u64) -> Result<u64> {
        self.call("evm_setTime", params![timestamp]).await
    }

    /// Moves time forward.
    pub async fn evm_increase_time(&self, seconds: U256) -> Result<i64> {
        self.call("evm_increaseTime", params![seconds]).await
    }

    /// Sets the interval between the timestamps of consecutive blocks.
    pub async fn evm_set_block_timestamp_interval(&self, seconds: u64) -> Result<()> {
        self.call("anvil_setBlockTimestampInterval", params![seconds]).await
    }

    /// Removes the block timestamp interval.
    pub async fn evm_remove_block_timestamp_interval(&self) -> Result<bool> {
        self.call("anvil_removeBlockTimestampInterval", ArrayParams::new()).await
    }

    /// Sets the block gas limit.
    pub async fn evm_set_block_gas_limit(&self, gas_limit: U256) -> Result<bool> {
        self.call("evm_setBlockGasLimit", params![gas_limit]).await
    }

    /// Returns whether automine is on.
    pub async fn anvil_get_auto_mine(&self) -> Result<bool> {
        self.call("anvil_getAutomine", ArrayParams::new()).await
    }

    /// Mines a block every `secs` seconds.
    pub async fn anvil_set_interval_mining(&self, secs: u64) -> Result<()> {
        self.call("evm_setIntervalMining", params![secs]).await
    }

    /// Stops impersonating an account.
    pub async fn anvil_stop_impersonating_account(&self, address: Address) -> Result<()> {
        self.call("anvil_stopImpersonatingAccount", params![address]).await
    }

    /// Impersonates every sender.
    pub async fn anvil_auto_impersonate_account(&self, enabled: bool) -> Result<()> {
        self.call("anvil_autoImpersonateAccount", params![enabled]).await
    }

    /// Returns the node info.
    pub async fn anvil_node_info(&self) -> Result<NodeInfo> {
        self.call("anvil_nodeInfo", ArrayParams::new()).await
    }

    /// Returns the node metadata.
    pub async fn anvil_metadata(&self) -> Result<Metadata> {
        self.call("anvil_metadata", ArrayParams::new()).await
    }

    /// Reorganizes the chain.
    pub async fn anvil_reorg(&self, options: ReorgOptions) -> Result<()> {
        self.call("anvil_reorg", params![options]).await
    }

    /// Rolls the chain back by `depth` blocks.
    pub async fn anvil_rollback(&self, depth: Option<u64>) -> Result<()> {
        self.call("anvil_rollback", params![depth]).await
    }

    /// Sets the minimum gas price before London.
    pub async fn anvil_set_min_gas_price(&self, gas_price: U256) -> Result<()> {
        self.call("anvil_setMinGasPrice", params![gas_price]).await
    }

    /// Sets the parent beacon block root of the next block.
    pub async fn anvil_set_next_block_parent_beacon_block_root(&self, root: B256) -> Result<()> {
        self.call("anvil_setNextBlockParentBeaconBlockRoot", params![root]).await
    }

    /// Removes every pool transaction of a sender.
    pub async fn anvil_remove_pool_transactions(&self, address: Address) -> Result<()> {
        self.call("anvil_removePoolTransactions", params![address]).await
    }

    /// Adds to the balance of an account.
    pub async fn anvil_add_balance(&self, address: Address, balance: U256) -> Result<()> {
        self.call("anvil_addBalance", params![address, balance]).await
    }

    /// Sets the coinbase of the following blocks.
    pub async fn anvil_set_coinbase(&self, address: Address) -> Result<()> {
        self.call("anvil_setCoinbase", params![address]).await
    }

    /// Replaces the fork endpoint.
    pub async fn anvil_set_rpc_url(&self, url: String) -> Result<()> {
        self.call("anvil_setRpcUrl", params![url]).await
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
