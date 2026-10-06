use crate::{
    block_env::BlockEnvOverrides,
    impersonation::ImpersonationState,
    mining::MiningController,
    snapshot::{Snapshot, SnapshotManager},
    state::SharedAnvilState,
    time::TimeManager,
};
use alloy_consensus::{BlockHeader, transaction::TxHashRef};
use alloy_eips::BlockNumberOrTag;
use alloy_network::Ethereum;
use alloy_primitives::{Address, B256, Bytes, U256};
use alloy_rpc_types::anvil::{Metadata, MineOptions, NodeEnvironment, NodeForkConfig, NodeInfo};
use alloy_rpc_types_eth::Block;
use jsonrpsee::{
    core::{RpcResult, async_trait},
    proc_macros::rpc,
    types::{
        ErrorObjectOwned,
        error::{INTERNAL_ERROR_CODE, INVALID_PARAMS_CODE},
    },
};
use reth_ethereum::{
    chainspec::{ChainSpec, EthChainSpec},
    pool::TransactionPool,
    primitives::{Bytecode, SealedHeader},
    storage::{BlockNumReader, HeaderProvider, StateProviderFactory, TransactionsProvider},
};
use reth_rpc_eth_api::{EthApiServer, FullEthApiServer};
use std::sync::Arc;

/// The `anvil_*` RPC namespace, with the `hardhat_*` and `evm_*` aliases that anvil accepts.
#[rpc(server, namespace = "anvil")]
pub trait AnvilApi {
    /// Impersonates the given account for `eth_sendTransaction`.
    #[method(name = "impersonateAccount", aliases = ["hardhat_impersonateAccount"])]
    async fn anvil_impersonate_account(&self, address: Address) -> RpcResult<()>;

    /// Stops impersonating the given account.
    #[method(name = "stopImpersonatingAccount", aliases = ["hardhat_stopImpersonatingAccount"])]
    async fn anvil_stop_impersonating_account(&self, address: Address) -> RpcResult<()>;

    /// Enables or disables impersonation of every account.
    #[method(name = "autoImpersonateAccount", aliases = ["hardhat_autoImpersonateAccount"])]
    async fn anvil_auto_impersonate_account(&self, enabled: bool) -> RpcResult<()>;

    /// Returns whether automine is enabled.
    #[method(name = "getAutomine", aliases = ["hardhat_getAutomine"])]
    async fn anvil_get_automine(&self) -> RpcResult<bool>;

    /// Returns the interval mining period in seconds, if enabled.
    #[method(name = "getIntervalMining")]
    async fn anvil_get_interval_mining(&self) -> RpcResult<Option<u64>>;

    /// Enables or disables automine.
    #[method(name = "setAutomine", aliases = ["evm_setAutomine"])]
    async fn anvil_set_automine(&self, enabled: bool) -> RpcResult<()>;

    /// Sets the interval mining period in seconds. Zero disables interval mining.
    #[method(name = "setIntervalMining", aliases = ["evm_setIntervalMining"])]
    async fn anvil_set_interval_mining(&self, interval: u64) -> RpcResult<()>;

    /// Mines the given number of blocks, with an optional timestamp interval between them.
    #[method(name = "mine", aliases = ["hardhat_mine"])]
    async fn anvil_mine(&self, num_blocks: Option<U256>, interval: Option<U256>) -> RpcResult<()>;

    /// Mines blocks and returns them with full transactions.
    #[method(name = "mine_detailed", aliases = ["evm_mine_detailed"])]
    async fn anvil_mine_detailed(&self, opts: Option<MineOptions>) -> RpcResult<Vec<Block>>;

    /// Snapshots the chain head and the anvil settings. Returns the snapshot id.
    #[method(name = "snapshot", aliases = ["evm_snapshot"])]
    async fn anvil_snapshot(&self) -> RpcResult<U256>;

    /// Reverts the chain to the given snapshot. Returns whether the snapshot existed.
    #[method(name = "revert", aliases = ["evm_revert"])]
    async fn anvil_revert(&self, id: U256) -> RpcResult<bool>;

    /// Rewinds the chain by the given number of blocks.
    #[method(name = "rollback")]
    async fn anvil_rollback(&self, depth: Option<u64>) -> RpcResult<()>;

    /// Removes a transaction from the pool.
    #[method(name = "dropTransaction", aliases = ["hardhat_dropTransaction"])]
    async fn anvil_drop_transaction(&self, tx_hash: B256) -> RpcResult<Option<B256>>;

    /// Removes all transactions from the pool.
    #[method(name = "dropAllTransactions", aliases = ["hardhat_dropAllTransactions"])]
    async fn anvil_drop_all_transactions(&self) -> RpcResult<()>;

    /// Removes all transactions sent by the given address from the pool.
    #[method(name = "removePoolTransactions")]
    async fn anvil_remove_pool_transactions(&self, address: Address) -> RpcResult<()>;

    /// Returns the genesis block timestamp.
    #[method(name = "getGenesisTime")]
    async fn anvil_get_genesis_time(&self) -> RpcResult<u64>;

    /// Returns node configuration and the current head.
    #[method(name = "nodeInfo")]
    async fn anvil_node_info(&self) -> RpcResult<NodeInfo>;

    /// Returns client metadata.
    #[method(name = "metadata", aliases = ["hardhat_metadata"])]
    async fn anvil_metadata(&self) -> RpcResult<Metadata>;

    /// Sets the timestamp interval between mined blocks.
    #[method(name = "setBlockTimestampInterval")]
    async fn anvil_set_block_timestamp_interval(&self, seconds: u64) -> RpcResult<()>;

    /// Removes the block timestamp interval. Returns whether one was set.
    #[method(name = "removeBlockTimestampInterval")]
    async fn anvil_remove_block_timestamp_interval(&self) -> RpcResult<bool>;

    /// Moves the clock forward by the given number of seconds.
    #[method(name = "increaseTime", aliases = ["evm_increaseTime"])]
    async fn anvil_increase_time(&self, seconds: U256) -> RpcResult<i64>;

    /// Sets the clock to the given timestamp. Returns the applied offset in seconds.
    #[method(name = "setTime", aliases = ["evm_setTime"])]
    async fn anvil_set_time(&self, timestamp: u64) -> RpcResult<u64>;

    /// Sets the exact timestamp of the next block.
    #[method(name = "setNextBlockTimestamp", aliases = ["evm_setNextBlockTimestamp"])]
    async fn anvil_set_next_block_timestamp(&self, seconds: u64) -> RpcResult<()>;

    /// Sets the gas limit of all future blocks.
    #[method(name = "setBlockGasLimit", aliases = ["evm_setBlockGasLimit"])]
    async fn anvil_set_block_gas_limit(&self, gas_limit: U256) -> RpcResult<bool>;

    /// Sets the coinbase of all future blocks.
    #[method(name = "setCoinbase", aliases = ["hardhat_setCoinbase"])]
    async fn anvil_set_coinbase(&self, address: Address) -> RpcResult<()>;

    /// Sets the base fee of the next block.
    #[method(name = "setNextBlockBaseFeePerGas", aliases = ["hardhat_setNextBlockBaseFeePerGas"])]
    async fn anvil_set_next_block_base_fee_per_gas(&self, base_fee: U256) -> RpcResult<()>;

    /// Sets the balance of an account.
    #[method(name = "setBalance", aliases = ["hardhat_setBalance"])]
    async fn anvil_set_balance(&self, address: Address, balance: U256) -> RpcResult<()>;

    /// Adds to the balance of an account.
    #[method(name = "addBalance", aliases = ["hardhat_addBalance"])]
    async fn anvil_add_balance(&self, address: Address, balance: U256) -> RpcResult<()>;

    /// Sets the nonce of an account.
    #[method(name = "setNonce", aliases = ["hardhat_setNonce", "evm_setAccountNonce"])]
    async fn anvil_set_nonce(&self, address: Address, nonce: U256) -> RpcResult<()>;

    /// Sets the code of an account.
    #[method(name = "setCode", aliases = ["hardhat_setCode"])]
    async fn anvil_set_code(&self, address: Address, code: Bytes) -> RpcResult<()>;

    /// Sets one storage slot of an account.
    #[method(name = "setStorageAt", aliases = ["hardhat_setStorageAt"])]
    async fn anvil_set_storage_at(
        &self,
        address: Address,
        slot: U256,
        value: B256,
    ) -> RpcResult<bool>;
}

/// The `evm_*` methods that have no `anvil_*` counterpart.
#[rpc(server, namespace = "evm")]
pub trait EvmApi {
    /// Mines blocks and returns `"0x0"`, as Hardhat does.
    #[method(name = "mine")]
    async fn evm_mine(&self, opts: Option<MineOptions>) -> RpcResult<String>;
}

/// Implementation of the `anvil_*` RPC namespace.
#[derive(Debug, Clone)]
pub struct AnvilRpc<Pool, Provider, Eth> {
    impersonation: ImpersonationState,
    mining: MiningController,
    time: TimeManager,
    block_env: BlockEnvOverrides,
    state: SharedAnvilState,
    snapshots: SnapshotManager,
    chain_spec: Arc<ChainSpec>,
    instance_id: B256,
    pool: Pool,
    provider: Provider,
    eth: Eth,
}

impl<Pool, Provider, Eth> AnvilRpc<Pool, Provider, Eth> {
    /// Creates the `anvil_*` namespace over the given node components.
    #[expect(clippy::too_many_arguments)]
    pub const fn new(
        impersonation: ImpersonationState,
        mining: MiningController,
        time: TimeManager,
        block_env: BlockEnvOverrides,
        state: SharedAnvilState,
        snapshots: SnapshotManager,
        chain_spec: Arc<ChainSpec>,
        instance_id: B256,
        pool: Pool,
        provider: Provider,
        eth: Eth,
    ) -> Self {
        Self {
            impersonation,
            mining,
            time,
            block_env,
            state,
            snapshots,
            chain_spec,
            instance_id,
            pool,
            provider,
            eth,
        }
    }
}

impl<Pool, Provider, Eth> AnvilRpc<Pool, Provider, Eth>
where
    Provider: BlockNumReader
        + HeaderProvider<Header = alloy_consensus::Header>
        + TransactionsProvider
        + StateProviderFactory,
    Eth: FullEthApiServer<NetworkTypes = Ethereum>,
{
    fn best_block_number(&self) -> RpcResult<u64> {
        self.provider
            .best_block_number()
            .map_err(|error| internal_error(format!("failed to read latest block number: {error}")))
    }

    fn sealed_header(&self, number: u64) -> RpcResult<SealedHeader> {
        self.provider
            .sealed_header(number)
            .map_err(|error| internal_error(format!("failed to read header {number}: {error}")))?
            .ok_or_else(|| internal_error(format!("missing block header {number}")))
    }

    async fn block_by_number(&self, number: u64, full: bool) -> RpcResult<Block> {
        EthApiServer::block_by_number(&self.eth, BlockNumberOrTag::Number(number), full)
            .await?
            .ok_or_else(|| internal_error(format!("missing block {number}")))
    }

    async fn latest_block(&self) -> RpcResult<Block> {
        self.block_by_number(self.best_block_number()?, false).await
    }

    /// Mines `blocks` blocks and returns their numbers.
    async fn mine_blocks(&self, blocks: u64) -> RpcResult<Vec<u64>> {
        let mut mined = Vec::with_capacity(blocks as usize);
        for _ in 0..blocks {
            mined.push(self.mining.mine_block().await.map_err(internal_error)?.number);
        }
        Ok(mined)
    }

    /// Rewinds the chain to the given canonical header and drops the transactions of the removed
    /// blocks, so the pool does not mine them again.
    async fn rewind_to(&self, header: &SealedHeader) -> RpcResult<()> {
        let best = self.best_block_number()?;
        if header.number < best {
            let removed = self
                .provider
                .transactions_by_block_range(header.number + 1..=best)
                .map_err(|error| internal_error(format!("failed to read transactions: {error}")))?;
            self.impersonation.drop_txs(removed.into_iter().flatten().map(|tx| *tx.tx_hash()));
        }
        self.mining.rewind(header.clone()).await.map_err(internal_error)
    }

    /// Returns the balance of the account in the latest state, including anvil state writes.
    fn latest_balance(&self, address: Address) -> RpcResult<U256> {
        Ok(self
            .provider
            .latest()
            .and_then(|state| state.basic_account(&address))
            .map_err(|error| internal_error(format!("failed to read account: {error}")))?
            .unwrap_or_default()
            .balance)
    }

    /// Returns the lowercase name of the latest hardfork active at the given block.
    fn hardfork_name(&self, timestamp: u64, number: u64) -> String {
        self.chain_spec
            .hardforks
            .forks_iter()
            .filter(|(_, condition)| condition.active_at_timestamp_or_number(timestamp, number))
            .last()
            .map(|(fork, _)| fork.name().to_lowercase())
            .unwrap_or_default()
    }
}

#[async_trait]
impl<Pool, Provider, Eth> AnvilApiServer for AnvilRpc<Pool, Provider, Eth>
where
    Pool: TransactionPool + Send + Sync + 'static,
    Provider: BlockNumReader
        + HeaderProvider<Header = alloy_consensus::Header>
        + TransactionsProvider
        + StateProviderFactory
        + Send
        + Sync
        + 'static,
    Eth: FullEthApiServer<NetworkTypes = Ethereum>,
{
    async fn anvil_impersonate_account(&self, address: Address) -> RpcResult<()> {
        self.impersonation.impersonate(address);
        Ok(())
    }

    async fn anvil_stop_impersonating_account(&self, address: Address) -> RpcResult<()> {
        self.impersonation.stop_impersonating(address);
        Ok(())
    }

    async fn anvil_auto_impersonate_account(&self, enabled: bool) -> RpcResult<()> {
        self.impersonation.set_auto_impersonate(enabled);
        Ok(())
    }

    async fn anvil_get_automine(&self) -> RpcResult<bool> {
        Ok(self.mining.is_automine())
    }

    async fn anvil_get_interval_mining(&self) -> RpcResult<Option<u64>> {
        Ok(self.mining.interval_mining())
    }

    async fn anvil_set_automine(&self, enabled: bool) -> RpcResult<()> {
        self.mining.set_automine(enabled);
        if enabled && self.pool.pending_and_queued_txn_count().0 > 0 {
            self.mining.trigger();
        }
        Ok(())
    }

    async fn anvil_set_interval_mining(&self, interval: u64) -> RpcResult<()> {
        self.mining.set_interval_mining(interval);
        Ok(())
    }

    async fn anvil_mine(&self, num_blocks: Option<U256>, interval: Option<U256>) -> RpcResult<()> {
        let blocks = num_blocks.unwrap_or(U256::ONE).to::<u64>();

        let Some(interval) = interval.filter(|interval| !interval.is_zero()) else {
            self.mine_blocks(blocks).await?;
            return Ok(());
        };

        let previous_interval = self.time.interval();
        self.time.set_block_timestamp_interval(interval.to::<u64>());
        let result = self.mine_blocks(blocks).await;
        match previous_interval {
            Some(previous_interval) => self.time.set_block_timestamp_interval(previous_interval),
            None => {
                self.time.remove_block_timestamp_interval();
            }
        }
        result?;
        Ok(())
    }

    async fn anvil_mine_detailed(&self, opts: Option<MineOptions>) -> RpcResult<Vec<Block>> {
        let (timestamp, blocks) = match opts.unwrap_or_default() {
            MineOptions::Options { timestamp, blocks } => (timestamp, blocks.unwrap_or(1)),
            MineOptions::Timestamp(timestamp) => (timestamp, 1),
        };

        if let Some(timestamp) = timestamp {
            self.time.set_next_block_timestamp(timestamp).map_err(invalid_params)?;
        }

        let mut mined = Vec::with_capacity(blocks as usize);
        for number in self.mine_blocks(blocks).await? {
            mined.push(self.block_by_number(number, true).await?);
        }
        Ok(mined)
    }

    async fn anvil_snapshot(&self) -> RpcResult<U256> {
        let header = self.sealed_header(self.best_block_number()?)?;
        let snapshot = Snapshot {
            header,
            state: self.state.read().clone(),
            time: self.time.snapshot(),
            block_env: self.block_env.snapshot(),
        };
        Ok(self.snapshots.insert(snapshot))
    }

    async fn anvil_revert(&self, id: U256) -> RpcResult<bool> {
        let Some(snapshot) = self.snapshots.take(id) else {
            return Ok(false);
        };
        self.rewind_to(&snapshot.header).await?;
        *self.state.write() = snapshot.state;
        self.time.restore(snapshot.time);
        self.block_env.restore(snapshot.block_env);
        Ok(true)
    }

    async fn anvil_rollback(&self, depth: Option<u64>) -> RpcResult<()> {
        let depth = depth.unwrap_or(1);
        let best = self.best_block_number()?;
        let target = best.checked_sub(depth).ok_or_else(|| {
            invalid_params(format!("cannot roll back {depth} blocks from {best}"))
        })?;
        let header = self.sealed_header(target)?;
        self.rewind_to(&header).await
    }

    async fn anvil_drop_transaction(&self, tx_hash: B256) -> RpcResult<Option<B256>> {
        Ok(self.pool.remove_transaction(tx_hash).map(|_| {
            self.impersonation.forget_tx_sender(&tx_hash);
            tx_hash
        }))
    }

    async fn anvil_drop_all_transactions(&self) -> RpcResult<()> {
        let hashes = self.pool.all_transaction_hashes();
        if !hashes.is_empty() {
            self.pool.remove_transactions(hashes.clone());
            self.impersonation.forget_tx_senders(hashes);
        }
        Ok(())
    }

    async fn anvil_remove_pool_transactions(&self, address: Address) -> RpcResult<()> {
        let removed = self.pool.remove_transactions_by_sender(address);
        self.impersonation.forget_tx_senders(removed.into_iter().map(|tx| *tx.hash()));
        Ok(())
    }

    async fn anvil_get_genesis_time(&self) -> RpcResult<u64> {
        let header = self
            .provider
            .sealed_header(0)
            .map_err(|error| internal_error(format!("failed to read genesis header: {error}")))?
            .ok_or_else(|| internal_error("genesis block not found"))?;
        Ok(header.timestamp())
    }

    async fn anvil_node_info(&self) -> RpcResult<NodeInfo> {
        let latest = self.latest_block().await?;
        let gas_price = EthApiServer::gas_price(&self.eth).await?;

        Ok(NodeInfo {
            current_block_number: latest.header.number,
            current_block_timestamp: latest.header.timestamp,
            current_block_hash: latest.header.hash,
            hard_fork: self.hardfork_name(latest.header.timestamp, latest.header.number),
            transaction_order: "fees".to_string(),
            environment: NodeEnvironment {
                base_fee: latest.header.base_fee_per_gas.unwrap_or_default().into(),
                chain_id: self.chain_spec.chain().id(),
                gas_limit: latest.header.gas_limit,
                gas_price: gas_price.to(),
            },
            fork_config: NodeForkConfig::default(),
            network: None,
        })
    }

    async fn anvil_metadata(&self) -> RpcResult<Metadata> {
        let latest = self.latest_block().await?;

        Ok(Metadata {
            client_version: format!("{}/v{}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION")),
            client_semver: Some(env!("CARGO_PKG_VERSION").to_string()),
            client_commit_sha: None,
            chain_id: self.chain_spec.chain().id(),
            instance_id: self.instance_id,
            latest_block_number: latest.header.number,
            latest_block_hash: latest.header.hash,
            forked_network: None,
            snapshots: self.snapshots.metadata(),
        })
    }

    async fn anvil_set_block_timestamp_interval(&self, seconds: u64) -> RpcResult<()> {
        self.time.set_block_timestamp_interval(seconds);
        Ok(())
    }

    async fn anvil_remove_block_timestamp_interval(&self) -> RpcResult<bool> {
        Ok(self.time.remove_block_timestamp_interval())
    }

    async fn anvil_increase_time(&self, seconds: U256) -> RpcResult<i64> {
        let offset = self.time.increase_time(seconds.to::<u64>());
        Ok(offset.min(i64::MAX as i128) as i64)
    }

    async fn anvil_set_time(&self, timestamp: u64) -> RpcResult<u64> {
        // Accept millisecond timestamps for compatibility with anvil.
        let timestamp = if timestamp > 1_000_000_000_000 { timestamp / 1000 } else { timestamp };
        let now = self.time.current_call_timestamp();
        self.time.set_time(timestamp);
        Ok(timestamp.saturating_sub(now))
    }

    async fn anvil_set_next_block_timestamp(&self, seconds: u64) -> RpcResult<()> {
        self.time.set_next_block_timestamp(seconds).map_err(invalid_params)
    }

    async fn anvil_set_block_gas_limit(&self, gas_limit: U256) -> RpcResult<bool> {
        let gas_limit =
            gas_limit.try_into().map_err(|_| invalid_params("gas_limit exceeds u64::MAX"))?;
        self.block_env.set_gas_limit(gas_limit);
        Ok(true)
    }

    async fn anvil_set_coinbase(&self, address: Address) -> RpcResult<()> {
        self.block_env.set_coinbase(address);
        Ok(())
    }

    async fn anvil_set_next_block_base_fee_per_gas(&self, base_fee: U256) -> RpcResult<()> {
        let base_fee =
            base_fee.try_into().map_err(|_| invalid_params("base_fee exceeds u64::MAX"))?;
        self.block_env.set_next_base_fee(base_fee);
        Ok(())
    }

    async fn anvil_set_balance(&self, address: Address, balance: U256) -> RpcResult<()> {
        self.state.write().set_balance(address, balance);
        Ok(())
    }

    async fn anvil_add_balance(&self, address: Address, balance: U256) -> RpcResult<()> {
        let current = self.latest_balance(address)?;
        self.state.write().set_balance(address, current.saturating_add(balance));
        Ok(())
    }

    async fn anvil_set_nonce(&self, address: Address, nonce: U256) -> RpcResult<()> {
        let nonce = nonce.try_into().map_err(|_| invalid_params("nonce exceeds u64::MAX"))?;
        self.state.write().set_nonce(address, nonce);
        Ok(())
    }

    async fn anvil_set_code(&self, address: Address, code: Bytes) -> RpcResult<()> {
        self.state.write().set_code(address, Bytecode::new_raw(code));
        Ok(())
    }

    async fn anvil_set_storage_at(
        &self,
        address: Address,
        slot: U256,
        value: B256,
    ) -> RpcResult<bool> {
        self.state.write().set_storage_at(address, slot.into(), value.into());
        Ok(true)
    }
}

#[async_trait]
impl<Pool, Provider, Eth> EvmApiServer for AnvilRpc<Pool, Provider, Eth>
where
    Pool: TransactionPool + Send + Sync + 'static,
    Provider: BlockNumReader
        + HeaderProvider<Header = alloy_consensus::Header>
        + TransactionsProvider
        + StateProviderFactory
        + Send
        + Sync
        + 'static,
    Eth: FullEthApiServer<NetworkTypes = Ethereum>,
{
    async fn evm_mine(&self, opts: Option<MineOptions>) -> RpcResult<String> {
        self.anvil_mine_detailed(opts).await?;
        Ok("0x0".to_string())
    }
}

fn internal_error(message: impl Into<String>) -> ErrorObjectOwned {
    ErrorObjectOwned::owned(INTERNAL_ERROR_CODE, message.into(), None::<()>)
}

fn invalid_params(message: impl Into<String>) -> ErrorObjectOwned {
    ErrorObjectOwned::owned(INVALID_PARAMS_CODE, message.into(), None::<()>)
}
