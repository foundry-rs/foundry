use alloy_eips::BlockId;
use alloy_primitives::{Address, B256, Bytes, U256};
use alloy_rpc_types::txpool::TxpoolStatus;
use alloy_rpc_types_eth::{Account, Index, Transaction, TransactionReceipt, TransactionRequest};
use eyre::Result;
use jsonrpsee::{RpcModule, core::params::ArrayParams};
use serde::de::DeserializeOwned;

/// In-process access to the node's RPC handlers.
///
/// Every call runs the same handler the HTTP and WebSocket servers run, without a transport.
#[derive(Clone, Debug)]
pub struct EthApi {
    module: RpcModule<()>,
    instance_id: B256,
}

impl EthApi {
    /// Creates the API over the node's RPC module.
    pub(crate) const fn new(module: RpcModule<()>, instance_id: B256) -> Self {
        Self { module, instance_id }
    }

    /// Calls an RPC method with positional parameters.
    pub async fn call<R>(&self, method: &str, params: ArrayParams) -> Result<R>
    where
        R: DeserializeOwned + Clone,
    {
        Ok(self.module.call(method, params).await?)
    }

    /// Returns the unique identifier of this node instance.
    pub const fn instance_id(&self) -> B256 {
        self.instance_id
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
        block: u64,
        index: Index,
    ) -> Result<Option<Transaction>> {
        self.call("eth_getTransactionByBlockNumberAndIndex", params![U256::from(block), index])
            .await
    }

    /// Signs and sends a transaction from a dev or impersonated account.
    pub async fn send_transaction(&self, request: TransactionRequest) -> Result<B256> {
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
