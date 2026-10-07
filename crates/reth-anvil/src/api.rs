use crate::{
    block_env::BlockEnvOverrides,
    fork::ForkInfo,
    impersonation::ImpersonationState,
    logging::LoggingState,
    mining::MiningController,
    node::Relauncher,
    snapshot::{Snapshot, SnapshotManager},
    state::{AnvilState, SharedAnvilState},
    state_dump::{AccountDump, SerializableState},
    time::TimeManager,
    types::{ForkChoice, ForkUrl, ReorgOptions, ReorgParams, TransactionData, TransactionOrder},
};
use alloy_consensus::{Blob, BlockHeader, transaction::TxHashRef};
use alloy_eips::{BlockId, BlockNumberOrTag, eip7594::BlobTransactionSidecarVariant};
use alloy_json_rpc::RpcObject;
use alloy_network::{TransactionBuilder, primitives::HeaderResponse};
use alloy_primitives::{Address, B256, Bytes, TxKind, U64, U256};
use alloy_rpc_types::anvil::{
    ForkedNetwork, Forking, Metadata, MineOptions, NodeEnvironment, NodeForkConfig, NodeInfo,
};
use alloy_rpc_types_eth::{
    BlockOverrides, TransactionRequest,
    erc4337::TransactionConditional,
    state::{AccountOverride, StateOverride, StateOverridesBuilder},
};
use foundry_common::version::{COMMIT_SHA, SEMVER_VERSION};
use foundry_evm_core::utils::block_env_from_header;
use jsonrpsee::{
    core::{RpcResult, async_trait},
    proc_macros::rpc,
    types::{
        ErrorObjectOwned,
        error::{INTERNAL_ERROR_CODE, INVALID_PARAMS_CODE},
    },
};
use parking_lot::RwLock;
use reth_ethereum::{
    chainspec::{EthChainSpec, EthereumHardforks, Hardforks, MIN_TRANSACTION_GAS},
    pool::TransactionPool,
    primitives::{Bytecode, SealedHeader},
    storage::{BlockNumReader, HeaderProvider, StateProviderFactory, TransactionsProvider},
};
use reth_rpc_eth_api::{EthApiServer, FullEthApiServer, RpcBlock, RpcReceipt, RpcTxReq, RpcTypes};
use reth_rpc_server_types::constants::gas_oracle::ESTIMATE_GAS_ERROR_RATIO;
use revm::context::BlockEnv;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

/// The `anvil_*` RPC namespace, with the `hardhat_*` and `evm_*` aliases that anvil accepts.
#[rpc(server, namespace = "anvil")]
pub trait AnvilApi<B: RpcObject, TxReq: RpcObject> {
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
    async fn anvil_mine_detailed(&self, opts: Option<MineOptions>) -> RpcResult<Vec<B>>;

    /// Snapshots the chain head and the anvil settings. Returns the snapshot id.
    #[method(name = "snapshot", aliases = ["evm_snapshot"])]
    async fn anvil_snapshot(&self) -> RpcResult<U256>;

    /// Reverts the chain to the given snapshot. Returns whether the snapshot existed.
    #[method(name = "revert", aliases = ["evm_revert"])]
    async fn anvil_revert(&self, id: U256) -> RpcResult<bool>;

    /// Rewinds the chain by the given number of blocks.
    #[method(name = "rollback")]
    async fn anvil_rollback(&self, depth: Option<u64>) -> RpcResult<()>;

    /// Rewinds the chain by `depth` blocks and mines `depth` blocks with the given transactions.
    #[method(name = "reorg")]
    async fn anvil_reorg(
        &self,
        params: ReorgParams<TxReq>,
        tx_block_pairs: Option<Vec<(TransactionData<TxReq>, u64)>>,
    ) -> RpcResult<()>;

    /// Resets the chain to genesis, or to the fork block when forking. Changing the fork endpoint
    /// or block is not supported yet.
    #[method(name = "reset", aliases = ["hardhat_reset"])]
    async fn anvil_reset(&self, forking: Option<Forking>) -> RpcResult<()>;

    /// Sets the chain id. The node relaunches with its state and height; earlier blocks are no
    /// longer served.
    #[method(name = "setChainId")]
    async fn anvil_set_chain_id(&self, chain_id: u64) -> RpcResult<()>;

    /// Replaces the fork endpoint.
    #[method(name = "setRpcUrl")]
    async fn anvil_set_rpc_url(&self, url: String) -> RpcResult<()>;

    /// Sets the minimum gas price. Rejected while EIP-1559 is active, as in anvil.
    #[method(name = "setMinGasPrice", aliases = ["hardhat_setMinGasPrice"])]
    async fn anvil_set_min_gas_price(&self, gas_price: U256) -> RpcResult<()>;

    /// Enables or disables transaction logging.
    #[method(name = "setLoggingEnabled", aliases = ["hardhat_setLoggingEnabled"])]
    async fn anvil_set_logging_enabled(&self, enabled: bool) -> RpcResult<()>;

    /// Returns the wall clock time at which the last block was built.
    #[method(name = "getLastBlockWallTime")]
    async fn anvil_get_last_block_wall_time(&self) -> RpcResult<u64>;

    /// Returns the pool blob with the given versioned hash.
    #[method(name = "getBlobByHash")]
    async fn anvil_get_blob_by_hash(&self, hash: B256) -> RpcResult<Option<Box<Blob>>>;

    /// Returns the pool blobs of the given transaction.
    #[method(name = "getBlobsByTransactionHash")]
    async fn anvil_get_blobs_by_transaction_hash(&self, hash: B256)
    -> RpcResult<Option<Vec<Blob>>>;

    /// Sets the ERC20 balance of an account by finding and overriding the balance slot.
    #[method(name = "dealERC20", aliases = ["hardhat_dealERC20", "anvil_setERC20Balance"])]
    async fn anvil_deal_erc20(
        &self,
        address: Address,
        token_address: Address,
        balance: U256,
    ) -> RpcResult<()>;

    /// Sets an ERC20 allowance by finding and overriding the allowance slot.
    #[method(name = "setERC20Allowance")]
    async fn anvil_set_erc20_allowance(
        &self,
        owner: Address,
        spender: Address,
        token_address: Address,
        amount: U256,
    ) -> RpcResult<()>;

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

    /// Sets the prevrandao of the next block.
    #[method(name = "setNextBlockPrevRandao")]
    async fn anvil_set_next_block_prev_randao(&self, prev_randao: B256) -> RpcResult<()>;

    /// Sets the parent beacon block root of the next block.
    #[method(name = "setNextBlockParentBeaconBlockRoot")]
    async fn anvil_set_next_block_parent_beacon_block_root(&self, root: B256) -> RpcResult<()>;

    /// Sets the balance of an account.
    #[method(name = "setBalance", aliases = ["hardhat_setBalance", "tenderly_setBalance"])]
    async fn anvil_set_balance(&self, address: Address, balance: U256) -> RpcResult<()>;

    /// Adds to the balance of an account.
    #[method(name = "addBalance", aliases = ["hardhat_addBalance", "tenderly_addBalance"])]
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

    /// Makes every transaction carrying `signature` recover to `address`.
    #[method(name = "impersonateSignature")]
    async fn anvil_impersonate_signature(
        &self,
        signature: Bytes,
        address: Address,
    ) -> RpcResult<()>;

    /// Returns the state of the chain as gzipped JSON.
    #[method(name = "dumpState", aliases = ["hardhat_dumpState"])]
    async fn anvil_dump_state(&self, preserve_historical_states: Option<bool>) -> RpcResult<Bytes>;

    /// Applies a state dump on top of the current state. Nonces take the higher value.
    #[method(name = "loadState", aliases = ["hardhat_loadState"])]
    async fn anvil_load_state(&self, buf: Bytes) -> RpcResult<bool>;
}

/// The `evm_*` methods that have no `anvil_*` counterpart.
#[rpc(server, namespace = "evm")]
pub trait EvmApi {
    /// Mines blocks and returns `"0x0"`, as Hardhat does.
    #[method(name = "mine")]
    async fn evm_mine(&self, opts: Option<MineOptions>) -> RpcResult<String>;
}

/// The `eth_*` methods anvil adds on top of the standard namespace, or replaces.
#[rpc(server, namespace = "eth")]
pub trait EthExtApi<TxReq: RpcObject, Receipt: RpcObject> {
    /// Signs and sends a transaction from a dev account. A request without `to` deploys a
    /// contract.
    ///
    /// Replaces reth's method, which rejects a request without `to`, because a create recipient
    /// serializes as `null` and reads back as missing.
    #[method(name = "sendTransaction")]
    async fn eth_send_transaction(&self, request: TxReq) -> RpcResult<B256>;

    /// Signs and sends a transaction, and waits for its receipt.
    #[method(name = "sendTransactionSync")]
    async fn eth_send_transaction_sync(&self, request: TxReq) -> RpcResult<Receipt>;

    /// Sends a transaction from `from` without a signature, as if the account were impersonated.
    #[method(name = "sendUnsignedTransaction")]
    async fn eth_send_unsigned_transaction(&self, request: TxReq) -> RpcResult<B256>;

    /// Sends `request` again with a new gas price or gas limit, replacing the pending
    /// transaction with the same nonce.
    #[method(name = "resend")]
    async fn eth_resend(
        &self,
        request: TxReq,
        gas_price: Option<U256>,
        gas_limit: Option<U64>,
    ) -> RpcResult<B256>;

    /// Sends a signed transaction. The condition is accepted and ignored, as anvil does.
    #[method(name = "sendRawTransactionConditional")]
    async fn eth_send_raw_transaction_conditional(
        &self,
        tx: Bytes,
        condition: TransactionConditional,
    ) -> RpcResult<B256>;

    /// Returns the dev accounts, like `eth_accounts`.
    #[method(name = "requestAccounts")]
    async fn eth_request_accounts(&self) -> RpcResult<Vec<Address>>;

    /// Returns the chain id as a decimal string, like `net_version`.
    #[method(name = "networkId")]
    async fn eth_network_id(&self) -> RpcResult<Option<String>>;

    /// Returns the gas price: the base fee plus the suggested tip, the base fee alone when the
    /// minimum priority fee is disabled, or the node's gas price before London, as anvil does.
    #[method(name = "gasPrice")]
    async fn eth_gas_price(&self) -> RpcResult<U256>;

    /// Estimates the gas of a call down to the exact limit, as anvil does; reth stops its search
    /// within 1.5% above it. A request without `from` is not capped by the zero address's
    /// balance.
    #[method(name = "estimateGas")]
    async fn eth_estimate_gas(
        &self,
        request: TxReq,
        block: Option<BlockId>,
        state_overrides: Option<StateOverride>,
        block_overrides: Option<Box<BlockOverrides>>,
    ) -> RpcResult<U256>;
}

/// The `personal_*` namespace.
#[rpc(server, namespace = "personal")]
pub trait PersonalApi {
    /// Signs `message` with `address`, like `eth_sign` with the parameters swapped.
    #[method(name = "sign")]
    async fn personal_sign(&self, message: Bytes, address: Address) -> RpcResult<Bytes>;
}

/// The client version `anvil_metadata` and `web3_clientVersion` report.
pub const CLIENT_VERSION: &str = concat!(env!("CARGO_PKG_NAME"), "/v", env!("CARGO_PKG_VERSION"));

/// The header type of a provider.
type HeaderOf<Provider> = <Provider as HeaderProvider>::Header;

/// Marks a request without `to` as a contract creation, so the signer can build it.
fn with_recipient<TxReq: AsMut<TransactionRequest>>(mut request: TxReq) -> TxReq {
    if request.as_mut().to.is_none() {
        request.as_mut().to = Some(TxKind::Create);
    }
    request
}

/// Funds the zero address, which stands in for a missing `from`, so a request with fee fields is
/// not capped by that balance. Anvil charges no fee for a request without `from`.
fn fund_default_caller(
    request: &TransactionRequest,
    overrides: Option<StateOverride>,
) -> Option<StateOverride> {
    let has_fees = request.gas_price.is_some()
        || request.max_fee_per_gas.is_some()
        || request.max_fee_per_blob_gas.is_some();
    if request.from.is_some() || !has_fees {
        return overrides;
    }
    let mut overrides = overrides.unwrap_or_default();
    overrides.entry(Address::ZERO).or_default().balance.get_or_insert(U256::from(u128::MAX));
    Some(overrides)
}

/// How long `eth_sendTransactionSync` waits for the receipt.
const TRANSACTION_CONFIRMATION_TIMEOUT: Duration = Duration::from_secs(30);

/// How often `eth_sendTransactionSync` polls for the receipt.
const RECEIPT_POLL_INTERVAL: Duration = Duration::from_millis(20);

/// What `anvil_nodeInfo` reports about the network the node runs.
#[derive(Clone, Debug, Default)]
pub struct NodeIdentity {
    /// The network family, if not Ethereum.
    pub network: Option<&'static str>,
    /// The hardfork name, when the chain spec's Ethereum forks do not describe it.
    pub hardfork: Option<String>,
}

/// Implementation of the `anvil_*` RPC namespace.
#[derive(Debug, Clone)]
pub struct AnvilRpc<Pool, Provider: HeaderProvider, Eth, Spec> {
    identity: NodeIdentity,
    relauncher: Relauncher,
    impersonation: ImpersonationState,
    mining: MiningController<HeaderOf<Provider>>,
    time: TimeManager,
    block_env: BlockEnvOverrides,
    state: SharedAnvilState,
    snapshots: SnapshotManager<HeaderOf<Provider>>,
    chain_spec: Arc<Spec>,
    instance_id: Arc<RwLock<B256>>,
    logging: LoggingState,
    transaction_order: TransactionOrder,
    min_priority_fee_enforced: bool,
    fork: Option<Arc<dyn ForkInfo>>,
    pool: Pool,
    provider: Provider,
    eth: Eth,
}

impl<Pool, Provider: HeaderProvider, Eth, Spec> AnvilRpc<Pool, Provider, Eth, Spec> {
    /// Creates the `anvil_*` namespace over the given node components.
    #[expect(clippy::too_many_arguments)]
    pub fn new(
        identity: NodeIdentity,
        relauncher: Relauncher,
        impersonation: ImpersonationState,
        mining: MiningController<HeaderOf<Provider>>,
        time: TimeManager,
        block_env: BlockEnvOverrides,
        state: SharedAnvilState,
        snapshots: SnapshotManager<HeaderOf<Provider>>,
        chain_spec: Arc<Spec>,
        instance_id: Arc<RwLock<B256>>,
        logging: LoggingState,
        transaction_order: TransactionOrder,
        min_priority_fee_enforced: bool,
        fork: Option<Arc<dyn ForkInfo>>,
        pool: Pool,
        provider: Provider,
        eth: Eth,
    ) -> Self {
        Self {
            identity,
            relauncher,
            impersonation,
            mining,
            time,
            block_env,
            state,
            snapshots,
            chain_spec,
            instance_id,
            logging,
            transaction_order,
            min_priority_fee_enforced,
            fork,
            pool,
            provider,
            eth,
        }
    }
}

impl<Pool, Provider: BlockNumReader + HeaderProvider, Eth, Spec>
    AnvilRpc<Pool, Provider, Eth, Spec>
{
    fn best_block_number(&self) -> RpcResult<u64> {
        self.provider
            .best_block_number()
            .map_err(|error| internal_error(format!("failed to read latest block number: {error}")))
    }

    fn sealed_header(&self, number: u64) -> RpcResult<SealedHeader<HeaderOf<Provider>>> {
        self.provider
            .sealed_header(number)
            .map_err(|error| internal_error(format!("failed to read header {number}: {error}")))?
            .ok_or_else(|| internal_error(format!("missing block header {number}")))
    }
}

impl<Pool, Provider, Eth, Spec> AnvilRpc<Pool, Provider, Eth, Spec>
where
    Provider:
        BlockNumReader + HeaderProvider + TransactionsProvider + StateProviderFactory + AccountDump,
    Eth: FullEthApiServer<NetworkTypes: RpcTypes<TransactionRequest: Default>>,
    Spec: EthChainSpec + EthereumHardforks + Hardforks,
{
    async fn block_by_number(
        &self,
        number: u64,
        full: bool,
    ) -> RpcResult<RpcBlock<Eth::NetworkTypes>> {
        EthApiServer::block_by_number(&self.eth, BlockNumberOrTag::Number(number), full)
            .await?
            .ok_or_else(|| internal_error(format!("missing block {number}")))
    }

    async fn latest_block(&self) -> RpcResult<RpcBlock<Eth::NetworkTypes>> {
        self.block_by_number(self.best_block_number()?, false).await
    }

    /// Mines `blocks` blocks and returns their numbers.
    async fn mine_blocks(&self, blocks: u64) -> RpcResult<Vec<u64>> {
        // Anvil mines the blocks of one request within the same second. Blocks take longer
        // here, so they share the first block's timestamp.
        let shared = (blocks > 1 && self.time.interval().is_none())
            .then(|| self.time.current_call_timestamp());
        let mut mined = Vec::with_capacity(blocks as usize);
        for _ in 0..blocks {
            if let Some(timestamp) = shared {
                self.time.pin_next_timestamp(timestamp);
            }
            mined.push(self.mining.mine_block().await.map_err(internal_error)?.number());
        }
        Ok(mined)
    }

    /// Rewinds the chain to the given canonical header and drops the transactions of the removed
    /// blocks, so the pool does not mine them again.
    async fn rewind_to(&self, header: &SealedHeader<HeaderOf<Provider>>) -> RpcResult<()> {
        let best = self.best_block_number()?;
        if header.number() < best {
            let removed = self
                .provider
                .transactions_by_block_range(header.number() + 1..=best)
                .map_err(|error| internal_error(format!("failed to read transactions: {error}")))?;
            self.impersonation.drop_txs(removed.into_iter().flatten().map(|tx| *tx.tx_hash()));
        }
        self.mining.rewind(header.clone()).await.map_err(internal_error)?;
        self.state.write().rewind_to(header.number());
        Ok(())
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

    /// Finds the storage slot of `token_address` that `calldata` reads, by checking which slot
    /// from the access list changes the call result to `expected_value`.
    async fn find_erc20_storage_slot(
        &self,
        token_address: Address,
        calldata: Bytes,
        expected_value: U256,
    ) -> RpcResult<B256> {
        let mut tx = RpcTxReq::<Eth::NetworkTypes>::default();
        *tx.as_mut() = TransactionRequest::default().with_to(token_address).with_input(calldata);
        let access_list =
            EthApiServer::create_access_list(&self.eth, tx.clone(), None, None).await?.access_list;

        for item in access_list.0 {
            if item.address != token_address {
                continue;
            }
            for slot in item.storage_keys {
                let state_override = StateOverridesBuilder::default()
                    .append(
                        token_address,
                        AccountOverride::default()
                            .with_state_diff(std::iter::once((slot, expected_value.into()))),
                    )
                    .build();
                let Ok(result) =
                    EthApiServer::call(&self.eth, tx.clone(), None, Some(state_override), None)
                        .await
                else {
                    continue;
                };
                if U256::from_be_slice(result.as_ref()) == expected_value {
                    return Ok(slot);
                }
            }
        }

        Err(internal_error("Unable to find storage slot"))
    }

    /// Returns the lowercase name of the latest hardfork active at the given block.
    /// Reads the accounts, the pending anvil state writes, and the block environment into a
    /// state dump.
    fn serializable_state(&self) -> RpcResult<SerializableState> {
        let best = self.best_block_number()?;
        let header = self.sealed_header(best)?;
        let mut accounts = self
            .provider
            .dump_accounts()
            .map_err(|error| internal_error(format!("failed to read accounts: {error}")))?;
        // The anvil state writes not yet in a block.
        {
            let state = self.state.read();
            for (address, account_override) in state.accounts() {
                let record = accounts.entry(*address).or_default();
                if let Some(balance) = account_override.balance() {
                    record.balance = balance;
                }
                if let Some(nonce) = account_override.nonce() {
                    record.nonce = nonce;
                }
                if let Some(code) =
                    account_override.code_hash().and_then(|hash| state.bytecode_by_hash(&hash))
                {
                    record.code = code.original_bytes();
                }
                for (slot, value) in account_override.storage() {
                    if value.is_zero() {
                        record.storage.remove(slot);
                    } else {
                        record.storage.insert(*slot, (*value).into());
                    }
                }
            }
        }
        let block = block_env_from_header::<BlockEnv>(header.header());
        let state = SerializableState {
            block: Some(
                serde_json::to_value(block).map_err(|error| internal_error(error.to_string()))?,
            ),
            accounts,
            best_block_number: Some(best),
            ..Default::default()
        };
        Ok(state)
    }

    fn hardfork_name(&self, timestamp: u64, number: u64) -> String {
        self.chain_spec
            .forks_iter()
            .filter(|(_, condition)| condition.active_at_timestamp_or_number(timestamp, number))
            .last()
            .map(|(fork, _)| fork.name().to_string())
            .unwrap_or_default()
    }
}

#[async_trait]
impl<Pool, Provider, Eth, Spec>
    AnvilApiServer<RpcBlock<Eth::NetworkTypes>, RpcTxReq<Eth::NetworkTypes>>
    for AnvilRpc<Pool, Provider, Eth, Spec>
where
    Pool: TransactionPool + Send + Sync + 'static,
    Provider: BlockNumReader
        + HeaderProvider
        + TransactionsProvider
        + StateProviderFactory
        + AccountDump
        + Send
        + Sync
        + 'static,
    Eth: FullEthApiServer<NetworkTypes: RpcTypes<TransactionRequest: Default>>,
    Spec: EthChainSpec + EthereumHardforks + Hardforks + Send + Sync + 'static,
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

    async fn anvil_mine_detailed(
        &self,
        opts: Option<MineOptions>,
    ) -> RpcResult<Vec<RpcBlock<Eth::NetworkTypes>>> {
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

    async fn anvil_reorg(
        &self,
        params: ReorgParams<RpcTxReq<Eth::NetworkTypes>>,
        tx_block_pairs: Option<Vec<(TransactionData<RpcTxReq<Eth::NetworkTypes>>, u64)>>,
    ) -> RpcResult<()> {
        let ReorgOptions { depth, mut tx_block_pairs } = params.into_options(tx_block_pairs);
        if let Some((_, number)) = tx_block_pairs.iter().find(|(_, number)| *number >= depth) {
            let Some(last_block) = depth.checked_sub(1) else {
                return Err(invalid_params(
                    "Reorg depth must be at least 1 to include transactions",
                ));
            };
            return Err(invalid_params(format!(
                "Block number for reorg tx will exceed the reorged chain height. Block number {number} must not exceed (depth-1) {last_block}"
            )));
        }
        tx_block_pairs.sort_by_key(|(_, number)| *number);

        let current_height = self.best_block_number()?;
        let common_height = current_height.checked_sub(depth).ok_or_else(|| {
            invalid_params(format!(
                "Reorg depth must not exceed current chain height: current height {current_height}, depth {depth}"
            ))
        })?;
        let header = self.sealed_header(common_height)?;
        self.rewind_to(&header).await?;

        // Mine the blocks by hand so interval or automine blocks do not interleave.
        let automine = self.mining.is_automine();
        let interval = self.mining.interval_mining();
        self.mining.set_interval_mining(0);
        let mut pairs = tx_block_pairs.into_iter().peekable();
        let mut result = Ok(());
        for offset in 0..depth {
            while let Some((tx, _)) = pairs.next_if(|(_, number)| *number == offset) {
                let sent = match tx {
                    TransactionData::JSON(request) => match self.with_sender(request) {
                        Ok(request) => EthApiServer::send_transaction(&self.eth, request).await,
                        Err(error) => Err(error),
                    },
                    TransactionData::Raw(bytes) => {
                        EthApiServer::send_raw_transaction(&self.eth, bytes).await
                    }
                };
                if let Err(error) = sent {
                    result = Err(error);
                    break;
                }
            }
            if result.is_err() {
                break;
            }
            if let Err(error) = self.mining.mine_block().await {
                result = Err(internal_error(error));
                break;
            }
        }
        if let Some(interval) = interval {
            self.mining.set_interval_mining(interval);
        }
        self.mining.set_automine(automine);
        result
    }

    async fn anvil_reset(&self, forking: Option<Forking>) -> RpcResult<()> {
        if let Some(forking) = forking {
            let same_url = forking
                .json_rpc_url
                .as_ref()
                .is_none_or(|url| self.fork.as_ref().is_some_and(|fork| fork.url() == *url));
            let same_block = forking.block_number.is_none_or(|number| {
                self.fork.as_ref().is_some_and(|fork| fork.block_number() == number)
            });
            if !same_url || !same_block {
                // Another endpoint or block means another chain spec, so the node relaunches.
                return self
                    .relauncher
                    .relaunch(|config| {
                        if let Some(url) = forking.json_rpc_url {
                            config.fork_urls = vec![ForkUrl { url, block: None }];
                        }
                        config.fork_choice =
                            forking.block_number.map(|number| ForkChoice::Block(number.into()));
                        config.init_state = None;
                    })
                    .await
                    .map_err(internal_error);
            }
        }
        if self.fork.is_some() {
            // Anvil turns a forked node back into a plain one.
            return self
                .relauncher
                .relaunch_from_original(|config| {
                    config.fork_urls.clear();
                    config.fork_choice = None;
                    config.init_state = None;
                })
                .await
                .map_err(internal_error);
        }
        let genesis = self.sealed_header(self.chain_spec.genesis_header().number())?;
        self.rewind_to(&genesis).await?;
        let hashes = self.pool.all_transaction_hashes();
        if !hashes.is_empty() {
            self.pool.remove_transactions(hashes.clone());
            self.impersonation.forget_tx_senders(hashes);
        }
        *self.state.write() = AnvilState::default();
        self.snapshots.clear();
        self.time.set_time(self.chain_spec.genesis().timestamp);
        self.time.remove_block_timestamp_interval();
        self.block_env.restore(Default::default());
        *self.instance_id.write() = B256::random();
        Ok(())
    }

    async fn anvil_set_rpc_url(&self, url: String) -> RpcResult<()> {
        let Some(fork) = &self.fork else {
            return Err(invalid_params("anvil_setRpcUrl requires a forked node"));
        };
        fork.set_rpc_url(url).map_err(|error| internal_error(error.to_string()))
    }

    async fn anvil_set_min_gas_price(&self, gas_price: U256) -> RpcResult<()> {
        if self.chain_spec.is_london_active_at_block(0) {
            return Err(invalid_params(
                "anvil_setMinGasPrice is not supported when EIP-1559 is active",
            ));
        }
        self.block_env.set_gas_price(gas_price.saturating_to());
        Ok(())
    }

    async fn anvil_set_logging_enabled(&self, enabled: bool) -> RpcResult<()> {
        self.logging.set_enabled(enabled);
        Ok(())
    }

    async fn anvil_get_last_block_wall_time(&self) -> RpcResult<u64> {
        Ok(self.time.last_block_wall_time())
    }

    async fn anvil_get_blob_by_hash(&self, hash: B256) -> RpcResult<Option<Box<Blob>>> {
        let blobs = self
            .pool
            .get_blobs_for_versioned_hashes_v1(&[hash])
            .map_err(|error| internal_error(format!("failed to read blobs: {error}")))?;
        Ok(blobs.into_iter().flatten().next().map(|blob| blob.blob))
    }

    async fn anvil_get_blobs_by_transaction_hash(
        &self,
        hash: B256,
    ) -> RpcResult<Option<Vec<Blob>>> {
        let sidecar = self
            .pool
            .get_blob(hash)
            .map_err(|error| internal_error(format!("failed to read blobs: {error}")))?;
        Ok(sidecar.map(|sidecar| match sidecar.as_ref() {
            BlobTransactionSidecarVariant::Eip4844(sidecar) => sidecar.blobs.clone(),
            BlobTransactionSidecarVariant::Eip7594(sidecar) => sidecar.blobs.clone(),
        }))
    }

    async fn anvil_deal_erc20(
        &self,
        address: Address,
        token_address: Address,
        balance: U256,
    ) -> RpcResult<()> {
        const BALANCE_OF_SELECTOR: [u8; 4] = [0x70, 0xa0, 0x82, 0x31];

        let mut calldata = Vec::with_capacity(4 + 32);
        calldata.extend_from_slice(&BALANCE_OF_SELECTOR);
        calldata.extend_from_slice(&[0u8; 12]);
        calldata.extend_from_slice(address.as_slice());

        let slot = self.find_erc20_storage_slot(token_address, calldata.into(), balance).await?;
        self.state.write().set_storage_at(token_address, slot, balance);
        Ok(())
    }

    async fn anvil_set_erc20_allowance(
        &self,
        owner: Address,
        spender: Address,
        token_address: Address,
        amount: U256,
    ) -> RpcResult<()> {
        const ALLOWANCE_SELECTOR: [u8; 4] = [0xdd, 0x62, 0xed, 0x3e];

        let mut calldata = Vec::with_capacity(4 + 32 + 32);
        calldata.extend_from_slice(&ALLOWANCE_SELECTOR);
        calldata.extend_from_slice(&[0u8; 12]);
        calldata.extend_from_slice(owner.as_slice());
        calldata.extend_from_slice(&[0u8; 12]);
        calldata.extend_from_slice(spender.as_slice());

        let slot = self.find_erc20_storage_slot(token_address, calldata.into(), amount).await?;
        self.state.write().set_storage_at(token_address, slot, amount);
        Ok(())
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
            current_block_number: latest.header.number(),
            current_block_timestamp: latest.header.timestamp(),
            current_block_hash: latest.header.hash(),
            hard_fork: self.identity.hardfork.clone().unwrap_or_else(|| {
                self.hardfork_name(latest.header.timestamp(), latest.header.number())
            }),
            transaction_order: self.transaction_order.to_string(),
            environment: NodeEnvironment {
                base_fee: latest.header.base_fee_per_gas().unwrap_or_default().into(),
                chain_id: self.chain_spec.chain().id(),
                gas_limit: latest.header.gas_limit(),
                gas_price: gas_price.to(),
            },
            fork_config: self.fork.as_ref().map_or_else(NodeForkConfig::default, |fork| {
                NodeForkConfig {
                    fork_url: Some(fork.url()),
                    fork_block_number: Some(fork.block_number()),
                    fork_retry_backoff: Some(fork.retry_backoff().as_millis()),
                }
            }),
            network: self.identity.network.map(str::to_string),
        })
    }

    async fn anvil_metadata(&self) -> RpcResult<Metadata> {
        let latest = self.latest_block().await?;

        Ok(Metadata {
            client_version: CLIENT_VERSION.to_string(),
            client_semver: Some(SEMVER_VERSION.to_string()),
            client_commit_sha: Some(COMMIT_SHA.to_string()),
            chain_id: self.chain_spec.chain().id(),
            instance_id: *self.instance_id.read(),
            latest_block_number: latest.header.number(),
            latest_block_hash: latest.header.hash(),
            forked_network: self.fork.as_ref().map(|fork| ForkedNetwork {
                chain_id: fork.chain_id(),
                fork_block_number: fork.block_number(),
                fork_block_hash: fork.block_hash(),
            }),
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

    async fn anvil_set_next_block_prev_randao(&self, prev_randao: B256) -> RpcResult<()> {
        self.block_env.set_next_prev_randao(prev_randao);
        Ok(())
    }

    async fn anvil_set_next_block_parent_beacon_block_root(&self, root: B256) -> RpcResult<()> {
        self.block_env.set_next_parent_beacon_block_root(root);
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

    async fn anvil_impersonate_signature(
        &self,
        signature: Bytes,
        address: Address,
    ) -> RpcResult<()> {
        if signature.len() != 65 {
            return Err(invalid_params("signature must be 65 bytes"));
        }
        self.impersonation.add_signature_override(signature, address);
        Ok(())
    }

    async fn anvil_set_chain_id(&self, chain_id: u64) -> RpcResult<()> {
        let state = self.serializable_state()?;
        self.relauncher
            .relaunch(|config| {
                config.set_chain_id(Some(chain_id));
                config.init_state = Some(state);
            })
            .await
            .map_err(internal_error)
    }

    async fn anvil_dump_state(
        &self,
        _preserve_historical_states: Option<bool>,
    ) -> RpcResult<Bytes> {
        self.serializable_state()?.encode().map_err(|error| internal_error(error.to_string()))
    }

    async fn anvil_load_state(&self, buf: Bytes) -> RpcResult<bool> {
        let state = SerializableState::decode(&buf)
            .map_err(|error| invalid_params(format!("invalid state dump: {error}")))?;
        let latest = self
            .provider
            .latest()
            .map_err(|error| internal_error(format!("failed to read state: {error}")))?;
        // Read the current nonces before taking the write lock: the state provider reads the
        // overlay under the same lock.
        let mut nonces = Vec::with_capacity(state.accounts.len());
        for (address, record) in &state.accounts {
            let current_nonce = latest
                .basic_account(address)
                .map_err(|error| internal_error(format!("failed to read account: {error}")))?
                .map(|account| account.nonce)
                .unwrap_or_default();
            nonces.push(record.nonce.max(current_nonce));
        }
        drop(latest);
        let mut writes = self.state.write();
        for ((address, record), nonce) in state.accounts.into_iter().zip(nonces) {
            writes.set_nonce(address, nonce);
            writes.set_balance(address, record.balance);
            if !record.code.is_empty() {
                writes.set_code(address, Bytecode::new_raw(record.code));
            }
            for (slot, value) in record.storage {
                writes.set_storage_at(address, slot, value.into());
            }
        }
        Ok(true)
    }
}

#[async_trait]
impl<Pool, Provider, Eth, Spec> EvmApiServer for AnvilRpc<Pool, Provider, Eth, Spec>
where
    Pool: TransactionPool + Send + Sync + 'static,
    Provider: BlockNumReader
        + HeaderProvider
        + TransactionsProvider
        + StateProviderFactory
        + AccountDump
        + Send
        + Sync
        + 'static,
    Eth: FullEthApiServer<NetworkTypes: RpcTypes<TransactionRequest: Default>>,
    Spec: EthChainSpec + EthereumHardforks + Hardforks + Send + Sync + 'static,
{
    async fn evm_mine(&self, opts: Option<MineOptions>) -> RpcResult<String> {
        self.anvil_mine_detailed(opts).await?;
        Ok("0x0".to_string())
    }
}

impl<Pool, Provider: HeaderProvider, Eth: FullEthApiServer, Spec>
    AnvilRpc<Pool, Provider, Eth, Spec>
{
    /// Fills a missing `from` with the first dev account, as anvil does, and marks a missing
    /// `to` as a contract creation.
    fn with_sender(
        &self,
        mut request: RpcTxReq<Eth::NetworkTypes>,
    ) -> RpcResult<RpcTxReq<Eth::NetworkTypes>> {
        if request.as_ref().from.is_none() {
            let accounts = EthApiServer::accounts(&self.eth)?;
            let from =
                accounts.first().copied().ok_or_else(|| invalid_params("No Signer available"))?;
            request.as_mut().from = Some(from);
        }
        Ok(with_recipient(request))
    }
}

#[async_trait]
impl<Pool, Provider, Eth, Spec>
    EthExtApiServer<RpcTxReq<Eth::NetworkTypes>, RpcReceipt<Eth::NetworkTypes>>
    for AnvilRpc<Pool, Provider, Eth, Spec>
where
    Pool: Send + Sync + 'static,
    Provider: BlockNumReader + HeaderProvider + Send + Sync + 'static,
    Eth: FullEthApiServer,
    Spec: Send + Sync + 'static,
{
    async fn eth_send_transaction(&self, request: RpcTxReq<Eth::NetworkTypes>) -> RpcResult<B256> {
        EthApiServer::send_transaction(&self.eth, self.with_sender(request)?).await
    }

    async fn eth_send_transaction_sync(
        &self,
        request: RpcTxReq<Eth::NetworkTypes>,
    ) -> RpcResult<RpcReceipt<Eth::NetworkTypes>> {
        let hash = EthApiServer::send_transaction(&self.eth, self.with_sender(request)?).await?;
        let deadline = Instant::now() + TRANSACTION_CONFIRMATION_TIMEOUT;
        loop {
            if let Some(receipt) = EthApiServer::transaction_receipt(&self.eth, hash).await? {
                return Ok(receipt);
            }
            if Instant::now() >= deadline {
                return Err(internal_error(format!(
                    "transaction {hash} was not mined within {TRANSACTION_CONFIRMATION_TIMEOUT:?}"
                )));
            }
            tokio::time::sleep(RECEIPT_POLL_INTERVAL).await;
        }
    }

    async fn eth_resend(
        &self,
        mut request: RpcTxReq<Eth::NetworkTypes>,
        gas_price: Option<U256>,
        gas_limit: Option<U64>,
    ) -> RpcResult<B256> {
        if request.as_ref().nonce.is_none() {
            return Err(invalid_params("missing transaction nonce in transaction spec"));
        }
        if let Some(gas_price) = gas_price {
            let gas_price =
                gas_price.try_into().map_err(|_| invalid_params("gas price exceeds u128"))?;
            let tx = request.as_mut();
            if tx.max_fee_per_gas.is_some() || tx.max_priority_fee_per_gas.is_some() {
                tx.max_fee_per_gas = Some(gas_price);
            } else {
                tx.gas_price = Some(gas_price);
            }
        }
        if let Some(gas_limit) = gas_limit {
            request.as_mut().gas = Some(gas_limit.to());
        }
        EthApiServer::send_transaction(&self.eth, with_recipient(request)).await
    }

    async fn eth_send_raw_transaction_conditional(
        &self,
        tx: Bytes,
        _condition: TransactionConditional,
    ) -> RpcResult<B256> {
        EthApiServer::send_raw_transaction(&self.eth, tx).await
    }

    async fn eth_request_accounts(&self) -> RpcResult<Vec<Address>> {
        EthApiServer::accounts(&self.eth)
    }

    async fn eth_network_id(&self) -> RpcResult<Option<String>> {
        Ok(EthApiServer::chain_id(&self.eth).await?.map(|id| id.to::<u64>().to_string()))
    }

    async fn eth_gas_price(&self) -> RpcResult<U256> {
        let base_fee = self.sealed_header(self.best_block_number()?)?.base_fee_per_gas();
        match base_fee {
            // Before London, the node's gas price, as `anvil_setMinGasPrice` sets it.
            None if let Some(gas_price) = self.block_env.gas_price() => Ok(U256::from(gas_price)),
            Some(base_fee) if !self.min_priority_fee_enforced => Ok(U256::from(base_fee)),
            _ => EthApiServer::gas_price(&self.eth).await,
        }
    }

    async fn eth_estimate_gas(
        &self,
        request: RpcTxReq<Eth::NetworkTypes>,
        block: Option<BlockId>,
        state_overrides: Option<StateOverride>,
        block_overrides: Option<Box<BlockOverrides>>,
    ) -> RpcResult<U256> {
        let state_overrides = fund_default_caller(request.as_ref(), state_overrides);
        let estimate = EthApiServer::estimate_gas(
            &self.eth,
            request.clone(),
            block,
            state_overrides.clone(),
            block_overrides.clone(),
        )
        .await?;
        let mut high = estimate.saturating_to::<u64>();
        if high <= MIN_TRANSACTION_GAS {
            return Ok(estimate);
        }
        // Reth stops once the failing and the passing limit are within the error ratio, so the
        // exact limit is above `low`. Probe the calls between them.
        let mut low = (high as f64 * (1.0 - ESTIMATE_GAS_ERROR_RATIO)) as u64;
        while low + 1 < high {
            let mid = low + (high - low) / 2;
            let mut probe = request.clone();
            probe.as_mut().gas = Some(mid);
            let passes = EthApiServer::call(
                &self.eth,
                probe,
                block,
                state_overrides.clone(),
                block_overrides.clone(),
            )
            .await
            .is_ok();
            if passes {
                high = mid;
            } else {
                low = mid;
            }
        }
        Ok(U256::from(high))
    }

    async fn eth_send_unsigned_transaction(
        &self,
        request: RpcTxReq<Eth::NetworkTypes>,
    ) -> RpcResult<B256> {
        let from = request.as_ref().from.ok_or_else(|| invalid_params("No Signer available"))?;
        let impersonated = self.impersonation.is_impersonated(&from);
        if !impersonated {
            self.impersonation.impersonate(from);
        }
        let result = EthApiServer::send_transaction(&self.eth, with_recipient(request)).await;
        if !impersonated {
            self.impersonation.stop_impersonating(from);
        }
        result
    }
}

#[async_trait]
impl<Pool, Provider, Eth, Spec> PersonalApiServer for AnvilRpc<Pool, Provider, Eth, Spec>
where
    Pool: Send + Sync + 'static,
    Provider: HeaderProvider + Send + Sync + 'static,
    Eth: FullEthApiServer,
    Spec: Send + Sync + 'static,
{
    async fn personal_sign(&self, message: Bytes, address: Address) -> RpcResult<Bytes> {
        EthApiServer::sign(&self.eth, address, message).await
    }
}

fn internal_error(message: impl Into<String>) -> ErrorObjectOwned {
    ErrorObjectOwned::owned(INTERNAL_ERROR_CODE, message.into(), None::<()>)
}

fn invalid_params(message: impl Into<String>) -> ErrorObjectOwned {
    ErrorObjectOwned::owned(INVALID_PARAMS_CODE, message.into(), None::<()>)
}
