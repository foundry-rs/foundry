//! This module extends the Ethereum JSON-RPC provider with the Anvil namespace's RPC methods.

use crate::Provider;
use alloy_network::Network;
use alloy_primitives::{Address, Bytes, TxHash, B256, U128, U256, U64};
use alloy_rpc_types_anvil::{Forking, Metadata, MineOptions, NodeInfo, ReorgOptions};
use alloy_rpc_types_eth::Block;
use alloy_transport::TransportResult;

/// Anvil namespace rpc interface that gives access to several non-standard RPC methods.
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
pub trait AnvilApi<N: Network>: Send + Sync {
    // Not implemented:
    // - anvil_enable_traces: Not implemented in the Anvil RPC API.
    // - anvil_set_block: Not implemented / wired correctly in the Anvil RPC API.

    /// Send transactions impersonating specific account and contract addresses.
    async fn anvil_impersonate_account(&self, address: Address) -> TransportResult<()>;

    /// Stops impersonating an account if previously set with `anvil_impersonateAccount`.
    async fn anvil_stop_impersonating_account(&self, address: Address) -> TransportResult<()>;

    /// If set to true will make every account impersonated.
    async fn anvil_auto_impersonate_account(&self, enabled: bool) -> TransportResult<()>;

    /// Returns true if auto mining is enabled, and false.
    async fn anvil_get_auto_mine(&self) -> TransportResult<bool>;

    /// Enables or disables, based on the single boolean argument, the automatic mining of new
    /// blocks with each new transaction submitted to the network.
    async fn anvil_set_auto_mine(&self, enable_automine: bool) -> TransportResult<()>;

    /// Mines a series of blocks.
    async fn anvil_mine(
        &self,
        num_blocks: Option<u64>,
        interval: Option<u64>,
    ) -> TransportResult<()>;

    /// Sets the mining behavior to interval with the given interval (seconds).
    async fn anvil_set_interval_mining(&self, secs: u64) -> TransportResult<()>;

    /// Removes transactions from the pool.
    async fn anvil_drop_transaction(&self, tx_hash: TxHash) -> TransportResult<Option<TxHash>>;

    /// Removes all transactions from the pool.
    async fn anvil_drop_all_transactions(&self) -> TransportResult<()>;

    /// Reset the fork to a fresh forked state, and optionally update the fork config.
    ///
    /// If `forking` is `None` then this will disable forking entirely.
    async fn anvil_reset(&self, forking: Option<Forking>) -> TransportResult<()>;

    /// Sets the chain ID.
    async fn anvil_set_chain_id(&self, chain_id: u64) -> TransportResult<()>;

    /// Modifies the balance of an account.
    async fn anvil_set_balance(&self, address: Address, balance: U256) -> TransportResult<()>;

    /// Sets the code of a contract.
    async fn anvil_set_code(&self, address: Address, code: Bytes) -> TransportResult<()>;

    /// Sets the nonce of an address.
    async fn anvil_set_nonce(&self, address: Address, nonce: u64) -> TransportResult<()>;

    /// Writes a single slot of the account's storage.
    async fn anvil_set_storage_at(
        &self,
        address: Address,
        slot: U256,
        val: B256,
    ) -> TransportResult<bool>;

    /// Enable or disable logging.
    async fn anvil_set_logging(&self, enable: bool) -> TransportResult<()>;

    /// Set the minimum gas price for the node.
    async fn anvil_set_min_gas_price(&self, gas: u128) -> TransportResult<()>;

    /// Sets the base fee of the next block.
    async fn anvil_set_next_block_base_fee_per_gas(&self, basefee: u128) -> TransportResult<()>;

    /// Sets the coinbase address.
    async fn anvil_set_coinbase(&self, address: Address) -> TransportResult<()>;

    /// Create a buffer that represents all state on the chain, which can be loaded to separate
    /// process by calling `anvil_loadState`
    async fn anvil_dump_state(&self) -> TransportResult<Bytes>;

    /// Append chain state buffer to current chain. Will overwrite any conflicting addresses or
    /// storage.
    async fn anvil_load_state(&self, buf: Bytes) -> TransportResult<bool>;

    /// Retrieves the Anvil node configuration params.
    async fn anvil_node_info(&self) -> TransportResult<NodeInfo>;

    /// Retrieves metadata about the Anvil instance.
    async fn anvil_metadata(&self) -> TransportResult<Metadata>;

    /// Removes all transactions from the pool for a specific address.
    async fn anvil_remove_pool_transactions(&self, address: Address) -> TransportResult<()>;

    /// Snapshot the state of the blockchain at the current block.
    async fn anvil_snapshot(&self) -> TransportResult<U256>;

    /// Revert the state of the blockchain to a previous snapshot.
    /// Takes a single parameter, which is the snapshot id to revert to.
    async fn anvil_revert(&self, id: U256) -> TransportResult<bool>;

    /// Jump forward in time by the given amount of time, in seconds.
    async fn anvil_increase_time(&self, seconds: u64) -> TransportResult<i64>;

    /// Similar to `evm_increaseTime` but takes the exact timestamp that you want in the next block.
    async fn anvil_set_next_block_timestamp(&self, timestamp: u64) -> TransportResult<()>;

    /// Sets the specific timestamp and returns the number of seconds between the given timestamp
    /// and the current time.
    async fn anvil_set_time(&self, timestamp: u64) -> TransportResult<u64>;

    /// Set the next block gas limit.
    async fn anvil_set_block_gas_limit(&self, gas_limit: u64) -> TransportResult<bool>;

    /// Sets an interval for the block timestamp.
    async fn anvil_set_block_timestamp_interval(&self, seconds: u64) -> TransportResult<()>;

    /// Unsets the interval for the block timestamp.
    async fn anvil_remove_block_timestamp_interval(&self) -> TransportResult<bool>;

    /// Mine blocks, instantly.
    /// This will mine the blocks regardless of the configured mining mode.
    async fn evm_mine(&self, opts: Option<MineOptions>) -> TransportResult<String>;

    /// Mine blocks, instantly and return the mined blocks.
    /// This will mine the blocks regardless of the configured mining mode.
    async fn anvil_mine_detailed(&self, opts: Option<MineOptions>) -> TransportResult<Vec<Block>>;

    /// Sets the backend rpc url.
    async fn anvil_set_rpc_url(&self, url: String) -> TransportResult<()>;

    /// Reorg the chain
    async fn anvil_reorg(&self, options: ReorgOptions) -> TransportResult<()>;

    /// Rollback the chain  
    async fn anvil_rollback(&self, depth: Option<u64>) -> TransportResult<()>;

    /// Execute a transaction regardless of signature status.
    async fn eth_send_unsigned_transaction(
        &self,
        request: N::TransactionRequest,
    ) -> TransportResult<TxHash>;
}

#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
impl<N, P> AnvilApi<N> for P
where
    N: Network,
    P: Provider<N>,
{
    async fn anvil_impersonate_account(&self, address: Address) -> TransportResult<()> {
        self.client().request("anvil_impersonateAccount", (address,)).await
    }

    async fn anvil_stop_impersonating_account(&self, address: Address) -> TransportResult<()> {
        self.client().request("anvil_stopImpersonatingAccount", (address,)).await
    }

    async fn anvil_auto_impersonate_account(&self, enabled: bool) -> TransportResult<()> {
        self.client().request("anvil_autoImpersonateAccount", (enabled,)).await
    }

    async fn anvil_get_auto_mine(&self) -> TransportResult<bool> {
        self.client().request_noparams("anvil_getAutomine").await
    }

    async fn anvil_set_auto_mine(&self, enabled: bool) -> TransportResult<()> {
        self.client().request("anvil_setAutomine", (enabled,)).await
    }

    async fn anvil_mine(
        &self,
        num_blocks: Option<u64>,
        interval: Option<u64>,
    ) -> TransportResult<()> {
        self.client()
            .request("anvil_mine", (num_blocks.map(U64::from), interval.map(U64::from)))
            .await
    }

    async fn anvil_set_interval_mining(&self, secs: u64) -> TransportResult<()> {
        self.client().request("anvil_setIntervalMining", (secs,)).await
    }

    async fn anvil_drop_transaction(&self, tx_hash: TxHash) -> TransportResult<Option<TxHash>> {
        self.client().request("anvil_dropTransaction", (tx_hash,)).await
    }

    async fn anvil_drop_all_transactions(&self) -> TransportResult<()> {
        self.client().request_noparams("anvil_dropAllTransactions").await
    }

    async fn anvil_reset(&self, forking: Option<Forking>) -> TransportResult<()> {
        self.client().request("anvil_reset", (forking,)).await
    }

    async fn anvil_set_chain_id(&self, chain_id: u64) -> TransportResult<()> {
        self.client().request("anvil_setChainId", (chain_id,)).await
    }

    async fn anvil_set_balance(&self, address: Address, balance: U256) -> TransportResult<()> {
        self.client().request("anvil_setBalance", (address, balance)).await
    }

    async fn anvil_set_code(&self, address: Address, code: Bytes) -> TransportResult<()> {
        self.client().request("anvil_setCode", (address, code)).await
    }

    async fn anvil_set_nonce(&self, address: Address, nonce: u64) -> TransportResult<()> {
        self.client().request("anvil_setNonce", (address, U64::from(nonce))).await
    }

    async fn anvil_set_storage_at(
        &self,
        address: Address,
        slot: U256,
        val: B256,
    ) -> TransportResult<bool> {
        self.client().request("anvil_setStorageAt", (address, slot, val)).await
    }

    async fn anvil_set_logging(&self, enable: bool) -> TransportResult<()> {
        self.client().request("anvil_setLoggingEnabled", (enable,)).await
    }

    async fn anvil_set_min_gas_price(&self, gas: u128) -> TransportResult<()> {
        self.client().request("anvil_setMinGasPrice", (U128::from(gas),)).await
    }

    async fn anvil_set_next_block_base_fee_per_gas(&self, basefee: u128) -> TransportResult<()> {
        self.client().request("anvil_setNextBlockBaseFeePerGas", (U128::from(basefee),)).await
    }

    async fn anvil_set_coinbase(&self, address: Address) -> TransportResult<()> {
        self.client().request("anvil_setCoinbase", (address,)).await
    }

    async fn anvil_dump_state(&self) -> TransportResult<Bytes> {
        self.client().request_noparams("anvil_dumpState").await
    }

    async fn anvil_load_state(&self, buf: Bytes) -> TransportResult<bool> {
        self.client().request("anvil_loadState", (buf,)).await
    }

    async fn anvil_node_info(&self) -> TransportResult<NodeInfo> {
        self.client().request_noparams("anvil_nodeInfo").await
    }

    async fn anvil_metadata(&self) -> TransportResult<Metadata> {
        self.client().request_noparams("anvil_metadata").await
    }

    async fn anvil_remove_pool_transactions(&self, address: Address) -> TransportResult<()> {
        self.client().request("anvil_removePoolTransactions", (address,)).await
    }

    async fn anvil_snapshot(&self) -> TransportResult<U256> {
        self.client().request_noparams("evm_snapshot").await
    }

    async fn anvil_revert(&self, id: U256) -> TransportResult<bool> {
        self.client().request("evm_revert", (id,)).await
    }

    async fn anvil_increase_time(&self, seconds: u64) -> TransportResult<i64> {
        self.client().request("evm_increaseTime", (U64::from(seconds),)).await
    }

    async fn anvil_set_next_block_timestamp(&self, seconds: u64) -> TransportResult<()> {
        self.client().request("evm_setNextBlockTimestamp", (seconds,)).await
    }

    async fn anvil_set_time(&self, timestamp: u64) -> TransportResult<u64> {
        self.client().request("evm_setTime", (timestamp,)).await
    }

    async fn anvil_set_block_gas_limit(&self, gas_limit: u64) -> TransportResult<bool> {
        self.client().request("evm_setBlockGasLimit", (U64::from(gas_limit),)).await
    }

    async fn anvil_set_block_timestamp_interval(&self, seconds: u64) -> TransportResult<()> {
        self.client().request("anvil_setBlockTimestampInterval", (seconds,)).await
    }

    async fn anvil_remove_block_timestamp_interval(&self) -> TransportResult<bool> {
        self.client().request_noparams("anvil_removeBlockTimestampInterval").await
    }

    async fn evm_mine(&self, opts: Option<MineOptions>) -> TransportResult<String> {
        self.client().request("evm_mine", (opts,)).await
    }

    async fn anvil_mine_detailed(&self, opts: Option<MineOptions>) -> TransportResult<Vec<Block>> {
        self.client().request("evm_mine_detailed", (opts,)).await
    }

    async fn anvil_set_rpc_url(&self, url: String) -> TransportResult<()> {
        self.client().request("anvil_setRpcUrl", (url,)).await
    }

    async fn anvil_reorg(&self, options: ReorgOptions) -> TransportResult<()> {
        self.client().request("anvil_reorg", options).await
    }

    async fn anvil_rollback(&self, depth: Option<u64>) -> TransportResult<()> {
        self.client().request("anvil_rollback", (depth,)).await
    }

    async fn eth_send_unsigned_transaction(
        &self,
        request: N::TransactionRequest,
    ) -> TransportResult<TxHash> {
        self.client().request("eth_sendUnsignedTransaction", (request,)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProviderBuilder;
    use alloy_eips::BlockNumberOrTag;
    use alloy_network::TransactionBuilder;
    use alloy_network_primitives::BlockTransactionsKind;
    use alloy_primitives::B256;
    use alloy_rpc_types_eth::TransactionRequest;

    // use alloy_node_bindings::Anvil; (to be used in `test_anvil_reset`)

    #[tokio::test]
    async fn test_anvil_impersonate_account_stop_impersonating_account() {
        let provider = ProviderBuilder::new().on_anvil();

        let impersonate = Address::random();
        let to = Address::random();
        let val = U256::from(1337);
        let funding = U256::from(1e18 as u64);

        provider.anvil_set_balance(impersonate, funding).await.unwrap();

        let balance = provider.get_balance(impersonate).await.unwrap();
        assert_eq!(balance, funding);

        let tx = TransactionRequest::default().with_from(impersonate).with_to(to).with_value(val);

        let res = provider.send_transaction(tx.clone()).await;
        res.unwrap_err();

        provider.anvil_impersonate_account(impersonate).await.unwrap();
        assert!(provider.get_accounts().await.unwrap().contains(&impersonate));

        let res = provider.send_transaction(tx.clone()).await.unwrap().get_receipt().await.unwrap();
        assert_eq!(res.from, impersonate);

        let nonce = provider.get_transaction_count(impersonate).await.unwrap();
        assert_eq!(nonce, 1);

        let balance = provider.get_balance(to).await.unwrap();
        assert_eq!(balance, val);

        provider.anvil_stop_impersonating_account(impersonate).await.unwrap();
        let res = provider.send_transaction(tx).await;
        res.unwrap_err();
    }

    #[tokio::test]
    async fn test_anvil_auto_impersonate_account() {
        let provider = ProviderBuilder::new().on_anvil();

        let impersonate = Address::random();
        let to = Address::random();
        let val = U256::from(1337);
        let funding = U256::from(1e18 as u64);

        provider.anvil_set_balance(impersonate, funding).await.unwrap();

        let balance = provider.get_balance(impersonate).await.unwrap();
        assert_eq!(balance, funding);

        let tx = TransactionRequest::default().with_from(impersonate).with_to(to).with_value(val);

        let res = provider.send_transaction(tx.clone()).await;
        res.unwrap_err();

        provider.anvil_auto_impersonate_account(true).await.unwrap();

        let res = provider.send_transaction(tx.clone()).await.unwrap().get_receipt().await.unwrap();
        assert_eq!(res.from, impersonate);

        let nonce = provider.get_transaction_count(impersonate).await.unwrap();
        assert_eq!(nonce, 1);

        let balance = provider.get_balance(to).await.unwrap();
        assert_eq!(balance, val);

        provider.anvil_auto_impersonate_account(false).await.unwrap();
        let res = provider.send_transaction(tx).await;
        res.unwrap_err();

        provider.anvil_impersonate_account(impersonate).await.unwrap();
        assert!(provider.get_accounts().await.unwrap().contains(&impersonate));
    }

    #[tokio::test]
    async fn test_anvil_get_auto_mine_set_auto_mine() {
        let provider = ProviderBuilder::new().on_anvil();

        provider.anvil_set_auto_mine(false).await.unwrap();

        let enabled = provider.anvil_get_auto_mine().await.unwrap();
        assert!(!enabled);

        provider.anvil_set_auto_mine(true).await.unwrap();

        let enabled = provider.anvil_get_auto_mine().await.unwrap();
        assert!(enabled);
    }

    #[tokio::test]
    async fn test_anvil_mine() {
        let provider = ProviderBuilder::new().on_anvil();

        let start_num = provider.get_block_number().await.unwrap();

        provider.anvil_mine(Some(10), None).await.unwrap();

        let num = provider.get_block_number().await.unwrap();

        assert_eq!(num, start_num + 10);
    }

    #[tokio::test]
    async fn test_anvil_set_interval_mining() {
        let provider = ProviderBuilder::new().on_anvil();

        provider.anvil_set_interval_mining(1).await.unwrap();

        let start_num = provider.get_block_number().await.unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(1500)).await;

        let num = provider.get_block_number().await.unwrap();

        assert_eq!(num, start_num + 1);
    }

    #[tokio::test]
    async fn test_anvil_drop_transaction() {
        let provider = ProviderBuilder::new().on_anvil_with_wallet();

        provider.anvil_set_auto_mine(false).await.unwrap();

        let alice = provider.get_accounts().await.unwrap()[0];
        let bob = provider.get_accounts().await.unwrap()[1];
        let chain_id = provider.get_chain_id().await.unwrap();

        let tx = TransactionRequest::default()
            .with_from(alice)
            .with_to(bob)
            .with_nonce(0)
            .with_chain_id(chain_id)
            .with_value(U256::from(100))
            .with_gas_limit(21_000)
            .with_max_priority_fee_per_gas(1_000_000_000)
            .with_max_fee_per_gas(20_000_000_000);

        let tx_hash =
            provider.send_transaction(tx).await.unwrap().register().await.unwrap().tx_hash;

        let res = provider.anvil_drop_transaction(tx_hash).await.unwrap();

        assert_eq!(res, Some(tx_hash));
    }

    #[tokio::test]
    async fn test_anvil_drop_all_transactions() {
        let provider = ProviderBuilder::new().on_anvil_with_wallet();

        provider.anvil_set_auto_mine(false).await.unwrap();

        let alice = provider.get_accounts().await.unwrap()[0];
        let bob = provider.get_accounts().await.unwrap()[1];
        let chain_id = provider.get_chain_id().await.unwrap();

        let tx = TransactionRequest::default()
            .with_from(alice)
            .with_to(bob)
            .with_nonce(0)
            .with_chain_id(chain_id)
            .with_value(U256::from(100))
            .with_gas_limit(21_000)
            .with_max_priority_fee_per_gas(1_000_000_000)
            .with_max_fee_per_gas(20_000_000_000);

        provider.send_transaction(tx.clone()).await.unwrap().register().await.unwrap();

        let tx = tx.clone().with_nonce(1);

        provider.send_transaction(tx).await.unwrap().register().await.unwrap();

        provider.anvil_drop_all_transactions().await.unwrap();
    }

    // TODO: Fix this test, `chain_id` is not being set correctly.
    // #[tokio::test]
    // async fn test_anvil_reset() {
    //     let fork1 = Anvil::default().chain_id(777).spawn();
    //     let fork2 = Anvil::default().chain_id(888).spawn();

    //     let provider = ProviderBuilder::new()
    //         .on_anvil_with_config(|config| config.fork(fork1.endpoint_url().to_string()));

    //     let chain_id = provider.get_chain_id().await.unwrap();
    //     assert_eq!(chain_id, 777);

    //     provider
    //         .anvil_reset(Some(Forking {
    //             json_rpc_url: Some(fork2.endpoint_url().to_string()),
    //             block_number: Some(0),
    //         }))
    //         .await
    //         .unwrap();

    //     let chain_id = provider.get_chain_id().await.unwrap();
    //     assert_eq!(chain_id, 888);
    // }

    #[tokio::test]
    async fn test_anvil_set_chain_id() {
        let provider = ProviderBuilder::new().on_anvil();

        let chain_id = 1337;
        provider.anvil_set_chain_id(chain_id).await.unwrap();

        let new_chain_id = provider.get_chain_id().await.unwrap();
        assert_eq!(new_chain_id, chain_id);
    }

    #[tokio::test]
    async fn test_anvil_set_balance() {
        let provider = ProviderBuilder::new().on_anvil();

        let address = Address::random();
        let balance = U256::from(1337);
        provider.anvil_set_balance(address, balance).await.unwrap();

        let new_balance = provider.get_balance(address).await.unwrap();
        assert_eq!(new_balance, balance);
    }

    #[tokio::test]
    async fn test_anvil_set_code() {
        let provider = ProviderBuilder::new().on_anvil();

        let address = Address::random();
        provider.anvil_set_code(address, Bytes::from("0xbeef")).await.unwrap();

        let code = provider.get_code_at(address).await.unwrap();
        assert_eq!(code, Bytes::from("0xbeef"));
    }

    #[tokio::test]
    async fn test_anvil_set_nonce() {
        let provider = ProviderBuilder::new().on_anvil();

        let address = Address::random();
        let nonce = 1337;
        provider.anvil_set_nonce(address, nonce).await.unwrap();

        let new_nonce = provider.get_transaction_count(address).await.unwrap();
        assert_eq!(new_nonce, nonce);
    }

    #[tokio::test]
    async fn test_anvil_set_storage_at() {
        let provider = ProviderBuilder::new().on_anvil();

        let address = Address::random();
        let slot = U256::from(1337);
        let val = B256::from(U256::from(1337));
        provider.anvil_set_storage_at(address, slot, val).await.unwrap();

        let storage = provider.get_storage_at(address, slot).await.unwrap();
        assert_eq!(B256::from(storage), val);
    }

    #[tokio::test]
    async fn test_anvil_set_logging() {
        let provider = ProviderBuilder::new().on_anvil();

        provider.anvil_set_logging(true).await.unwrap();
    }

    #[tokio::test]
    async fn test_anvil_set_min_gas_price() {
        let provider = ProviderBuilder::new().on_anvil();

        let gas = U256::from(1337);

        if let Err(e) = provider.anvil_set_min_gas_price(gas.try_into().unwrap()).await {
            assert_eq!(
                e.to_string(),
                "server returned an error response: error code -32602: anvil_setMinGasPrice is not supported when EIP-1559 is active"
            );
        }
    }

    #[tokio::test]
    async fn test_anvil_set_next_block_base_fee_per_gas() {
        let provider = ProviderBuilder::new().on_anvil();

        let basefee = 1337;
        provider.anvil_set_next_block_base_fee_per_gas(basefee).await.unwrap();

        provider.evm_mine(None).await.unwrap();

        let block = provider
            .get_block_by_number(BlockNumberOrTag::Latest, BlockTransactionsKind::Hashes)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(block.header.base_fee_per_gas, Some(basefee as u64));
    }

    #[tokio::test]
    async fn test_anvil_set_coinbase() {
        let provider = ProviderBuilder::new().on_anvil();

        let coinbase = Address::random();
        provider.anvil_set_coinbase(coinbase).await.unwrap();

        provider.evm_mine(None).await.unwrap();

        let block = provider
            .get_block_by_number(BlockNumberOrTag::Latest, BlockTransactionsKind::Hashes)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(block.header.beneficiary, coinbase);
    }

    #[tokio::test]
    async fn test_anvil_dump_state_load_state() {
        let provider = ProviderBuilder::new().on_anvil();

        let state = provider.anvil_dump_state().await.unwrap();

        assert!(!state.is_empty());

        let res = provider.anvil_load_state(state).await.unwrap();

        assert!(res);
    }

    #[tokio::test]
    async fn test_anvil_node_info() {
        let provider = ProviderBuilder::new().on_anvil();

        let latest_block = provider
            .get_block_by_number(BlockNumberOrTag::Latest, BlockTransactionsKind::Hashes)
            .await
            .unwrap()
            .unwrap();

        provider.evm_mine(None).await.unwrap();

        let node_info = provider.anvil_node_info().await.unwrap();

        assert_eq!(node_info.current_block_number, latest_block.header.number + 1);
    }

    #[tokio::test]
    async fn test_anvil_metadata() {
        let provider = ProviderBuilder::new().on_anvil();

        let client_version = provider.get_client_version().await.unwrap();
        let chain_id = provider.get_chain_id().await.unwrap();

        let metadata = provider.anvil_metadata().await.unwrap();

        assert_eq!(metadata.client_version, client_version);
        assert_eq!(metadata.chain_id, chain_id);
    }

    #[tokio::test]
    async fn test_anvil_remove_pool_transactions() {
        let provider = ProviderBuilder::new().on_anvil_with_wallet();

        provider.anvil_set_auto_mine(false).await.unwrap();

        let alice = provider.get_accounts().await.unwrap()[0];
        let bob = provider.get_accounts().await.unwrap()[1];
        let chain_id = provider.get_chain_id().await.unwrap();

        let tx = TransactionRequest::default()
            .with_from(alice)
            .with_to(bob)
            .with_nonce(0)
            .with_chain_id(chain_id)
            .with_value(U256::from(100))
            .with_gas_limit(21_000)
            .with_max_priority_fee_per_gas(1_000_000_000)
            .with_max_fee_per_gas(20_000_000_000);

        provider.send_transaction(tx.clone()).await.unwrap().register().await.unwrap();

        let tx = tx.clone().with_nonce(1);

        provider.send_transaction(tx).await.unwrap().register().await.unwrap();

        provider.anvil_remove_pool_transactions(alice).await.unwrap();
    }

    #[tokio::test]
    async fn test_anvil_snapshot_revert() {
        let provider = ProviderBuilder::new().on_anvil();

        let snapshot_id = provider.anvil_snapshot().await.unwrap();

        let alice = provider.get_accounts().await.unwrap()[0];
        let bob = provider.get_accounts().await.unwrap()[1];
        let chain_id = provider.get_chain_id().await.unwrap();

        let tx = TransactionRequest::default()
            .with_from(alice)
            .with_to(bob)
            .with_nonce(0)
            .with_chain_id(chain_id)
            .with_value(U256::from(100))
            .with_gas_limit(21_000)
            .with_max_priority_fee_per_gas(1_000_000_000)
            .with_max_fee_per_gas(20_000_000_000);

        provider.send_transaction(tx.clone()).await.unwrap().get_receipt().await.unwrap();

        let tx = tx.clone().with_nonce(1);

        provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

        let tx_count = provider.get_transaction_count(alice).await.unwrap();
        assert_eq!(tx_count, 2);

        let res = provider.anvil_revert(snapshot_id).await.unwrap();
        assert!(res);

        let tx_count = provider.get_transaction_count(alice).await.unwrap();
        assert_eq!(tx_count, 0);
    }

    #[tokio::test]
    async fn test_anvil_increase_time() {
        let provider = ProviderBuilder::new().on_anvil();

        let timestamp = provider
            .get_block_by_number(BlockNumberOrTag::Latest, BlockTransactionsKind::Hashes)
            .await
            .unwrap()
            .unwrap()
            .header
            .timestamp;

        let seconds = provider.anvil_increase_time(1337).await.unwrap();

        assert_eq!(timestamp as i64 + seconds, timestamp as i64 + 1337_i64);
    }

    #[tokio::test]
    async fn test_anvil_set_next_block_timestamp() {
        let provider = ProviderBuilder::new().on_anvil();

        let timestamp = provider
            .get_block_by_number(BlockNumberOrTag::Latest, BlockTransactionsKind::Hashes)
            .await
            .unwrap()
            .unwrap()
            .header
            .timestamp;

        provider.anvil_set_next_block_timestamp(timestamp + 1337).await.unwrap();

        provider.evm_mine(None).await.unwrap();

        let latest_block = provider
            .get_block_by_number(BlockNumberOrTag::Latest, BlockTransactionsKind::Hashes)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(latest_block.header.timestamp, timestamp + 1337);
    }

    #[tokio::test]
    async fn test_anvil_set_time() {
        let provider = ProviderBuilder::new().on_anvil();

        provider.anvil_set_time(0).await.unwrap();

        let seconds = provider.anvil_set_time(1001).await.unwrap();

        assert_eq!(seconds, 1);
    }

    #[tokio::test]
    async fn test_anvil_set_block_gas_limit() {
        let provider = ProviderBuilder::new().on_anvil();

        let block_gas_limit = 1337;
        assert!(provider.anvil_set_block_gas_limit(block_gas_limit).await.unwrap());

        provider.evm_mine(None).await.unwrap();

        let latest_block = provider
            .get_block_by_number(BlockNumberOrTag::Latest, BlockTransactionsKind::Hashes)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(block_gas_limit, latest_block.header.gas_limit);
    }

    #[tokio::test]
    async fn test_anvil_block_timestamp_interval() {
        let provider = ProviderBuilder::new().on_anvil();

        provider.anvil_set_block_timestamp_interval(1).await.unwrap();

        let start_timestamp = provider
            .get_block_by_number(BlockNumberOrTag::Latest, BlockTransactionsKind::Hashes)
            .await
            .unwrap()
            .unwrap()
            .header
            .timestamp;

        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

        provider.evm_mine(None).await.unwrap();

        let latest_block = provider
            .get_block_by_number(BlockNumberOrTag::Latest, BlockTransactionsKind::Hashes)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(latest_block.header.timestamp, start_timestamp + 1);

        provider.anvil_remove_block_timestamp_interval().await.unwrap();

        provider.evm_mine(None).await.unwrap();

        let start_timestamp = provider
            .get_block_by_number(BlockNumberOrTag::Latest, BlockTransactionsKind::Hashes)
            .await
            .unwrap()
            .unwrap()
            .header
            .timestamp;

        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

        let latest_block = provider
            .get_block_by_number(BlockNumberOrTag::Latest, BlockTransactionsKind::Hashes)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(latest_block.header.timestamp, start_timestamp);
    }

    #[tokio::test]
    async fn test_evm_mine_single_block() {
        let provider = ProviderBuilder::new().on_anvil();

        let start_num = provider.get_block_number().await.unwrap();

        for (idx, _) in std::iter::repeat(()).take(10).enumerate() {
            provider.evm_mine(None).await.unwrap();
            let num = provider.get_block_number().await.unwrap();
            assert_eq!(num, start_num + idx as u64 + 1);
        }

        let num = provider.get_block_number().await.unwrap();
        assert_eq!(num, start_num + 10);
    }

    // TODO: Fix this test, only a single block is being mined regardless of the `blocks` parameter.
    // #[tokio::test]
    // async fn test_evm_mine_with_configuration() {
    //     let provider = ProviderBuilder::new().on_anvil();

    //     let start_num = provider.get_block_number().await.unwrap();

    //     provider
    //         .evm_mine(Some(MineOptions::Options { timestamp: Some(100), blocks: Some(10) }))
    //         .await
    //         .unwrap();

    //     let num = provider.get_block_number().await.unwrap();
    //     assert_eq!(num, start_num + 10);
    // }

    #[tokio::test]
    async fn test_anvil_mine_detailed_single_block() {
        let provider = ProviderBuilder::new().on_anvil();

        let start_num = provider.get_block_number().await.unwrap();

        for (idx, _) in std::iter::repeat(()).take(10).enumerate() {
            provider.anvil_mine_detailed(None).await.unwrap();
            let num = provider.get_block_number().await.unwrap();
            assert_eq!(num, start_num + idx as u64 + 1);
        }

        let num = provider.get_block_number().await.unwrap();
        assert_eq!(num, start_num + 10);
    }

    // TODO: Fix this test, only a single block is being mined regardless of the `blocks` parameter.
    // #[tokio::test]
    // async fn test_anvil_mine_detailed_with_configuration() {
    //     let provider = ProviderBuilder::new().on_anvil();

    //     let start_num = provider.get_block_number().await.unwrap();

    //     let blocks = provider
    //         .anvil_mine_detailed(Some(MineOptions::Options {
    //             timestamp: Some(100),
    //             blocks: Some(10),
    //         }))
    //         .await
    //         .unwrap();

    //     let num = provider.get_block_number().await.unwrap();
    //     assert_eq!(num, start_num + 10);

    //     for (idx, block) in blocks.iter().enumerate() {
    //         assert_eq!(block.header.number, Some(start_num + idx as u64 + 1));
    //     }
    // }

    #[tokio::test]
    async fn test_anvil_set_rpc_url() {
        let provider = ProviderBuilder::new().on_anvil();

        let url = "https://example.com".to_string();
        provider.anvil_set_rpc_url(url.clone()).await.unwrap();
    }

    #[tokio::test]
    async fn test_anvil_reorg() {
        let provider = ProviderBuilder::new().on_anvil();

        // Mine two blocks
        provider.anvil_mine(Some(2), None).await.unwrap();

        let reorged_block = provider
            .get_block_by_number(2.into(), BlockTransactionsKind::Hashes)
            .await
            .unwrap()
            .unwrap();
        provider.anvil_reorg(ReorgOptions { depth: 1, tx_block_pairs: Vec::new() }).await.unwrap();

        let new_block = provider
            .get_block_by_number(2.into(), BlockTransactionsKind::Hashes)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(reorged_block.header.number, new_block.header.number);
        assert_ne!(reorged_block.header.hash, new_block.header.hash);
    }

    #[tokio::test]
    #[ignore]
    async fn test_anvil_rollback() {
        let provider = ProviderBuilder::new().on_anvil();

        // Mine two blocks
        provider.anvil_mine(Some(2), None).await.unwrap();

        let target_height = provider
            .get_block_by_number(1.into(), BlockTransactionsKind::Hashes)
            .await
            .unwrap()
            .unwrap();

        provider.anvil_rollback(Some(1)).await.unwrap();

        let new_head = provider
            .get_block_by_number(BlockNumberOrTag::Latest, BlockTransactionsKind::Hashes)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(target_height, new_head);
    }

    #[tokio::test]
    async fn test_eth_send_unsigned_transaction() {
        let provider = ProviderBuilder::new().on_anvil();

        let alice = Address::random();
        let bob = Address::random();
        let chain_id = provider.get_chain_id().await.unwrap();

        provider.anvil_set_balance(alice, U256::from(1e18 as u64)).await.unwrap();

        let tx = TransactionRequest::default()
            .with_from(alice)
            .with_to(bob)
            .with_nonce(0)
            .with_chain_id(chain_id)
            .with_value(U256::from(100))
            .with_gas_limit(21_000)
            .with_max_priority_fee_per_gas(1_000_000_000)
            .with_max_fee_per_gas(20_000_000_000);

        let tx_hash = provider.eth_send_unsigned_transaction(tx).await.unwrap();

        provider.evm_mine(None).await.unwrap();

        let res = provider.get_transaction_receipt(tx_hash).await.unwrap().unwrap();
        assert_eq!(res.from, alice);
        assert_eq!(res.to, Some(bob));
    }
}
