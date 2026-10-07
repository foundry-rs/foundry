use crate::{
    block_env::BlockEnvOverrides,
    fork::ForkInfo,
    impersonation::ImpersonationState,
    logging::LoggingState,
    miner::HookFuture,
    mining::{MiningController, wait_for_pool},
    node::Relauncher,
    snapshot::{Snapshot, SnapshotManager},
    state::{AnvilState, SharedAnvilState},
    state_dump::{AccountDump, SerializableState},
    time::TimeManager,
    types::{ForkChoice, ForkUrl, ReorgOptions, ReorgParams, TransactionData, TransactionOrder},
};
use alloy_consensus::{
    Blob, BlockHeader, Transaction,
    transaction::{PooledTransaction, TxHashRef},
};
use alloy_dyn_abi::TypedData;
use alloy_eips::{BlockId, BlockNumberOrTag, eip7594::BlobTransactionSidecarVariant};
use alloy_json_rpc::RpcObject;
use alloy_network::{TransactionBuilder, primitives::HeaderResponse};
use alloy_primitives::{Address, B256, Bytes, TxKind, U64, U256};
use alloy_rpc_types::anvil::{
    ForkedNetwork, Forking, Metadata, MineOptions, NodeEnvironment, NodeForkConfig, NodeInfo,
};
use alloy_rpc_types_eth::{
    BlockOverrides, Bundle, EthCallResponse, FeeHistory, Filter, FilterId, StateContext,
    TransactionRequest,
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
use parking_lot::{Mutex, RwLock};
use reth_ethereum::{
    chainspec::{EthChainSpec, EthereumHardforks, Hardforks, MIN_TRANSACTION_GAS},
    pool::{TransactionPool, TransactionPoolExt},
    primitives::{Bytecode, SealedHeader},
    rpc::eth::{
        EthApiError, RpcInvalidTransactionError, error::RpcPoolError,
        utils::recover_raw_transaction,
    },
    storage::{BlockNumReader, HeaderProvider, StateProviderFactory, TransactionsProvider},
};
use reth_execution_types::ChangedAccount;
use reth_rpc_eth_api::{
    EthApiServer, FullEthApiServer, RpcBlock, RpcReceipt, RpcTransaction, RpcTxReq, RpcTypes,
};
use reth_rpc_server_types::constants::gas_oracle::ESTIMATE_GAS_ERROR_RATIO;
use revm::{context::BlockEnv, primitives::eip7825::TX_GAS_LIMIT_CAP};
use std::{
    collections::{HashMap, HashSet},
    fmt,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex as AsyncMutex;

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
pub trait EthExtApi<TxReq: RpcObject, Receipt: RpcObject, Tx: RpcObject> {
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

    /// Sends a signed transaction. A replacement must raise the fee, as on anvil.
    #[method(name = "sendRawTransaction")]
    async fn eth_send_raw_transaction(&self, tx: Bytes) -> RpcResult<B256>;

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
    /// balance, and fee fields below the base fee do not fail the call.
    #[method(name = "estimateGas")]
    async fn eth_estimate_gas(
        &self,
        request: TxReq,
        block: Option<BlockId>,
        state_overrides: Option<StateOverride>,
        block_overrides: Option<Box<BlockOverrides>>,
    ) -> RpcResult<U256>;

    /// Runs a call. Fee fields below the base fee do not fail the call, as on anvil.
    #[method(name = "call")]
    async fn eth_call(
        &self,
        request: TxReq,
        block: Option<BlockId>,
        state_overrides: Option<StateOverride>,
        block_overrides: Option<Box<BlockOverrides>>,
    ) -> RpcResult<Bytes>;

    /// Runs bundles of calls. Every bundle answers, an empty one with no results, and each
    /// bundle runs one block later than the one before it, as on anvil.
    #[method(name = "callMany")]
    async fn eth_call_many(
        &self,
        bundles: Vec<Bundle<TxReq>>,
        state_context: Option<StateContext>,
        state_override: Option<StateOverride>,
    ) -> RpcResult<Vec<Vec<EthCallResponse>>>;

    /// Returns the base fee of the next block, with the `anvil_setNextBlockBaseFeePerGas`
    /// override.
    #[method(name = "baseFee")]
    async fn eth_base_fee(&self) -> RpcResult<Option<U256>>;

    /// Returns the fee history. The entry for the block after the newest one comes from that
    /// block when it exists, or from the next-block override, where reth computes it from the
    /// newest block alone; a block without a gas limit has a zero gas-used ratio instead of NaN.
    #[method(name = "feeHistory")]
    async fn eth_fee_history(
        &self,
        block_count: U64,
        newest_block: BlockNumberOrTag,
        reward_percentiles: Option<Vec<f64>>,
    ) -> RpcResult<FeeHistory>;

    /// Returns the receipt of a transaction. An impersonated transaction has no valid signature,
    /// so a lookup that recovers the sender from it fails; the lookup then runs again with the
    /// block in the RPC cache, which carries the senders the block recorded.
    #[method(name = "getTransactionReceipt")]
    async fn eth_get_transaction_receipt(&self, hash: B256) -> RpcResult<Option<Receipt>>;

    /// Returns a transaction by hash; see `eth_getTransactionReceipt` for impersonated
    /// transactions.
    #[method(name = "getTransactionByHash")]
    async fn eth_get_transaction_by_hash(&self, hash: B256) -> RpcResult<Option<Tx>>;

    /// Returns the transaction count of an account. At `pending`, the count comes from the pool
    /// and the latest state, as on anvil, without building reth's pending block, whose cache
    /// would then serve a block without the transactions that arrive in the next second.
    #[method(name = "getTransactionCount")]
    async fn eth_transaction_count(
        &self,
        address: Address,
        block: Option<BlockId>,
    ) -> RpcResult<U256>;

    /// Installs a log filter. A filter without `fromBlock` reports the blocks after the current
    /// one, as on anvil; reth's first poll includes the current block.
    #[method(name = "newFilter")]
    async fn eth_new_filter(&self, filter: Filter) -> RpcResult<FilterId>;

    /// Installs a block filter that reports the blocks after the current one, as on anvil.
    #[method(name = "newBlockFilter")]
    async fn eth_new_block_filter(&self) -> RpcResult<FilterId>;

    /// Returns the uncle count of a block, and fails for an unknown block, as anvil does; reth
    /// answers `null`.
    #[method(name = "getUncleCountByBlockHash")]
    async fn eth_block_uncles_count_by_hash(&self, hash: B256) -> RpcResult<Option<U256>>;

    /// Returns the uncle count of a block, and fails for a block above the head, as anvil does.
    #[method(name = "getUncleCountByBlockNumber")]
    async fn eth_block_uncles_count_by_number(
        &self,
        number: BlockNumberOrTag,
    ) -> RpcResult<Option<U256>>;

    /// Signs a transaction with a dev account. The chain id and the gas limit are filled in, as
    /// on anvil; reth requires them.
    #[method(name = "signTransaction")]
    async fn eth_sign_transaction(&self, request: TxReq) -> RpcResult<Bytes>;

    /// Signs typed data, like `eth_signTypedData`.
    #[method(name = "signTypedData_v4")]
    async fn eth_sign_typed_data_v4(&self, address: Address, data: TypedData) -> RpcResult<Bytes>;

    /// Sends a signed transaction and waits for its receipt, for `timeout_ms` at most.
    #[method(name = "sendRawTransactionSync")]
    async fn eth_send_raw_transaction_sync(
        &self,
        tx: Bytes,
        timeout_ms: Option<u64>,
    ) -> RpcResult<Receipt>;
}

/// The `web3_*` methods anvil replaces.
#[rpc(server, namespace = "web3")]
pub trait Web3ExtApi {
    /// Returns the client version, `reth-anvil/v<version>`.
    #[method(name = "clientVersion")]
    async fn web3_client_version(&self) -> RpcResult<String>;
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
    if request.from.is_some() {
        return overrides;
    }
    let mut overrides = overrides.unwrap_or_default();
    overrides.entry(Address::ZERO).or_default().balance.get_or_insert(U256::from(u128::MAX));
    Some(overrides)
}

/// Gives a revert without data the empty data anvil reports, where reth leaves it out.
fn with_revert_data(error: ErrorObjectOwned) -> ErrorObjectOwned {
    if error.code() == REVERT_ERROR_CODE && error.data().is_none() {
        return ErrorObjectOwned::owned(error.code(), error.message().to_string(), Some("0x"));
    }
    error
}

/// The error code of a reverted call, as anvil and reth report it.
const REVERT_ERROR_CODE: i32 = 3;

/// Reth's error for a transaction whose sender cannot be recovered.
const INVALID_SIGNATURE_MESSAGE: &str = "invalid transaction signature";

/// The error of a sender that cannot pay for a transaction.
fn insufficient_funds(cost: U256, balance: U256) -> ErrorObjectOwned {
    EthApiError::InvalidTransaction(RpcInvalidTransactionError::InsufficientFunds { cost, balance })
        .into()
}

/// How long a snapshot revert waits for the pool to take the reverted transactions back.
const POOL_RESTORE_TIMEOUT: Duration = Duration::from_secs(5);

/// How long `eth_sendTransactionSync` waits for the receipt.
const TRANSACTION_CONFIRMATION_TIMEOUT: Duration = Duration::from_secs(30);

/// The error code of a transaction confirmation timeout, as anvil reports it.
const TRANSACTION_CONFIRMATION_TIMEOUT_CODE: i32 = 4;

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
    enforce_tx_gas_limit: bool,
    fork: Option<Arc<dyn ForkInfo>>,
    pool: Pool,
    provider: Provider,
    eth: Eth,
    /// One lock per sender for requests without a nonce, so concurrent requests get distinct
    /// nonces.
    nonce_locks: Arc<Mutex<HashMap<Address, Arc<AsyncMutex<()>>>>>,
    /// Installs a log filter that reports the blocks after the current one; see
    /// `eth_newFilter`.
    new_filter: NewFilterHook,
}

/// Installs a log filter, or a block filter for `None`, and returns its id.
#[derive(Clone)]
pub struct NewFilterHook(
    Arc<dyn Fn(Option<Filter>) -> HookFuture<RpcResult<FilterId>> + Send + Sync>,
);

impl fmt::Debug for NewFilterHook {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NewFilterHook")
    }
}

impl NewFilterHook {
    /// Wraps the given installer.
    pub fn new(
        install: impl Fn(Option<Filter>) -> HookFuture<RpcResult<FilterId>> + Send + Sync + 'static,
    ) -> Self {
        Self(Arc::new(install))
    }
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
        enforce_tx_gas_limit: bool,
        fork: Option<Arc<dyn ForkInfo>>,
        pool: Pool,
        provider: Provider,
        eth: Eth,
        new_filter: NewFilterHook,
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
            enforce_tx_gas_limit,
            fork,
            pool,
            provider,
            eth,
            nonce_locks: Default::default(),
            new_filter,
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
    Pool: TransactionPool,
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
        self.rewind_to_keeping(header, &HashSet::new()).await?;
        Ok(())
    }

    /// Rewinds the chain to `header`. The transactions of the removed blocks are dropped, so the
    /// pool does not take them back, except the ones in `keep`, whose hashes are returned.
    async fn rewind_to_keeping(
        &self,
        header: &SealedHeader<HeaderOf<Provider>>,
        keep: &HashSet<B256>,
    ) -> RpcResult<Vec<B256>> {
        let best = self.best_block_number()?;
        let mut kept = Vec::new();
        if header.number() < best {
            let removed = self
                .provider
                .transactions_by_block_range(header.number() + 1..=best)
                .map_err(|error| internal_error(format!("failed to read transactions: {error}")))?;
            let (keep_hashes, drop_hashes): (Vec<_>, Vec<_>) = removed
                .into_iter()
                .flatten()
                .map(|tx| *tx.tx_hash())
                .partition(|hash| keep.contains(hash));
            kept = keep_hashes;
            self.impersonation.drop_txs(drop_hashes);
        }
        self.mining.rewind(header.clone()).await.map_err(internal_error)?;
        self.state.write().rewind_to(header.number());
        Ok(kept)
    }

    /// Brings the pool back to the transactions in `keep` after a rewind to `head`: waits for
    /// the pool to take the removed blocks' transactions in `restored` back, and removes the
    /// transactions that are not in `keep`.
    async fn restore_pool(&self, head: B256, keep: &HashSet<B256>, restored: &[B256]) {
        wait_for_pool(&self.pool, head).await;
        let deadline = Instant::now() + POOL_RESTORE_TIMEOUT;
        while restored.iter().any(|hash| !self.pool.contains(hash)) && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        let extra: Vec<B256> = self
            .pool
            .all_transaction_hashes()
            .into_iter()
            .filter(|hash| !keep.contains(hash))
            .collect();
        if !extra.is_empty() {
            self.pool.remove_transactions(extra.clone());
            self.impersonation.forget_tx_senders(extra);
        }
    }

    /// Tells the pool the nonce and balance of an account after a state write, so it promotes or
    /// parks the account's transactions, as anvil's pool sees the write at once.
    fn sync_pool_account(&self, address: Address) -> RpcResult<()>
    where
        Pool: TransactionPoolExt,
    {
        let account = self
            .provider
            .latest()
            .and_then(|state| state.basic_account(&address))
            .map_err(|error| internal_error(format!("failed to read account: {error}")))?
            .unwrap_or_default();
        self.pool.update_accounts(vec![ChangedAccount {
            address,
            nonce: account.nonce,
            balance: account.balance,
        }]);
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

    /// Returns the lowercase name of the latest hardfork active at the given block.
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
    Pool: TransactionPool + TransactionPoolExt + Send + Sync + 'static,
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
        if enabled {
            self.mining.trigger_if_pending();
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
            pool: self.pool.all_transaction_hashes(),
        };
        Ok(self.snapshots.insert(snapshot))
    }

    async fn anvil_revert(&self, id: U256) -> RpcResult<bool> {
        let Some(snapshot) = self.snapshots.take(id) else {
            return Ok(false);
        };
        // The pool goes back to the snapshot too, as on anvil: the transactions mined since
        // return to it, the ones sent since go.
        let keep: HashSet<B256> = snapshot.pool.iter().copied().collect();
        let restored = self.rewind_to_keeping(&snapshot.header, &keep).await?;
        *self.state.write() = snapshot.state;
        self.time.restore(snapshot.time);
        self.block_env.restore(snapshot.block_env);
        self.restore_pool(snapshot.header.hash(), &keep, &restored).await;
        if self.mining.is_automine() {
            self.mining.trigger_if_pending();
        }
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
        // The first block keeps the genesis base fee, as at launch.
        if let Some(base_fee) = genesis.base_fee_per_gas() {
            self.block_env.set_next_base_fee(base_fee);
        }
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
        let Some(tx) = self.pool.get(&tx_hash) else {
            return Ok(None);
        };
        // The sender's later transactions go with it, as on anvil; reth only parks them.
        let (sender, nonce) = (tx.sender(), tx.nonce());
        let mut hashes = vec![tx_hash];
        hashes.extend(
            self.pool
                .get_transactions_by_sender(sender)
                .into_iter()
                .filter(|tx| tx.nonce() > nonce)
                .map(|tx| *tx.hash()),
        );
        self.pool.remove_transactions(hashes.clone());
        self.impersonation.forget_tx_senders(hashes);
        Ok(Some(tx_hash))
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
        self.sync_pool_account(address)
    }

    async fn anvil_add_balance(&self, address: Address, balance: U256) -> RpcResult<()> {
        let current = self.latest_balance(address)?;
        self.state.write().set_balance(address, current.saturating_add(balance));
        Ok(())
    }

    async fn anvil_set_nonce(&self, address: Address, nonce: U256) -> RpcResult<()> {
        let nonce = nonce.try_into().map_err(|_| invalid_params("nonce exceeds u64::MAX"))?;
        self.state.write().set_nonce(address, nonce);
        self.sync_pool_account(address)
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
    Pool: TransactionPool + TransactionPoolExt + Send + Sync + 'static,
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

impl<Pool, Provider, Eth, Spec> AnvilRpc<Pool, Provider, Eth, Spec>
where
    Pool: TransactionPool,
    Provider: BlockNumReader + HeaderProvider + StateProviderFactory,
    Eth: FullEthApiServer,
    Spec: EthChainSpec + EthereumHardforks,
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

    /// Estimates the gas of a call down to the exact limit; see `eth_estimateGas`.
    async fn estimate_gas_exact(
        &self,
        request: RpcTxReq<Eth::NetworkTypes>,
        block: Option<BlockId>,
        state_overrides: Option<StateOverride>,
        block_overrides: Option<Box<BlockOverrides>>,
    ) -> RpcResult<U256> {
        let request = self.with_call_fees(request)?;
        // Anvil checks the value against the balance before it runs the call.
        if let Some(from) = request.as_ref().from
            && let Some(value) = request.as_ref().value
            && !value.is_zero()
        {
            let balance = self.balance_of(from, block, state_overrides.as_ref())?;
            if value > balance {
                return Err(insufficient_funds(value, balance));
            }
        }
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

    /// Prepares a request for `eth_sendTransaction`: the sender and the recipient as
    /// [`Self::with_sender`], a gas limit, and fees. A request without a gas limit gets the
    /// estimate, or, when the estimate fails because the call reverts, the largest limit a
    /// transaction may have, so the transaction is mined and reverts, as on anvil. A request
    /// without fees gets the ones reth fills, or the gas price before London.
    async fn prepare_send(
        &self,
        request: RpcTxReq<Eth::NetworkTypes>,
    ) -> RpcResult<RpcTxReq<Eth::NetworkTypes>> {
        let mut request = self.with_sender(request)?;
        if request.as_ref().gas.is_none() {
            let gas = match self.estimate_gas_exact(request.clone(), None, None, None).await {
                Ok(gas) => gas.saturating_to(),
                Err(_) => self.fallback_gas_limit()?,
            };
            request.as_mut().gas = Some(gas);
        }
        if request.as_ref().gas_price.is_none() && request.as_ref().max_fee_per_gas.is_none() {
            match self.sealed_header(self.best_block_number()?)?.base_fee_per_gas() {
                Some(base_fee) => {
                    let tip = match request.as_ref().max_priority_fee_per_gas {
                        Some(tip) => tip,
                        None => EthApiServer::max_priority_fee_per_gas(&self.eth).await?.to(),
                    };
                    let tx = request.as_mut();
                    tx.max_priority_fee_per_gas = Some(tip);
                    tx.max_fee_per_gas = Some(u128::from(base_fee) * 2 + tip);
                }
                None => request.as_mut().gas_price = Some(self.gas_price().await?.to()),
            }
        }
        Ok(request)
    }

    /// Returns the gas price: the base fee plus the suggested tip, the base fee alone when the
    /// minimum priority fee is disabled, or the node's gas price before London, as anvil does.
    async fn gas_price(&self) -> RpcResult<U256> {
        let base_fee = self.sealed_header(self.best_block_number()?)?.base_fee_per_gas();
        match base_fee {
            // Before London, the node's gas price, as `anvil_setMinGasPrice` sets it.
            None if let Some(gas_price) = self.block_env.gas_price() => Ok(U256::from(gas_price)),
            Some(base_fee) if !self.min_priority_fee_enforced => Ok(U256::from(base_fee)),
            _ => EthApiServer::gas_price(&self.eth).await,
        }
    }

    /// Sends a transaction from a dev or impersonated account; see `eth_sendTransaction`. The
    /// sender's lock is held from the nonce selection to the pool insertion, so concurrent
    /// requests get distinct nonces and a replacement is checked against the pool it meets.
    async fn send(&self, request: RpcTxReq<Eth::NetworkTypes>) -> RpcResult<B256> {
        let request = self.with_sender(request)?;
        let _guard = self.nonce_lock(request.as_ref().from.unwrap_or_default()).lock_owned().await;
        let request = self.prepare_send(request).await?;
        if let Some(max_fee) = request.as_ref().max_fee_per_gas.or(request.as_ref().gas_price) {
            self.ensure_fee_cap(max_fee)?;
        }
        self.ensure_request_replacement_priced(&request)?;
        EthApiServer::send_transaction(&self.eth, request).await
    }

    /// Sends a signed transaction; see `eth_sendRawTransaction`. The sender's lock is held
    /// through the pool insertion, as in [`Self::send`]. A transaction reth cannot decode fails
    /// in reth's handler with reth's error.
    async fn send_raw(&self, tx: Bytes) -> RpcResult<B256> {
        let recovered = recover_raw_transaction::<PooledTransaction>(&tx).ok();
        let _guard = match &recovered {
            Some(recovered) => Some(self.nonce_lock(recovered.signer()).lock_owned().await),
            None => None,
        };
        if let Some(recovered) = &recovered {
            self.ensure_fee_cap(recovered.max_fee_per_gas())?;
            self.ensure_replacement_priced(
                recovered.signer(),
                recovered.nonce(),
                recovered.max_fee_per_gas(),
            )?;
        }
        EthApiServer::send_raw_transaction(&self.eth, tx).await
    }

    /// Returns whether a lookup by transaction hash that failed with `error` should run again:
    /// the lookup failed to recover the sender from the signature, and the transaction's block
    /// is now in the RPC cache. An impersonated transaction has no valid signature, and the
    /// cache carries the senders the block recorded.
    async fn cache_block_of(&self, hash: B256, error: &ErrorObjectOwned) -> RpcResult<bool>
    where
        Provider: TransactionsProvider,
    {
        if error.message() != INVALID_SIGNATURE_MESSAGE {
            return Ok(false);
        }
        let Some((_, meta)) = self
            .provider
            .transaction_by_hash_with_meta(hash)
            .map_err(|error| internal_error(format!("failed to read transaction: {error}")))?
        else {
            return Ok(false);
        };
        self.eth
            .cache()
            .get_recovered_block(meta.block_hash)
            .await
            .map_err(|error| internal_error(format!("failed to read block: {error}")))?;
        Ok(true)
    }

    /// Returns the nonce lock of a sender.
    fn nonce_lock(&self, sender: Address) -> Arc<AsyncMutex<()>> {
        self.nonce_locks.lock().entry(sender).or_default().clone()
    }

    /// Returns the balance of an account in a block, with the state override applied.
    fn balance_of(
        &self,
        address: Address,
        block: Option<BlockId>,
        state_overrides: Option<&StateOverride>,
    ) -> RpcResult<U256> {
        if let Some(AccountOverride { balance: Some(balance), .. }) =
            state_overrides.and_then(|overrides| overrides.get(&address))
        {
            return Ok(*balance);
        }
        let state = self
            .provider
            .state_by_block_id(block.unwrap_or_default())
            .map_err(|error| internal_error(format!("failed to read state: {error}")))?;
        let balance = state
            .account_balance(&address)
            .map_err(|error| internal_error(format!("failed to read account: {error}")))?;
        Ok(balance.unwrap_or_default())
    }

    /// Returns the base fee of the next block: the override, or the fee the latest block gives
    /// it. `None` before London.
    fn next_base_fee(&self) -> RpcResult<Option<u64>> {
        if let Some(fee) = self.block_env.building_base_fee().or(self.block_env.next_base_fee()) {
            return Ok(Some(fee));
        }
        let header = self.sealed_header(self.best_block_number()?)?;
        let params =
            self.chain_spec.base_fee_params_at_timestamp(self.time.current_call_timestamp());
        Ok(header.next_block_base_fee(params))
    }

    /// Rejects a transaction whose fee cap is below the next block's base fee, as anvil does;
    /// reth parks it in the pool until the base fee drops.
    fn ensure_fee_cap(&self, max_fee: u128) -> RpcResult<()> {
        if let Some(base_fee) = self.next_base_fee()?
            && max_fee < u128::from(base_fee)
        {
            return Err(
                EthApiError::InvalidTransaction(RpcInvalidTransactionError::FeeCapTooLow).into()
            );
        }
        Ok(())
    }

    /// Fails a priced call whose sender cannot pay for its gas and value, as anvil does; reth
    /// runs calls without the balance check. A call without a gas limit pays for at least a
    /// transfer.
    fn ensure_call_funds(
        &self,
        request: &RpcTxReq<Eth::NetworkTypes>,
        block: Option<BlockId>,
        state_overrides: Option<&StateOverride>,
    ) -> RpcResult<()> {
        let tx = request.as_ref();
        let (Some(from), Some(price)) = (tx.from, tx.gas_price.or(tx.max_fee_per_gas)) else {
            return Ok(());
        };
        if price == 0 {
            return Ok(());
        }
        let balance = self.balance_of(from, block, state_overrides)?;
        let gas = tx.gas.unwrap_or(MIN_TRANSACTION_GAS);
        let cost = U256::from(price) * U256::from(gas) + tx.value.unwrap_or_default();
        if balance < cost {
            return Err(insufficient_funds(cost, balance));
        }
        Ok(())
    }

    /// Rejects a transaction whose fee does not exceed the pooled transaction with the same
    /// sender and nonce, as anvil does; reth replaces at an equal fee.
    fn ensure_replacement_priced(
        &self,
        sender: Address,
        nonce: u64,
        max_fee: u128,
    ) -> RpcResult<()> {
        if let Some(existing) = self.pool.get_transaction_by_sender_and_nonce(sender, nonce)
            && max_fee <= existing.transaction.max_fee_per_gas()
        {
            return Err(EthApiError::PoolError(RpcPoolError::ReplaceUnderpriced).into());
        }
        Ok(())
    }

    /// [`Self::ensure_replacement_priced`] for a request with an explicit nonce and fee.
    fn ensure_request_replacement_priced(
        &self,
        request: &RpcTxReq<Eth::NetworkTypes>,
    ) -> RpcResult<()> {
        let tx = request.as_ref();
        if let (Some(from), Some(nonce), Some(max_fee)) =
            (tx.from, tx.nonce, tx.max_fee_per_gas.or(tx.gas_price))
        {
            self.ensure_replacement_priced(from, nonce, max_fee)?;
        }
        Ok(())
    }

    /// Returns the largest gas limit a transaction may have: the block gas limit, capped by
    /// EIP-7825 when the cap is enforced and Osaka is active.
    fn fallback_gas_limit(&self) -> RpcResult<u64> {
        let header = self.sealed_header(self.best_block_number()?)?;
        let mut limit = header.gas_limit();
        if self.enforce_tx_gas_limit
            && self.chain_spec.is_osaka_active_at_timestamp(header.timestamp())
        {
            limit = limit.min(TX_GAS_LIMIT_CAP);
        }
        Ok(limit)
    }

    /// Drops fee fields below the base fee from a call, so the call runs instead of failing
    /// the fee check; anvil runs calls with the base fee check off.
    fn with_call_fees(
        &self,
        mut request: RpcTxReq<Eth::NetworkTypes>,
    ) -> RpcResult<RpcTxReq<Eth::NetworkTypes>> {
        let Some(base_fee) = self.sealed_header(self.best_block_number()?)?.base_fee_per_gas()
        else {
            return Ok(request);
        };
        let fees = request.as_mut();
        let below = |fee: Option<u128>| fee.is_some_and(|fee| fee < u128::from(base_fee));
        if below(fees.gas_price) || below(fees.max_fee_per_gas) {
            fees.gas_price = None;
            fees.max_fee_per_gas = None;
            fees.max_priority_fee_per_gas = None;
        }
        Ok(request)
    }

    /// Waits for the receipt of a transaction. Anvil reports a timeout with code 4 and the
    /// transaction hash as data.
    async fn await_receipt(
        &self,
        hash: B256,
        timeout: Duration,
    ) -> RpcResult<RpcReceipt<Eth::NetworkTypes>> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(receipt) = EthApiServer::transaction_receipt(&self.eth, hash).await? {
                return Ok(receipt);
            }
            if Instant::now() >= deadline {
                return Err(ErrorObjectOwned::owned(
                    TRANSACTION_CONFIRMATION_TIMEOUT_CODE,
                    "Transaction confirmation timeout",
                    Some(hash),
                ));
            }
            tokio::time::sleep(RECEIPT_POLL_INTERVAL).await;
        }
    }
}

#[async_trait]
impl<Pool, Provider, Eth, Spec>
    EthExtApiServer<
        RpcTxReq<Eth::NetworkTypes>,
        RpcReceipt<Eth::NetworkTypes>,
        RpcTransaction<Eth::NetworkTypes>,
    > for AnvilRpc<Pool, Provider, Eth, Spec>
where
    Pool: TransactionPool + 'static,
    Provider: BlockNumReader
        + HeaderProvider
        + StateProviderFactory
        + TransactionsProvider
        + Send
        + Sync
        + 'static,
    Eth: FullEthApiServer,
    Spec: EthChainSpec + EthereumHardforks + Send + Sync + 'static,
{
    async fn eth_send_transaction(&self, request: RpcTxReq<Eth::NetworkTypes>) -> RpcResult<B256> {
        self.send(request).await
    }

    async fn eth_send_transaction_sync(
        &self,
        request: RpcTxReq<Eth::NetworkTypes>,
    ) -> RpcResult<RpcReceipt<Eth::NetworkTypes>> {
        let hash = self.send(request).await?;
        self.await_receipt(hash, TRANSACTION_CONFIRMATION_TIMEOUT).await
    }

    async fn eth_resend(
        &self,
        request: RpcTxReq<Eth::NetworkTypes>,
        gas_price: Option<U256>,
        gas_limit: Option<U64>,
    ) -> RpcResult<B256> {
        let Some(nonce) = request.as_ref().nonce else {
            return Err(invalid_params("missing transaction nonce in transaction spec"));
        };
        let mut request = self.with_sender(request)?;
        let from = request.as_ref().from.unwrap_or_default();
        if self.pool.get_transaction_by_sender_and_nonce(from, nonce).is_none() {
            return Err(invalid_params("transaction not found"));
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
        self.ensure_request_replacement_priced(&request)?;
        EthApiServer::send_transaction(&self.eth, request).await
    }

    async fn eth_send_raw_transaction(&self, tx: Bytes) -> RpcResult<B256> {
        self.send_raw(tx).await
    }

    async fn eth_send_raw_transaction_conditional(
        &self,
        tx: Bytes,
        _condition: TransactionConditional,
    ) -> RpcResult<B256> {
        self.send_raw(tx).await
    }

    async fn eth_request_accounts(&self) -> RpcResult<Vec<Address>> {
        EthApiServer::accounts(&self.eth)
    }

    async fn eth_network_id(&self) -> RpcResult<Option<String>> {
        Ok(EthApiServer::chain_id(&self.eth).await?.map(|id| id.to::<u64>().to_string()))
    }

    async fn eth_gas_price(&self) -> RpcResult<U256> {
        self.gas_price().await
    }

    async fn eth_estimate_gas(
        &self,
        request: RpcTxReq<Eth::NetworkTypes>,
        block: Option<BlockId>,
        state_overrides: Option<StateOverride>,
        block_overrides: Option<Box<BlockOverrides>>,
    ) -> RpcResult<U256> {
        self.estimate_gas_exact(request, block, state_overrides, block_overrides)
            .await
            .map_err(with_revert_data)
    }

    async fn eth_call(
        &self,
        request: RpcTxReq<Eth::NetworkTypes>,
        block: Option<BlockId>,
        state_overrides: Option<StateOverride>,
        block_overrides: Option<Box<BlockOverrides>>,
    ) -> RpcResult<Bytes> {
        self.ensure_call_funds(&request, block, state_overrides.as_ref())?;
        let request = self.with_call_fees(request)?;
        EthApiServer::call(&self.eth, request, block, state_overrides, block_overrides)
            .await
            .map_err(with_revert_data)
    }

    async fn eth_call_many(
        &self,
        bundles: Vec<Bundle<RpcTxReq<Eth::NetworkTypes>>>,
        state_context: Option<StateContext>,
        state_override: Option<StateOverride>,
    ) -> RpcResult<Vec<Vec<EthCallResponse>>> {
        // Reth skips empty bundles and runs every bundle in the same block. Anvil answers every
        // bundle and moves each one a block and a second past the one before, from the block
        // the first bundle names or the pending block.
        let latest = self.sealed_header(self.best_block_number()?)?;
        let mut number = U256::from(latest.number() + 1);
        let mut time = latest.timestamp() + 1;
        let mut requests = Vec::with_capacity(bundles.len());
        let mut layout = Vec::with_capacity(bundles.len());
        for Bundle { transactions, block_override } in bundles {
            let mut block_override = block_override.unwrap_or_default();
            number = block_override.number.unwrap_or(number);
            time = block_override.time.unwrap_or(time);
            block_override.number = Some(number);
            block_override.time = Some(time);
            number += U256::ONE;
            time += 1;
            layout.push(transactions.is_empty());
            if !transactions.is_empty() {
                let transactions = transactions
                    .into_iter()
                    .map(|request| self.with_call_fees(request))
                    .collect::<RpcResult<Vec<_>>>()?;
                requests.push(Bundle { transactions, block_override: Some(block_override) });
            }
        }
        let mut results =
            EthApiServer::call_many(&self.eth, requests, state_context, state_override)
                .await?
                .into_iter();
        Ok(layout
            .into_iter()
            .map(|empty| if empty { Vec::new() } else { results.next().unwrap_or_default() })
            .collect())
    }

    async fn eth_base_fee(&self) -> RpcResult<Option<U256>> {
        Ok(self.next_base_fee()?.map(U256::from))
    }

    async fn eth_transaction_count(
        &self,
        address: Address,
        block: Option<BlockId>,
    ) -> RpcResult<U256> {
        if !block.is_some_and(|block| block.is_pending()) {
            return EthApiServer::transaction_count(&self.eth, address, block).await;
        }
        let latest =
            EthApiServer::transaction_count(&self.eth, address, Some(BlockId::latest())).await?;
        let pooled = self
            .pool
            .get_highest_transaction_by_sender(address)
            .map(|tx| U256::from(tx.nonce() + 1))
            .unwrap_or_default();
        Ok(latest.max(pooled))
    }

    async fn eth_new_filter(&self, filter: Filter) -> RpcResult<FilterId> {
        (self.new_filter.0)(Some(filter)).await
    }

    async fn eth_new_block_filter(&self) -> RpcResult<FilterId> {
        (self.new_filter.0)(None).await
    }

    async fn eth_block_uncles_count_by_hash(&self, hash: B256) -> RpcResult<Option<U256>> {
        let known = self
            .provider
            .block_number(hash)
            .map_err(|error| internal_error(format!("failed to read block: {error}")))?;
        if known.is_none() {
            return Err(EthApiError::HeaderNotFound(hash.into()).into());
        }
        EthApiServer::block_uncles_count_by_hash(&self.eth, hash).await
    }

    async fn eth_block_uncles_count_by_number(
        &self,
        number: BlockNumberOrTag,
    ) -> RpcResult<Option<U256>> {
        if let BlockNumberOrTag::Number(number) = number
            && number > self.best_block_number()?
        {
            return Err(EthApiError::HeaderNotFound(number.into()).into());
        }
        EthApiServer::block_uncles_count_by_number(&self.eth, number).await
    }

    async fn eth_sign_transaction(&self, request: RpcTxReq<Eth::NetworkTypes>) -> RpcResult<Bytes> {
        let mut request = self.with_sender(request)?;
        if request.as_ref().chain_id.is_none() {
            request.as_mut().chain_id = EthApiServer::chain_id(&self.eth).await?.map(|id| id.to());
        }
        if request.as_ref().gas.is_none() {
            // The estimate runs without the fee fields, which the signed transaction keeps as
            // given, so a tip above the fee cap does not fail it.
            let mut probe = request.clone();
            let fees = probe.as_mut();
            fees.gas_price = None;
            fees.max_fee_per_gas = None;
            fees.max_priority_fee_per_gas = None;
            let gas = match self.estimate_gas_exact(probe, None, None, None).await {
                Ok(gas) => gas.saturating_to(),
                Err(_) => self.fallback_gas_limit()?,
            };
            request.as_mut().gas = Some(gas);
        }
        EthApiServer::sign_transaction(&self.eth, request).await
    }

    async fn eth_sign_typed_data_v4(&self, address: Address, data: TypedData) -> RpcResult<Bytes> {
        EthApiServer::sign_typed_data(&self.eth, address, data).await
    }

    async fn eth_get_transaction_receipt(
        &self,
        hash: B256,
    ) -> RpcResult<Option<RpcReceipt<Eth::NetworkTypes>>> {
        match EthApiServer::transaction_receipt(&self.eth, hash).await {
            Err(error) if self.cache_block_of(hash, &error).await? => {
                EthApiServer::transaction_receipt(&self.eth, hash).await
            }
            result => result,
        }
    }

    async fn eth_get_transaction_by_hash(
        &self,
        hash: B256,
    ) -> RpcResult<Option<RpcTransaction<Eth::NetworkTypes>>> {
        match EthApiServer::transaction_by_hash(&self.eth, hash).await {
            Err(error) if self.cache_block_of(hash, &error).await? => {
                EthApiServer::transaction_by_hash(&self.eth, hash).await
            }
            result => result,
        }
    }

    async fn eth_fee_history(
        &self,
        block_count: U64,
        newest_block: BlockNumberOrTag,
        reward_percentiles: Option<Vec<f64>>,
    ) -> RpcResult<FeeHistory> {
        let mut history =
            EthApiServer::fee_history(&self.eth, block_count, newest_block, reward_percentiles)
                .await?;
        let blocks = history.base_fee_per_gas.len().saturating_sub(1) as u64;
        if blocks == 0 {
            return Ok(history);
        }
        let newest = history.oldest_block + blocks - 1;
        let child = self
            .provider
            .sealed_header(newest + 1)
            .map_err(|error| internal_error(format!("failed to read header: {error}")))?;
        let (next_base_fee, next_blob_fee) = match child {
            Some(child) => {
                let blob_params = self.chain_spec.blob_params_at_timestamp(child.timestamp());
                (
                    child.base_fee_per_gas().map(u128::from),
                    blob_params.and_then(|params| child.blob_fee(params)),
                )
            }
            None => (self.next_base_fee()?.map(u128::from), None),
        };
        if let Some(fee) = next_base_fee
            && let Some(last) = history.base_fee_per_gas.last_mut()
        {
            *last = fee;
        }
        if let Some(fee) = next_blob_fee
            && let Some(last) = history.base_fee_per_blob_gas.last_mut()
        {
            *last = fee;
        }
        for ratio in history.gas_used_ratio.iter_mut().chain(history.blob_gas_used_ratio.iter_mut())
        {
            if ratio.is_nan() {
                *ratio = 0.0;
            }
        }
        Ok(history)
    }

    async fn eth_send_raw_transaction_sync(
        &self,
        tx: Bytes,
        timeout_ms: Option<u64>,
    ) -> RpcResult<RpcReceipt<Eth::NetworkTypes>> {
        let hash = self.send_raw(tx).await?;
        let timeout = timeout_ms.map_or(TRANSACTION_CONFIRMATION_TIMEOUT, Duration::from_millis);
        self.await_receipt(hash, timeout).await
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

#[async_trait]
impl<Pool, Provider, Eth, Spec> Web3ExtApiServer for AnvilRpc<Pool, Provider, Eth, Spec>
where
    Pool: Send + Sync + 'static,
    Provider: HeaderProvider + Send + Sync + 'static,
    Eth: Send + Sync + 'static,
    Spec: Send + Sync + 'static,
{
    async fn web3_client_version(&self) -> RpcResult<String> {
        Ok(CLIENT_VERSION.to_string())
    }
}
