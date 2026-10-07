use crate::server::SharedModule;
use alloy_eips::{BlockId, BlockNumberOrTag};
use alloy_primitives::{Address, B256, Bytes, U256};
use alloy_rpc_types::{anvil::Forking, txpool::TxpoolStatus};
use alloy_rpc_types_eth::{Account, Index, Transaction, TransactionReceipt, TransactionRequest};
use alloy_serde::WithOtherFields;
use eyre::Result;
use jsonrpsee::core::params::ArrayParams;
use parking_lot::RwLock;
use serde::de::DeserializeOwned;
use std::sync::Arc;

/// In-process access to the node's RPC handlers.
///
/// Every call runs the same handler the HTTP and WebSocket servers run, without a transport.
#[derive(Clone, Debug)]
pub struct EthApi {
    /// The RPC module of the current node. A relaunch replaces it.
    module: SharedModule,
    /// The instance id, shared with the `anvil_*` namespace, which rotates it on `anvil_reset`.
    instance_id: Arc<RwLock<B256>>,
}

impl EthApi {
    /// Creates the API over the node's RPC module.
    pub(crate) const fn new(module: SharedModule, instance_id: Arc<RwLock<B256>>) -> Self {
        Self { module, instance_id }
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
    pub async fn chain_id(&self) -> Result<U256> {
        self.call("eth_chainId", ArrayParams::new()).await
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
