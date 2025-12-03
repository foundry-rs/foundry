//! In-memory blockchain backend.

use self::state::trie_storage;
use super::executor::new_evm_with_inspector_ref;
use crate::{
    ForkChoice, NodeConfig, PrecompileFactory,
    config::PruneStateHistoryConfig,
    eth::{
        backend::{
            cheats::{CheatEcrecover, CheatsManager},
            db::{Db, MaybeFullDatabase, SerializableState, StateDb},
            env::Env,
            executor::{ExecutedTransactions, TransactionExecutor},
            fork::ClientFork,
            genesis::GenesisConfig,
            mem::{
                state::{storage_root, trie_accounts},
                storage::MinedTransactionReceipt,
            },
            notifications::{NewBlockNotification, NewBlockNotifications},
            time::{TimeManager, utc_from_secs},
            validate::TransactionValidator,
        },
        error::{BlockchainError, ErrDetail, InvalidTransactionError},
        fees::{FeeDetails, FeeManager, MIN_SUGGESTED_PRIORITY_FEE},
        macros::node_info,
        pool::transactions::PoolTransaction,
        sign::build_typed_transaction,
    },
    mem::{
        inspector::AnvilInspector,
        storage::{BlockchainStorage, InMemoryBlockStates, MinedBlockOutcome},
    },
};
use alloy_chains::NamedChain;
use alloy_consensus::{
    Account, Blob, BlockHeader, EnvKzgSettings, Header, Receipt, ReceiptWithBloom, Signed,
    Transaction as TransactionTrait, TxEnvelope, Typed2718,
    proofs::{calculate_receipt_root, calculate_transaction_root},
    transaction::Recovered,
};
use alloy_eip5792::{Capabilities, DelegationCapability};
use alloy_eips::{
    Encodable2718,
    eip1559::BaseFeeParams,
    eip4844::{BlobTransactionSidecar, kzg_to_versioned_hash},
    eip7840::BlobParams,
    eip7910::SystemContract,
};
use alloy_evm::{
    Database, Evm, FromRecoveredTx,
    eth::EthEvmContext,
    overrides::{OverrideBlockHashes, apply_state_overrides},
    precompiles::{DynPrecompile, Precompile, PrecompilesMap},
};
use alloy_network::{
    AnyHeader, AnyRpcBlock, AnyRpcHeader, AnyRpcTransaction, AnyTxEnvelope, AnyTxType,
    EthereumWallet, UnknownTxEnvelope, UnknownTypedTransaction,
};
use alloy_primitives::{
    Address, B256, Bytes, TxHash, TxKind, U64, U256, address, hex, keccak256, logs_bloom,
    map::HashMap,
};
use alloy_rpc_types::{
    AccessList, Block as AlloyBlock, BlockId, BlockNumberOrTag as BlockNumber, BlockTransactions,
    EIP1186AccountProofResponse as AccountProof, EIP1186StorageProof as StorageProof, Filter,
    Header as AlloyHeader, Index, Log, Transaction, TransactionReceipt,
    anvil::Forking,
    request::TransactionRequest,
    serde_helpers::JsonStorageKey,
    simulate::{SimBlock, SimCallResult, SimulatePayload, SimulatedBlock},
    state::EvmOverrides,
    trace::{
        filter::TraceFilter,
        geth::{
            FourByteFrame, GethDebugBuiltInTracerType, GethDebugTracerType,
            GethDebugTracingCallOptions, GethDebugTracingOptions, GethTrace, NoopFrame,
            TraceResult,
        },
        parity::LocalizedTransactionTrace,
    },
};
use alloy_serde::{OtherFields, WithOtherFields};
use alloy_signer::Signature;
use alloy_signer_local::PrivateKeySigner;
use alloy_trie::{HashBuilder, Nibbles, proof::ProofRetainer};
use anvil_core::eth::{
    block::{Block, BlockInfo},
    transaction::{
        MaybeImpersonatedTransaction, PendingTransaction, ReceiptResponse, TransactionInfo,
        TypedReceipt, TypedReceiptRpc, has_optimism_fields, transaction_request_to_typed,
    },
    wallet::WalletCapabilities,
};
use anvil_rpc::error::RpcError;
use chrono::Datelike;
use eyre::{Context, Result};
use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use foundry_evm::{
    backend::{DatabaseError, DatabaseResult, RevertStateSnapshotAction},
    constants::DEFAULT_CREATE2_DEPLOYER_RUNTIME_CODE,
    core::{either_evm::EitherEvm, precompiles::EC_RECOVER},
    decode::RevertDecoder,
    inspectors::AccessListInspector,
    traces::{
        CallTraceDecoder, FourByteInspector, GethTraceBuilder, TracingInspector,
        TracingInspectorConfig,
    },
    utils::{get_blob_base_fee_update_fraction, get_blob_base_fee_update_fraction_by_spec_id},
};
use foundry_primitives::FoundryTxEnvelope;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use op_alloy_consensus::DEPOSIT_TX_TYPE_ID;
use op_revm::{
    OpContext, OpHaltReason, OpTransaction, transaction::deposit::DepositTransactionParts,
};
use parking_lot::{Mutex, RwLock, RwLockUpgradableReadGuard};
use revm::{
    DatabaseCommit, Inspector,
    context::{Block as RevmBlock, BlockEnv, Cfg, TxEnv},
    context_interface::{
        block::BlobExcessGasAndPrice,
        result::{ExecutionResult, Output, ResultAndState},
    },
    database::{CacheDB, DbAccount, WrapDatabaseRef},
    interpreter::InstructionResult,
    precompile::{PrecompileSpecId, Precompiles},
    primitives::{KECCAK_EMPTY, hardfork::SpecId},
    state::AccountInfo,
};
use std::{
    collections::BTreeMap,
    fmt::Debug,
    io::{Read, Write},
    ops::{Mul, Not},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use storage::{Blockchain, DEFAULT_HISTORY_LIMIT, MinedTransaction};
use tokio::sync::RwLock as AsyncRwLock;

pub mod cache;
pub mod fork_db;
pub mod in_memory_db;
pub mod inspector;
pub mod offline_fork_db;
pub mod state;
pub mod storage;

/// Helper trait that combines revm::DatabaseRef with Debug.
/// This is needed because alloy-evm requires Debug on Database implementations.
/// With trait upcasting now stable, we can now upcast from this trait to revm::DatabaseRef.
pub trait DatabaseRef: revm::DatabaseRef<Error = DatabaseError> + Debug {}
impl<T> DatabaseRef for T where T: revm::DatabaseRef<Error = DatabaseError> + Debug {}
impl DatabaseRef for dyn crate::eth::backend::db::Db {}

// Gas per transaction not creating a contract.
pub const MIN_TRANSACTION_GAS: u128 = 21000;
// Gas per transaction creating a contract.
pub const MIN_CREATE_GAS: u128 = 53000;
// Executor
pub const EXECUTOR: Address = address!("0x6634F723546eCc92277e8a2F93d4f248bf1189ea");
pub const EXECUTOR_PK: &str = "0x502d47e1421cb9abef497096728e69f07543232b93ef24de4998e18b5fd9ba0f";
// Experimental ERC20
pub const EXP_ERC20_CONTRACT: Address = address!("0x238c8CD93ee9F8c7Edf395548eF60c0d2e46665E");
// Runtime code of the experimental ERC20 contract
pub const EXP_ERC20_RUNTIME_CODE: &[u8] = &hex!(
    "60806040526004361015610010575b005b5f3560e01c806306fdde03146106f7578063095ea7b31461068c57806318160ddd1461066757806323b872dd146105a15780632bb7c5951461050e578063313ce567146104f35780633644e5151461045557806340c10f191461043057806370a08231146103fe5780637ecebe00146103cc57806395d89b4114610366578063a9059cbb146102ea578063ad0c8fdd146102ad578063d505accf146100fb5763dd62ed3e0361000e57346100f75760403660031901126100f7576100d261075c565b6100da610772565b602052637f5e9f20600c525f5260206034600c2054604051908152f35b5f80fd5b346100f75760e03660031901126100f75761011461075c565b61011c610772565b6084359160643560443560ff851685036100f757610138610788565b60208101906e04578706572696d656e74455243323608c1b8252519020908242116102a0576040519360018060a01b03169460018060a01b03169565383775081901600e52855f5260c06020600c20958654957f8b73c3c69bb8fe3d512ecc4cf759cc79239f7b179b0ffacaa9a75d522b39400f8252602082019586528660408301967fc89efdaa54c0f20c7adf612882df0950f5a951637e0307cdcb4c672f298b8bc688528b6060850198468a528c608087019330855260a08820602e527f6e71edae12b1b97f4d1f60370fef10105fa2faae0126114a169c64845d6126c9885252528688525260a082015220604e526042602c205f5260ff1660205260a43560405260c43560605260208060805f60015afa93853d5103610293577f8c5be1e5ebec7d5bd14f71427d1e84f3dd0314c0f7b2291e5b200ac8c7c3b92594602094019055856303faf4f960a51b176040526034602c2055a3005b63ddafbaef5f526004601cfd5b631a15a3cc5f526004601cfd5b5f3660031901126100f7576103e834023481046103e814341517156102d65761000e90336107ac565b634e487b7160e01b5f52601160045260245ffd5b346100f75760403660031901126100f75761030361075c565b602435906387a211a2600c52335f526020600c2080548084116103595783900390555f526020600c20818154019055602052600c5160601c335f51602061080d5f395f51905f52602080a3602060405160018152f35b63f4d678b85f526004601cfd5b346100f7575f3660031901126100f757604051604081019080821067ffffffffffffffff8311176103b8576103b491604052600381526204558560ec1b602082015260405191829182610732565b0390f35b634e487b7160e01b5f52604160045260245ffd5b346100f75760203660031901126100f7576103e561075c565b6338377508600c525f52602080600c2054604051908152f35b346100f75760203660031901126100f75761041761075c565b6387a211a2600c525f52602080600c2054604051908152f35b346100f75760403660031901126100f75761000e61044c61075c565b602435906107ac565b346100f7575f3660031901126100f757602060a0610471610788565b828101906e04578706572696d656e74455243323608c1b8252519020604051907f8b73c3c69bb8fe3d512ecc4cf759cc79239f7b179b0ffacaa9a75d522b39400f8252838201527fc89efdaa54c0f20c7adf612882df0950f5a951637e0307cdcb4c672f298b8bc6604082015246606082015230608082015220604051908152f35b346100f7575f3660031901126100f757602060405160128152f35b346100f75760203660031901126100f7576004356387a211a2600c52335f526020600c2090815490818111610359575f80806103e88487839688039055806805345cdf77eb68f44c54036805345cdf77eb68f44c5580835282335f51602061080d5f395f51905f52602083a304818115610598575b3390f11561058d57005b6040513d5f823e3d90fd5b506108fc610583565b346100f75760603660031901126100f7576105ba61075c565b6105c2610772565b604435908260601b33602052637f5e9f208117600c526034600c20908154918219610643575b506387a211a2915017600c526020600c2080548084116103595783900390555f526020600c20818154019055602052600c5160601c9060018060a01b03165f51602061080d5f395f51905f52602080a3602060405160018152f35b82851161065a57846387a211a293039055856105e8565b6313be252b5f526004601cfd5b346100f7575f3660031901126100f75760206805345cdf77eb68f44c54604051908152f35b346100f75760403660031901126100f7576106a561075c565b60243590602052637f5e9f20600c52335f52806034600c20555f52602c5160601c337f8c5be1e5ebec7d5bd14f71427d1e84f3dd0314c0f7b2291e5b200ac8c7c3b92560205fa3602060405160018152f35b346100f7575f3660031901126100f7576103b4610712610788565b6e04578706572696d656e74455243323608c1b6020820152604051918291825b602060409281835280519182918282860152018484015e5f828201840152601f01601f1916010190565b600435906001600160a01b03821682036100f757565b602435906001600160a01b03821682036100f757565b604051906040820182811067ffffffffffffffff8211176103b857604052600f8252565b6805345cdf77eb68f44c548281019081106107ff576805345cdf77eb68f44c556387a211a2600c525f526020600c20818154019055602052600c5160601c5f5f51602061080d5f395f51905f52602080a3565b63e5cfe9575f526004601cfdfeddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3efa2646970667358221220fbe302881d9891005ba1448ba48547cc1cb17dea1a5c4011dfcb035de325bb1d64736f6c634300081b0033"
);

pub type State = foundry_evm::utils::StateChangeset;

/// A block request, which includes the Pool Transactions if it's Pending
#[derive(Debug)]
pub enum BlockRequest {
    Pending(Vec<Arc<PoolTransaction>>),
    Number(u64),
}

impl BlockRequest {
    pub fn block_number(&self) -> BlockNumber {
        match *self {
            Self::Pending(_) => BlockNumber::Pending,
            Self::Number(n) => BlockNumber::Number(n),
        }
    }
}

/// Gives access to the [revm::Database]
#[derive(Clone, Debug)]
pub struct Backend {
    /// Access to [`revm::Database`] abstraction.
    ///
    /// This will be used in combination with [`alloy_evm::Evm`] and is responsible for feeding
    /// data to the evm during its execution.
    ///
    /// At time of writing, there are two different types of `Db`:
    ///   - [`MemDb`](crate::mem::in_memory_db::MemDb): everything is stored in memory
    ///   - [`ForkDb`](crate::mem::fork_db::ForkedDatabase): forks off a remote client, missing
    ///     data is retrieved via RPC-calls
    ///
    /// In order to commit changes to the [`revm::Database`], the [`alloy_evm::Evm`] requires
    /// mutable access, which requires a write-lock from this `db`. In forking mode, the time
    /// during which the write-lock is active depends on whether the `ForkDb` can provide all
    /// requested data from memory or whether it has to retrieve it via RPC calls first. This
    /// means that it potentially blocks for some time, even taking into account the rate
    /// limits of RPC endpoints. Therefore the `Db` is guarded by a `tokio::sync::RwLock` here
    /// so calls that need to read from it, while it's currently written to, don't block. E.g.
    /// a new block is currently mined and a new [`Self::set_storage_at()`] request is being
    /// executed.
    db: Arc<AsyncRwLock<Box<dyn Db>>>,
    /// stores all block related data in memory.
    blockchain: Blockchain,
    /// Historic states of previous blocks.
    states: Arc<RwLock<InMemoryBlockStates>>,
    /// Env data of the chain
    env: Arc<RwLock<Env>>,
    /// This is set if this is currently forked off another client.
    fork: Arc<RwLock<Option<ClientFork>>>,
    /// Provides time related info, like timestamp.
    time: TimeManager,
    /// Contains state of custom overrides.
    cheats: CheatsManager,
    /// Contains fee data.
    fees: FeeManager,
    /// Initialised genesis.
    genesis: GenesisConfig,
    /// Listeners for new blocks that get notified when a new block was imported.
    new_block_listeners: Arc<Mutex<Vec<UnboundedSender<NewBlockNotification>>>>,
    /// Keeps track of active state snapshots at a specific block.
    active_state_snapshots: Arc<Mutex<HashMap<U256, (u64, B256)>>>,
    enable_steps_tracing: bool,
    print_logs: bool,
    print_traces: bool,
    /// Recorder used for decoding traces, used together with print_traces
    call_trace_decoder: Arc<CallTraceDecoder>,
    /// How to keep history state
    prune_state_history_config: PruneStateHistoryConfig,
    /// max number of blocks with transactions in memory
    transaction_block_keeper: Option<usize>,
    node_config: Arc<AsyncRwLock<NodeConfig>>,
    /// Slots in an epoch
    slots_in_an_epoch: u64,
    /// Precompiles to inject to the EVM.
    precompile_factory: Option<Arc<dyn PrecompileFactory>>,
    /// Prevent race conditions during mining
    mining: Arc<tokio::sync::Mutex<()>>,
    // === wallet === //
    capabilities: Arc<RwLock<WalletCapabilities>>,
    executor_wallet: Arc<RwLock<Option<EthereumWallet>>>,
    /// Disable pool balance checks
    disable_pool_balance_checks: bool,
}

impl Backend {
    /// Initialises the balance of the given accounts
    #[expect(clippy::too_many_arguments)]
    pub async fn with_genesis(
        db: Arc<AsyncRwLock<Box<dyn Db>>>,
        env: Arc<RwLock<Env>>,
        genesis: GenesisConfig,
        fees: FeeManager,
        fork: Arc<RwLock<Option<ClientFork>>>,
        enable_steps_tracing: bool,
        print_logs: bool,
        print_traces: bool,
        call_trace_decoder: Arc<CallTraceDecoder>,
        prune_state_history_config: PruneStateHistoryConfig,
        max_persisted_states: Option<usize>,
        transaction_block_keeper: Option<usize>,
        automine_block_time: Option<Duration>,
        cache_path: Option<PathBuf>,
        node_config: Arc<AsyncRwLock<NodeConfig>>,
    ) -> Result<Self> {
        // if this is a fork then adjust the blockchain storage
        let blockchain = if let Some(fork) = fork.read().as_ref() {
            trace!(target: "backend", "using forked blockchain at {}", fork.block_number());
            Blockchain::forked(fork.block_number(), fork.block_hash(), fork.total_difficulty())
        } else {
            let env = env.read();
            Blockchain::new(
                &env,
                env.evm_env.cfg_env.spec,
                fees.is_eip1559().then(|| fees.base_fee()),
                genesis.timestamp,
                genesis.number,
            )
        };

        // Sync EVM block.number with genesis for non-fork mode.
        // Fork mode syncs in setup_fork_db_config() instead.
        if fork.read().is_none() {
            let mut write_env = env.write();
            write_env.evm_env.block_env.number = U256::from(genesis.number);
        }

        let start_timestamp = if let Some(fork) = fork.read().as_ref() {
            fork.timestamp()
        } else {
            genesis.timestamp
        };

        let mut states = if prune_state_history_config.is_config_enabled() {
            // if prune state history is enabled, configure the state cache only for memory
            prune_state_history_config
                .max_memory_history
                .map(|limit| InMemoryBlockStates::new(limit, 0))
                .unwrap_or_default()
                .memory_only()
        } else if max_persisted_states.is_some() {
            max_persisted_states
                .map(|limit| InMemoryBlockStates::new(DEFAULT_HISTORY_LIMIT, limit))
                .unwrap_or_default()
        } else {
            Default::default()
        };

        if let Some(cache_path) = cache_path {
            states = states.disk_path(cache_path);
        }

        let (slots_in_an_epoch, precompile_factory, disable_pool_balance_checks) = {
            let cfg = node_config.read().await;
            (cfg.slots_in_an_epoch, cfg.precompile_factory.clone(), cfg.disable_pool_balance_checks)
        };

        let backend = Self {
            db,
            blockchain,
            states: Arc::new(RwLock::new(states)),
            env,
            fork,
            time: TimeManager::new(start_timestamp),
            cheats: Default::default(),
            new_block_listeners: Default::default(),
            fees,
            genesis,
            active_state_snapshots: Arc::new(Mutex::new(Default::default())),
            enable_steps_tracing,
            print_logs,
            print_traces,
            call_trace_decoder,
            prune_state_history_config,
            transaction_block_keeper,
            node_config,
            slots_in_an_epoch,
            precompile_factory,
            mining: Arc::new(tokio::sync::Mutex::new(())),
            capabilities: Arc::new(RwLock::new(WalletCapabilities(Default::default()))),
            executor_wallet: Arc::new(RwLock::new(None)),
            disable_pool_balance_checks,
        };

        if let Some(interval_block_time) = automine_block_time {
            backend.update_interval_mine_block_time(interval_block_time);
        }

        // Note: this can only fail in forking mode, in which case we can't recover
        backend.apply_genesis().await.wrap_err("failed to create genesis")?;
        Ok(backend)
    }

    /// Writes the CREATE2 deployer code directly to the database at the address provided.
    pub async fn set_create2_deployer(&self, address: Address) -> DatabaseResult<()> {
        self.set_code(address, Bytes::from_static(DEFAULT_CREATE2_DEPLOYER_RUNTIME_CODE)).await?;

        Ok(())
    }

    /// Get the capabilities of the wallet.
    ///
    /// Currently the only capability is delegation.
    ///
    /// See `anvil_core::eth::wallet::Capabilities` for construction helpers.
    pub(crate) fn get_capabilities(&self) -> WalletCapabilities {
        self.capabilities.read().clone()
    }

    /// Updates memory limits that should be more strict when auto-mine is enabled
    pub(crate) fn update_interval_mine_block_time(&self, block_time: Duration) {
        self.states.write().update_interval_mine_block_time(block_time)
    }

    /// Adds an address to the wallet's delegation capability.
    pub(crate) fn add_capability(&self, address: Address) {
        let chain_id = self.env.read().evm_env.cfg_env.chain_id;
        let mut capabilities = self.capabilities.write();
        let mut capability = capabilities
            .get(chain_id)
            .cloned()
            .unwrap_or(Capabilities { delegation: DelegationCapability { addresses: vec![] } });
        capability.delegation.addresses.push(address);
        capabilities.0.insert(chain_id, capability);
    }

    pub(crate) fn set_executor(&self, executor_pk: String) -> Result<Address, BlockchainError> {
        let signer: PrivateKeySigner =
            executor_pk.parse().map_err(|_| RpcError::invalid_params("Invalid private key"))?;

        let executor = signer.address();
        let wallet = EthereumWallet::new(signer);

        *self.executor_wallet.write() = Some(wallet);

        Ok(executor)
    }

    /// Applies the configured genesis settings
    ///
    /// This will fund, create the genesis accounts
    async fn apply_genesis(&self) -> Result<(), DatabaseError> {
        trace!(target: "backend", "setting genesis balances");

        if self.fork.read().is_some() {
            // fetch all account first
            let mut genesis_accounts_futures = Vec::with_capacity(self.genesis.accounts.len());
            for address in self.genesis.accounts.iter().copied() {
                let db = Arc::clone(&self.db);

                // The forking Database backend can handle concurrent requests, we can fetch all dev
                // accounts concurrently by spawning the job to a new task
                // In offline mode, OfflineForkedDatabase will return defaults without RPC calls
                genesis_accounts_futures.push(tokio::task::spawn(async move {
                    let db = db.read().await;
                    let info = db.basic_ref(address)?.unwrap_or_default();
                    Ok::<_, DatabaseError>((address, info))
                }));
            }

            let genesis_accounts = futures::future::join_all(genesis_accounts_futures).await;

            let mut db = self.db.write().await;

            for res in genesis_accounts {
                let (address, mut info) = res.unwrap()?;
                info.balance = self.genesis.balance;
                db.insert_account(address, info.clone());
            }
        } else {
            let mut db = self.db.write().await;
            for (account, info) in self.genesis.account_infos() {
                db.insert_account(account, info);
            }

            // insert the new genesis hash to the database so it's available for the next block in
            // the evm
            db.insert_block_hash(U256::from(self.best_number()), self.best_hash());
        }

        let db = self.db.write().await;
        // apply the genesis.json alloc
        self.genesis.apply_genesis_json_alloc(db)?;

        trace!(target: "backend", "set genesis balances");

        Ok(())
    }

    /// Sets the account to impersonate
    ///
    /// Returns `true` if the account is already impersonated
    pub fn impersonate(&self, addr: Address) -> bool {
        if self.cheats.impersonated_accounts().contains(&addr) {
            return true;
        }
        // Ensure EIP-3607 is disabled
        let mut env = self.env.write();
        env.evm_env.cfg_env.disable_eip3607 = true;
        self.cheats.impersonate(addr)
    }

    /// Removes the account that from the impersonated set
    ///
    /// If the impersonated `addr` is a contract then we also reset the code here
    pub fn stop_impersonating(&self, addr: Address) {
        self.cheats.stop_impersonating(&addr);
    }

    /// If set to true will make every account impersonated
    pub fn auto_impersonate_account(&self, enabled: bool) {
        self.cheats.set_auto_impersonate_account(enabled);
    }

    /// Returns the configured fork, if any
    pub fn get_fork(&self) -> Option<ClientFork> {
        self.fork.read().clone()
    }

    /// Returns whether the node is in offline mode
    pub async fn is_offline(&self) -> bool {
        self.node_config.read().await.offline
    }

    /// Returns the database
    pub fn get_db(&self) -> &Arc<AsyncRwLock<Box<dyn Db>>> {
        &self.db
    }

    /// Returns the `AccountInfo` from the database
    pub async fn get_account(&self, address: Address) -> DatabaseResult<AccountInfo> {
        Ok(self.db.read().await.basic_ref(address)?.unwrap_or_default())
    }

    /// Whether we're forked off some remote client
    pub fn is_fork(&self) -> bool {
        self.fork.read().is_some()
    }

    /// Resets the fork to a fresh state
    pub async fn reset_fork(&self, forking: Forking) -> Result<(), BlockchainError> {
        if !self.is_fork() {
            if let Some(eth_rpc_url) = forking.clone().json_rpc_url {
                let mut env = self.env.read().clone();

                let (db, config) = {
                    let mut node_config = self.node_config.write().await;

                    // we want to force the correct base fee for the next block during
                    // `setup_fork_db_config`
                    node_config.base_fee.take();

                    node_config.setup_fork_db_config(eth_rpc_url, &mut env, &self.fees).await?
                };

                *self.db.write().await = Box::new(db);

                let fork = ClientFork::new(config, Arc::clone(&self.db));

                *self.env.write() = env;
                *self.fork.write() = Some(fork);
            } else {
                return Err(RpcError::invalid_params(
                    "Forking not enabled and RPC URL not provided to start forking",
                )
                .into());
            }
        }

        if let Some(fork) = self.get_fork() {
            let block_number =
                forking.block_number.map(BlockNumber::from).unwrap_or(BlockNumber::Latest);
            // reset the fork entirely and reapply the genesis config
            fork.reset(forking.json_rpc_url.clone(), block_number).await?;
            let fork_block_number = fork.block_number();
            let fork_block = fork
                .block_by_number(fork_block_number)
                .await?
                .ok_or(BlockchainError::BlockNotFound)?;
            // update all settings related to the forked block
            {
                if let Some(fork_url) = forking.json_rpc_url {
                    self.reset_block_number(fork_url, fork_block_number).await?;
                } else {
                    // If rpc url is unspecified, then update the fork with the new block number and
                    // existing rpc url, this updates the cache path
                    {
                        let maybe_fork_url = { self.node_config.read().await.eth_rpc_url.clone() };
                        if let Some(fork_url) = maybe_fork_url {
                            self.reset_block_number(fork_url, fork_block_number).await?;
                        }
                    }

                    let gas_limit = self.node_config.read().await.fork_gas_limit(&fork_block);
                    let mut env = self.env.write();

                    env.evm_env.cfg_env.chain_id = fork.chain_id();
                    env.evm_env.block_env = BlockEnv {
                        number: U256::from(fork_block_number),
                        timestamp: U256::from(fork_block.header.timestamp),
                        gas_limit,
                        difficulty: fork_block.header.difficulty,
                        prevrandao: Some(fork_block.header.mix_hash.unwrap_or_default()),
                        // Keep previous `beneficiary` and `basefee` value
                        beneficiary: env.evm_env.block_env.beneficiary,
                        basefee: env.evm_env.block_env.basefee,
                        ..env.evm_env.block_env.clone()
                    };

                    // this is the base fee of the current block, but we need the base fee of
                    // the next block
                    let next_block_base_fee = self.fees.get_next_block_base_fee_per_gas(
                        fork_block.header.gas_used,
                        gas_limit,
                        fork_block.header.base_fee_per_gas.unwrap_or_default(),
                    );

                    self.fees.set_base_fee(next_block_base_fee);
                }

                // reset the time to the timestamp of the forked block
                self.time.reset(fork_block.header.timestamp);

                // also reset the total difficulty
                self.blockchain.storage.write().total_difficulty = fork.total_difficulty();
            }
            // reset storage
            *self.blockchain.storage.write() = BlockchainStorage::forked(
                fork.block_number(),
                fork.block_hash(),
                fork.total_difficulty(),
            );
            self.states.write().clear();
            self.db.write().await.clear();

            self.apply_genesis().await?;

            trace!(target: "backend", "reset fork");

            Ok(())
        } else {
            Err(RpcError::invalid_params("Forking not enabled").into())
        }
    }

    /// Resets the backend to a fresh in-memory state, clearing all existing data
    pub async fn reset_to_in_mem(&self) -> Result<(), BlockchainError> {
        // Clear the fork if any exists
        *self.fork.write() = None;

        // Get environment and genesis config
        let env = self.env.read().clone();
        let genesis_timestamp = self.genesis.timestamp;
        let genesis_number = self.genesis.number;
        let spec_id = self.spec_id();

        // Reset environment to genesis state
        {
            let mut env = self.env.write();
            env.evm_env.block_env.number = U256::from(genesis_number);
            env.evm_env.block_env.timestamp = U256::from(genesis_timestamp);
            // Reset other block env fields to their defaults
            env.evm_env.block_env.basefee = self.fees.base_fee();
            env.evm_env.block_env.prevrandao = Some(B256::ZERO);
        }

        // Clear all storage and reinitialize with genesis
        let base_fee = if self.fees.is_eip1559() { Some(self.fees.base_fee()) } else { None };
        *self.blockchain.storage.write() =
            BlockchainStorage::new(&env, spec_id, base_fee, genesis_timestamp, genesis_number);
        self.states.write().clear();

        // Clear the database
        self.db.write().await.clear();

        // Reset time manager
        self.time.reset(genesis_timestamp);

        // Reset fees to initial state
        if self.fees.is_eip1559() {
            self.fees.set_base_fee(crate::eth::fees::INITIAL_BASE_FEE);
        }

        self.fees.set_gas_price(crate::eth::fees::INITIAL_GAS_PRICE);

        // Reapply genesis configuration
        self.apply_genesis().await?;

        trace!(target: "backend", "reset to fresh in-memory state");

        Ok(())
    }

    async fn reset_block_number(
        &self,
        fork_url: String,
        fork_block_number: u64,
    ) -> Result<(), BlockchainError> {
        let mut node_config = self.node_config.write().await;
        node_config.fork_choice = Some(ForkChoice::Block(fork_block_number as i128));

        let mut env = self.env.read().clone();
        let (forked_db, client_fork_config) =
            node_config.setup_fork_db_config(fork_url, &mut env, &self.fees).await?;

        *self.db.write().await = Box::new(forked_db);
        let fork = ClientFork::new(client_fork_config, Arc::clone(&self.db));
        *self.fork.write() = Some(fork);
        *self.env.write() = env;

        Ok(())
    }

    /// Returns the `TimeManager` responsible for timestamps
    pub fn time(&self) -> &TimeManager {
        &self.time
    }

    /// Returns the `CheatsManager` responsible for executing cheatcodes
    pub fn cheats(&self) -> &CheatsManager {
        &self.cheats
    }

    /// Whether to skip blob validation
    pub fn skip_blob_validation(&self, impersonator: Option<Address>) -> bool {
        self.cheats().auto_impersonate_accounts()
            || impersonator
                .is_some_and(|addr| self.cheats().impersonated_accounts().contains(&addr))
    }

    /// Returns the `FeeManager` that manages fee/pricings
    pub fn fees(&self) -> &FeeManager {
        &self.fees
    }

    /// The env data of the blockchain
    pub fn env(&self) -> &Arc<RwLock<Env>> {
        &self.env
    }

    /// Returns the current best hash of the chain
    pub fn best_hash(&self) -> B256 {
        self.blockchain.storage.read().best_hash
    }

    /// Returns the current best number of the chain
    pub fn best_number(&self) -> u64 {
        self.blockchain.storage.read().best_number
    }

    /// Sets the block number
    pub fn set_block_number(&self, number: u64) {
        let mut env = self.env.write();
        env.evm_env.block_env.number = U256::from(number);
    }

    /// Returns the client coinbase address.
    pub fn coinbase(&self) -> Address {
        self.env.read().evm_env.block_env.beneficiary
    }

    /// Returns the client coinbase address.
    pub fn chain_id(&self) -> U256 {
        U256::from(self.env.read().evm_env.cfg_env.chain_id)
    }

    pub fn set_chain_id(&self, chain_id: u64) {
        self.env.write().evm_env.cfg_env.chain_id = chain_id;
    }

    /// Returns the genesis data for the Beacon API.
    pub fn genesis_time(&self) -> u64 {
        self.genesis.timestamp
    }

    /// Returns balance of the given account.
    pub async fn current_balance(&self, address: Address) -> DatabaseResult<U256> {
        Ok(self.get_account(address).await?.balance)
    }

    /// Returns balance of the given account.
    pub async fn current_nonce(&self, address: Address) -> DatabaseResult<u64> {
        Ok(self.get_account(address).await?.nonce)
    }

    /// Sets the coinbase address
    pub fn set_coinbase(&self, address: Address) {
        self.env.write().evm_env.block_env.beneficiary = address;
    }

    /// Sets the nonce of the given address
    pub async fn set_nonce(&self, address: Address, nonce: U256) -> DatabaseResult<()> {
        self.db.write().await.set_nonce(address, nonce.try_into().unwrap_or(u64::MAX))
    }

    /// Sets the balance of the given address
    pub async fn set_balance(&self, address: Address, balance: U256) -> DatabaseResult<()> {
        self.db.write().await.set_balance(address, balance)
    }

    /// Sets the code of the given address
    pub async fn set_code(&self, address: Address, code: Bytes) -> DatabaseResult<()> {
        self.db.write().await.set_code(address, code)
    }

    /// Sets the value for the given slot of the given address
    pub async fn set_storage_at(
        &self,
        address: Address,
        slot: U256,
        val: B256,
    ) -> DatabaseResult<()> {
        self.db.write().await.set_storage_at(address, slot.into(), val)
    }

    /// Returns the configured specid
    pub fn spec_id(&self) -> SpecId {
        self.env.read().evm_env.cfg_env.spec
    }

    /// Returns true for post London
    pub fn is_eip1559(&self) -> bool {
        (self.spec_id() as u8) >= (SpecId::LONDON as u8)
    }

    /// Returns true for post Merge
    pub fn is_eip3675(&self) -> bool {
        (self.spec_id() as u8) >= (SpecId::MERGE as u8)
    }

    /// Returns true for post Berlin
    pub fn is_eip2930(&self) -> bool {
        (self.spec_id() as u8) >= (SpecId::BERLIN as u8)
    }

    /// Returns true for post Cancun
    pub fn is_eip4844(&self) -> bool {
        (self.spec_id() as u8) >= (SpecId::CANCUN as u8)
    }

    /// Returns true for post Prague
    pub fn is_eip7702(&self) -> bool {
        (self.spec_id() as u8) >= (SpecId::PRAGUE as u8)
    }

    /// Returns true if op-stack deposits are active
    pub fn is_optimism(&self) -> bool {
        self.env.read().networks.is_optimism()
    }

    /// Returns the precompiles for the current spec.
    pub fn precompiles(&self) -> BTreeMap<String, Address> {
        let spec_id = self.env.read().evm_env.cfg_env.spec;
        let precompiles = Precompiles::new(PrecompileSpecId::from_spec_id(spec_id));

        let mut precompiles_map = BTreeMap::<String, Address>::default();
        for (address, precompile) in precompiles.inner() {
            precompiles_map.insert(precompile.id().name().to_string(), *address);
        }

        // Extend with configured network precompiles.
        precompiles_map.extend(self.env.read().networks.precompiles());

        if let Some(factory) = &self.precompile_factory {
            for (address, precompile) in factory.precompiles() {
                precompiles_map.insert(precompile.precompile_id().to_string(), address);
            }
        }

        precompiles_map
    }

    /// Returns the system contracts for the current spec.
    pub fn system_contracts(&self) -> BTreeMap<SystemContract, Address> {
        let mut system_contracts = BTreeMap::<SystemContract, Address>::default();

        let spec_id = self.env.read().evm_env.cfg_env.spec;

        if spec_id >= SpecId::CANCUN {
            system_contracts.extend(SystemContract::cancun());
        }

        if spec_id >= SpecId::PRAGUE {
            system_contracts.extend(SystemContract::prague(None));
        }

        system_contracts
    }

    /// Returns [`BlobParams`] corresponding to the current spec.
    pub fn blob_params(&self) -> BlobParams {
        let spec_id = self.env.read().evm_env.cfg_env.spec;

        if spec_id >= SpecId::OSAKA {
            return BlobParams::osaka();
        }

        if spec_id >= SpecId::PRAGUE {
            return BlobParams::prague();
        }

        BlobParams::cancun()
    }

    /// Returns an error if EIP1559 is not active (pre Berlin)
    pub fn ensure_eip1559_active(&self) -> Result<(), BlockchainError> {
        if self.is_eip1559() {
            return Ok(());
        }
        Err(BlockchainError::EIP1559TransactionUnsupportedAtHardfork)
    }

    /// Returns an error if EIP1559 is not active (pre muirGlacier)
    pub fn ensure_eip2930_active(&self) -> Result<(), BlockchainError> {
        if self.is_eip2930() {
            return Ok(());
        }
        Err(BlockchainError::EIP2930TransactionUnsupportedAtHardfork)
    }

    pub fn ensure_eip4844_active(&self) -> Result<(), BlockchainError> {
        if self.is_eip4844() {
            return Ok(());
        }
        Err(BlockchainError::EIP4844TransactionUnsupportedAtHardfork)
    }

    pub fn ensure_eip7702_active(&self) -> Result<(), BlockchainError> {
        if self.is_eip7702() {
            return Ok(());
        }
        Err(BlockchainError::EIP7702TransactionUnsupportedAtHardfork)
    }

    /// Returns an error if op-stack deposits are not active
    pub fn ensure_op_deposits_active(&self) -> Result<(), BlockchainError> {
        if self.is_optimism() {
            return Ok(());
        }
        Err(BlockchainError::DepositTransactionUnsupported)
    }

    /// Returns the block gas limit
    pub fn gas_limit(&self) -> u64 {
        self.env.read().evm_env.block_env.gas_limit
    }

    /// Sets the block gas limit
    pub fn set_gas_limit(&self, gas_limit: u64) {
        self.env.write().evm_env.block_env.gas_limit = gas_limit;
    }

    /// Returns the current base fee
    pub fn base_fee(&self) -> u64 {
        self.fees.base_fee()
    }

    /// Returns whether the minimum suggested priority fee is enforced
    pub fn is_min_priority_fee_enforced(&self) -> bool {
        self.fees.is_min_priority_fee_enforced()
    }

    pub fn excess_blob_gas_and_price(&self) -> Option<BlobExcessGasAndPrice> {
        self.fees.excess_blob_gas_and_price()
    }

    /// Sets the current basefee
    pub fn set_base_fee(&self, basefee: u64) {
        self.fees.set_base_fee(basefee)
    }

    /// Sets the gas price
    pub fn set_gas_price(&self, price: u128) {
        self.fees.set_gas_price(price)
    }

    pub fn elasticity(&self) -> f64 {
        self.fees.elasticity()
    }

    /// Returns the total difficulty of the chain until this block
    ///
    /// Note: this will always be `0` in memory mode
    /// In forking mode this will always be the total difficulty of the forked block
    pub fn total_difficulty(&self) -> U256 {
        self.blockchain.storage.read().total_difficulty
    }

    /// Creates a new `evm_snapshot` at the current height.
    ///
    /// Returns the id of the snapshot created.
    pub async fn create_state_snapshot(&self) -> U256 {
        let num = self.best_number();
        let hash = self.best_hash();
        let id = self.db.write().await.snapshot_state();
        trace!(target: "backend", "creating snapshot {} at {}", id, num);
        self.active_state_snapshots.lock().insert(id, (num, hash));
        id
    }

    /// Reverts the state to the state snapshot identified by the given `id`.
    pub async fn revert_state_snapshot(&self, id: U256) -> Result<bool, BlockchainError> {
        let block = { self.active_state_snapshots.lock().remove(&id) };
        if let Some((num, hash)) = block {
            let best_block_hash = {
                // revert the storage that's newer than the snapshot
                let current_height = self.best_number();
                let mut storage = self.blockchain.storage.write();

                for n in ((num + 1)..=current_height).rev() {
                    trace!(target: "backend", "reverting block {}", n);
                    if let Some(hash) = storage.hashes.remove(&n)
                        && let Some(block) = storage.blocks.remove(&hash)
                    {
                        for tx in block.body.transactions {
                            let _ = storage.transactions.remove(&tx.hash());
                        }
                    }
                }

                storage.best_number = num;
                storage.best_hash = hash;
                hash
            };
            let block =
                self.block_by_hash(best_block_hash).await?.ok_or(BlockchainError::BlockNotFound)?;

            let reset_time = block.header.timestamp;
            self.time.reset(reset_time);

            let mut env = self.env.write();
            env.evm_env.block_env = BlockEnv {
                number: U256::from(num),
                timestamp: U256::from(block.header.timestamp),
                difficulty: block.header.difficulty,
                // ensures prevrandao is set
                prevrandao: Some(block.header.mix_hash.unwrap_or_default()),
                gas_limit: block.header.gas_limit,
                // Keep previous `beneficiary` and `basefee` value
                beneficiary: env.evm_env.block_env.beneficiary,
                basefee: env.evm_env.block_env.basefee,
                ..Default::default()
            }
        }
        Ok(self.db.write().await.revert_state(id, RevertStateSnapshotAction::RevertRemove))
    }

    pub fn list_state_snapshots(&self) -> BTreeMap<U256, (u64, B256)> {
        self.active_state_snapshots.lock().clone().into_iter().collect()
    }

    /// Get the current state.
    pub async fn serialized_state(
        &self,
        preserve_historical_states: bool,
    ) -> Result<SerializableState, BlockchainError> {
        let at = self.env.read().evm_env.block_env.clone();
        let best_number = self.blockchain.storage.read().best_number;
        let blocks = self.blockchain.storage.read().serialized_blocks();
        let transactions = self.blockchain.storage.read().serialized_transactions();
        let historical_states = if preserve_historical_states {
            Some(self.states.write().serialized_states())
        } else {
            None
        };

        let state = self.db.read().await.dump_state(
            at,
            best_number,
            blocks,
            transactions,
            historical_states,
        )?;
        state.ok_or_else(|| {
            RpcError::invalid_params("Dumping state not supported with the current configuration")
                .into()
        })
    }

    /// Write all chain data to serialized bytes buffer
    pub async fn dump_state(
        &self,
        preserve_historical_states: bool,
    ) -> Result<Bytes, BlockchainError> {
        let state = self.serialized_state(preserve_historical_states).await?;
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder
            .write_all(&serde_json::to_vec(&state).unwrap_or_default())
            .map_err(|_| BlockchainError::DataUnavailable)?;
        Ok(encoder.finish().unwrap_or_default().into())
    }

    /// Apply [SerializableState] data to the backend storage.
    pub async fn load_state(&self, state: SerializableState) -> Result<bool, BlockchainError> {
        // load the blocks and transactions into the storage
        self.blockchain.storage.write().load_blocks(state.blocks.clone());
        self.blockchain.storage.write().load_transactions(state.transactions.clone());
        // reset the block env
        if let Some(block) = state.block.clone() {
            self.env.write().evm_env.block_env = block.clone();

            // Set the current best block number.
            // Defaults to block number for compatibility with existing state files.
            let fork_num_and_hash = self.get_fork().map(|f| (f.block_number(), f.block_hash()));

            let best_number = state.best_block_number.unwrap_or(block.number.saturating_to());
            if let Some((number, hash)) = fork_num_and_hash {
                trace!(target: "backend", state_block_number=?best_number, fork_block_number=?number);
                // If the state.block_number is greater than the fork block number, set best number
                // to the state block number.
                // Ref: https://github.com/foundry-rs/foundry/issues/9539
                if best_number > number {
                    self.blockchain.storage.write().best_number = best_number;
                    let best_hash =
                        self.blockchain.storage.read().hash(best_number.into()).ok_or_else(
                            || {
                                BlockchainError::RpcError(RpcError::internal_error_with(format!(
                                    "Best hash not found for best number {best_number}",
                                )))
                            },
                        )?;
                    self.blockchain.storage.write().best_hash = best_hash;
                } else {
                    // If loading state file on a fork, set best number to the fork block number.
                    // Ref: https://github.com/foundry-rs/foundry/pull/9215#issue-2618681838
                    self.blockchain.storage.write().best_number = number;
                    self.blockchain.storage.write().best_hash = hash;
                }
            } else {
                self.blockchain.storage.write().best_number = best_number;

                // Set the current best block hash;
                let best_hash =
                    self.blockchain.storage.read().hash(best_number.into()).ok_or_else(|| {
                        BlockchainError::RpcError(RpcError::internal_error_with(format!(
                            "Best hash not found for best number {best_number}",
                        )))
                    })?;

                self.blockchain.storage.write().best_hash = best_hash;
            }
        }

        if let Some(latest) = state.blocks.iter().max_by_key(|b| b.header.number) {
            let header = &latest.header;
            let next_block_base_fee = self.fees.get_next_block_base_fee_per_gas(
                header.gas_used,
                header.gas_limit,
                header.base_fee_per_gas.unwrap_or_default(),
            );
            let next_block_excess_blob_gas = self.fees.get_next_block_blob_excess_gas(
                header.excess_blob_gas.unwrap_or_default(),
                header.blob_gas_used.unwrap_or_default(),
            );

            // update next base fee
            self.fees.set_base_fee(next_block_base_fee);

            self.fees.set_blob_excess_gas_and_price(BlobExcessGasAndPrice::new(
                next_block_excess_blob_gas,
                get_blob_base_fee_update_fraction(
                    self.env.read().evm_env.cfg_env.chain_id,
                    header.timestamp,
                ),
            ));
        }

        if !self.db.write().await.load_state(state.clone())? {
            return Err(RpcError::invalid_params(
                "Loading state not supported with the current configuration",
            )
            .into());
        }

        if let Some(historical_states) = state.historical_states {
            self.states.write().load_states(historical_states);
        }

        Ok(true)
    }

    /// Deserialize and add all chain data to the backend storage
    pub async fn load_state_bytes(&self, buf: Bytes) -> Result<bool, BlockchainError> {
        let orig_buf = &buf.0[..];
        let mut decoder = GzDecoder::new(orig_buf);
        let mut decoded_data = Vec::new();

        let state: SerializableState = serde_json::from_slice(if decoder.header().is_some() {
            decoder
                .read_to_end(decoded_data.as_mut())
                .map_err(|_| BlockchainError::FailedToDecodeStateDump)?;
            &decoded_data
        } else {
            &buf.0
        })
        .map_err(|_| BlockchainError::FailedToDecodeStateDump)?;

        self.load_state(state).await
    }

    /// Returns the environment for the next block
    ///
    /// This is used for obtaining the evm environment for the next (pending) block (e.g.
    /// transaction validation in eth_sendrawTransaction)
    fn next_env(&self) -> Env {
        let mut env = self.env.read().clone();
        // increase block number for this block
        env.evm_env.block_env.number = env.evm_env.block_env.number.saturating_add(U256::from(1));
        env.evm_env.block_env.basefee = self.base_fee();
        env.evm_env.block_env.blob_excess_gas_and_price = self.excess_blob_gas_and_price();
        env.evm_env.block_env.timestamp = U256::from(self.time.current_call_timestamp());
        env
    }

    /// Creates an EVM instance with optionally injected precompiles.
    fn new_evm_with_inspector_ref<'db, I, DB>(
        &self,
        db: &'db DB,
        env: &Env,
        inspector: &'db mut I,
    ) -> EitherEvm<WrapDatabaseRef<&'db DB>, &'db mut I, PrecompilesMap>
    where
        DB: DatabaseRef + ?Sized,
        I: Inspector<EthEvmContext<WrapDatabaseRef<&'db DB>>>
            + Inspector<OpContext<WrapDatabaseRef<&'db DB>>>,
        WrapDatabaseRef<&'db DB>: Database<Error = DatabaseError>,
    {
        let mut evm = new_evm_with_inspector_ref(db, env, inspector);
        self.env.read().networks.inject_precompiles(evm.precompiles_mut());

        if let Some(factory) = &self.precompile_factory {
            evm.precompiles_mut().extend_precompiles(factory.precompiles());
        }

        let cheats = Arc::new(self.cheats.clone());
        if cheats.has_recover_overrides() {
            let cheat_ecrecover = CheatEcrecover::new(Arc::clone(&cheats));
            evm.precompiles_mut().apply_precompile(&EC_RECOVER, move |_| {
                Some(DynPrecompile::new_stateful(
                    cheat_ecrecover.precompile_id().clone(),
                    move |input| cheat_ecrecover.call(input),
                ))
            });
        }

        evm
    }

    /// executes the transactions without writing to the underlying database
    pub async fn inspect_tx(
        &self,
        tx: Arc<PoolTransaction>,
    ) -> Result<
        (InstructionResult, Option<Output>, u64, State, Vec<revm::primitives::Log>),
        BlockchainError,
    > {
        let mut env = self.next_env();
        env.tx = FromRecoveredTx::from_recovered_tx(
            tx.pending_transaction.transaction.as_ref(),
            *tx.pending_transaction.sender(),
        );

        if env.networks.is_optimism() {
            env.tx.enveloped_tx =
                Some(alloy_rlp::encode(tx.pending_transaction.transaction.as_ref()).into());
        }

        let db = self.db.read().await;
        let mut inspector = self.build_inspector();
        let mut evm = self.new_evm_with_inspector_ref(&**db, &env, &mut inspector);
        let ResultAndState { result, state } = evm.transact(env.tx)?;
        let (exit_reason, gas_used, out, logs) = match result {
            ExecutionResult::Success { reason, gas_used, logs, output, .. } => {
                (reason.into(), gas_used, Some(output), Some(logs))
            }
            ExecutionResult::Revert { gas_used, output } => {
                (InstructionResult::Revert, gas_used, Some(Output::Call(output)), None)
            }
            ExecutionResult::Halt { reason, gas_used } => {
                let eth_reason = op_haltreason_to_instruction_result(reason);
                (eth_reason, gas_used, None, None)
            }
        };

        drop(evm);
        inspector.print_logs();

        if self.print_traces {
            inspector.print_traces(self.call_trace_decoder.clone());
        }

        Ok((exit_reason, out, gas_used, state, logs.unwrap_or_default()))
    }

    /// Creates the pending block
    ///
    /// This will execute all transaction in the order they come but will not mine the block
    pub async fn pending_block(&self, pool_transactions: Vec<Arc<PoolTransaction>>) -> BlockInfo {
        self.with_pending_block(pool_transactions, |_, block| block).await
    }

    /// Creates the pending block
    ///
    /// This will execute all transaction in the order they come but will not mine the block
    pub async fn with_pending_block<F, T>(
        &self,
        pool_transactions: Vec<Arc<PoolTransaction>>,
        f: F,
    ) -> T
    where
        F: FnOnce(Box<dyn MaybeFullDatabase + '_>, BlockInfo) -> T,
    {
        let db = self.db.read().await;
        let env = self.next_env();

        let mut cache_db = CacheDB::new(&*db);

        let storage = self.blockchain.storage.read();

        let executor = TransactionExecutor {
            db: &mut cache_db,
            validator: self,
            pending: pool_transactions.into_iter(),
            block_env: env.evm_env.block_env.clone(),
            cfg_env: env.evm_env.cfg_env,
            parent_hash: storage.best_hash,
            gas_used: 0,
            blob_gas_used: 0,
            enable_steps_tracing: self.enable_steps_tracing,
            print_logs: self.print_logs,
            print_traces: self.print_traces,
            call_trace_decoder: self.call_trace_decoder.clone(),
            precompile_factory: self.precompile_factory.clone(),
            networks: self.env.read().networks,
            blob_params: self.blob_params(),
            cheats: self.cheats().clone(),
        };

        // create a new pending block
        let executed = executor.execute();
        f(Box::new(cache_db), executed.block)
    }

    /// Mines a new block and stores it.
    ///
    /// this will execute all transaction in the order they come in and return all the markers they
    /// provide.
    pub async fn mine_block(
        &self,
        pool_transactions: Vec<Arc<PoolTransaction>>,
    ) -> MinedBlockOutcome {
        self.do_mine_block(pool_transactions).await
    }

    async fn do_mine_block(
        &self,
        pool_transactions: Vec<Arc<PoolTransaction>>,
    ) -> MinedBlockOutcome {
        let _mining_guard = self.mining.lock().await;
        trace!(target: "backend", "creating new block with {} transactions", pool_transactions.len());

        let (outcome, header, block_hash) = {
            let current_base_fee = self.base_fee();
            let current_excess_blob_gas_and_price = self.excess_blob_gas_and_price();

            let mut env = self.env.read().clone();

            if env.evm_env.block_env.basefee == 0 {
                // this is an edge case because the evm fails if `tx.effective_gas_price < base_fee`
                // 0 is only possible if it's manually set
                env.evm_env.cfg_env.disable_base_fee = true;
            }

            let block_number = self.blockchain.storage.read().best_number.saturating_add(1);

            // increase block number for this block
            if is_arbitrum(env.evm_env.cfg_env.chain_id) {
                // Temporary set `env.block.number` to `block_number` for Arbitrum chains.
                env.evm_env.block_env.number = U256::from(block_number);
            } else {
                env.evm_env.block_env.number =
                    env.evm_env.block_env.number.saturating_add(U256::from(1));
            }

            env.evm_env.block_env.basefee = current_base_fee;
            env.evm_env.block_env.blob_excess_gas_and_price = current_excess_blob_gas_and_price;

            let best_hash = self.blockchain.storage.read().best_hash;

            let mut input = Vec::with_capacity(40);
            input.extend_from_slice(best_hash.as_slice());
            input.extend_from_slice(&block_number.to_le_bytes());
            env.evm_env.block_env.prevrandao = Some(keccak256(&input));

            if self.prune_state_history_config.is_state_history_supported() {
                let db = self.db.read().await.current_state();
                // store current state before executing all transactions
                self.states.write().insert(best_hash, db);
            }

            let (executed_tx, block_hash) = {
                let mut db = self.db.write().await;

                // finally set the next block timestamp, this is done just before execution, because
                // there can be concurrent requests that can delay acquiring the db lock and we want
                // to ensure the timestamp is as close as possible to the actual execution.
                env.evm_env.block_env.timestamp = U256::from(self.time.next_timestamp());

                let executor = TransactionExecutor {
                    db: &mut **db,
                    validator: self,
                    pending: pool_transactions.into_iter(),
                    block_env: env.evm_env.block_env.clone(),
                    cfg_env: env.evm_env.cfg_env.clone(),
                    parent_hash: best_hash,
                    gas_used: 0,
                    blob_gas_used: 0,
                    enable_steps_tracing: self.enable_steps_tracing,
                    print_logs: self.print_logs,
                    print_traces: self.print_traces,
                    call_trace_decoder: self.call_trace_decoder.clone(),
                    networks: self.env.read().networks,
                    precompile_factory: self.precompile_factory.clone(),
                    blob_params: self.blob_params(),
                    cheats: self.cheats().clone(),
                };
                let executed_tx = executor.execute();

                // we also need to update the new blockhash in the db itself
                let block_hash = executed_tx.block.block.header.hash_slow();
                db.insert_block_hash(U256::from(executed_tx.block.block.header.number), block_hash);

                (executed_tx, block_hash)
            };

            // create the new block with the current timestamp
            let ExecutedTransactions { block, included, invalid } = executed_tx;
            let BlockInfo { block, transactions, receipts } = block;

            let header = block.header.clone();

            trace!(
                target: "backend",
                "Mined block {} with {} tx {:?}",
                block_number,
                transactions.len(),
                transactions.iter().map(|tx| tx.transaction_hash).collect::<Vec<_>>()
            );
            let mut storage = self.blockchain.storage.write();
            // update block metadata
            storage.best_number = block_number;
            storage.best_hash = block_hash;
            // Difficulty is removed and not used after Paris (aka TheMerge). Value is replaced with
            // prevrandao. https://github.com/bluealloy/revm/blob/1839b3fce8eaeebb85025576f2519b80615aca1e/crates/interpreter/src/instructions/host_env.rs#L27
            if !self.is_eip3675() {
                storage.total_difficulty =
                    storage.total_difficulty.saturating_add(header.difficulty);
            }

            storage.blocks.insert(block_hash, block);
            storage.hashes.insert(block_number, block_hash);

            node_info!("");
            // insert all transactions
            for (info, receipt) in transactions.into_iter().zip(receipts) {
                // log some tx info
                node_info!("    Transaction: {:?}", info.transaction_hash);
                if let Some(contract) = &info.contract_address {
                    node_info!("    Contract created: {contract}");
                }
                node_info!("    Gas used: {}", receipt.cumulative_gas_used());
                if !info.exit.is_ok() {
                    let r = RevertDecoder::new().decode(
                        info.out.as_ref().map(|b| &b[..]).unwrap_or_default(),
                        Some(info.exit),
                    );
                    node_info!("    Error: reverted with: {r}");
                }
                node_info!("");

                let mined_tx = MinedTransaction { info, receipt, block_hash, block_number };
                storage.transactions.insert(mined_tx.info.transaction_hash, mined_tx);
            }

            // remove old transactions that exceed the transaction block keeper
            if let Some(transaction_block_keeper) = self.transaction_block_keeper
                && storage.blocks.len() > transaction_block_keeper
            {
                let to_clear = block_number
                    .saturating_sub(transaction_block_keeper.try_into().unwrap_or(u64::MAX));
                storage.remove_block_transactions_by_number(to_clear)
            }

            // we intentionally set the difficulty to `0` for newer blocks
            env.evm_env.block_env.difficulty = U256::from(0);

            // update env with new values
            *self.env.write() = env;

            let timestamp = utc_from_secs(header.timestamp);

            node_info!("    Block Number: {}", block_number);
            node_info!("    Block Hash: {:?}", block_hash);
            if timestamp.year() > 9999 {
                // rf2822 panics with more than 4 digits
                node_info!("    Block Time: {:?}\n", timestamp.to_rfc3339());
            } else {
                node_info!("    Block Time: {:?}\n", timestamp.to_rfc2822());
            }

            let outcome = MinedBlockOutcome { block_number, included, invalid };

            (outcome, header, block_hash)
        };
        let next_block_base_fee = self.fees.get_next_block_base_fee_per_gas(
            header.gas_used,
            header.gas_limit,
            header.base_fee_per_gas.unwrap_or_default(),
        );
        let next_block_excess_blob_gas = self.fees.get_next_block_blob_excess_gas(
            header.excess_blob_gas.unwrap_or_default(),
            header.blob_gas_used.unwrap_or_default(),
        );

        // update next base fee
        self.fees.set_base_fee(next_block_base_fee);

        self.fees.set_blob_excess_gas_and_price(BlobExcessGasAndPrice::new(
            next_block_excess_blob_gas,
            get_blob_base_fee_update_fraction_by_spec_id(*self.env.read().evm_env.spec_id()),
        ));

        // notify all listeners
        self.notify_on_new_block(header, block_hash);

        outcome
    }

    /// Executes the [TransactionRequest] without writing to the DB
    ///
    /// # Errors
    ///
    /// Returns an error if the `block_number` is greater than the current height
    pub async fn call(
        &self,
        request: WithOtherFields<TransactionRequest>,
        fee_details: FeeDetails,
        block_request: Option<BlockRequest>,
        overrides: EvmOverrides,
    ) -> Result<(InstructionResult, Option<Output>, u128, State), BlockchainError> {
        self.with_database_at(block_request, |state, mut block| {
            let block_number = block.number;
            let (exit, out, gas, state) = {
                let mut cache_db = CacheDB::new(state);
                if let Some(state_overrides) = overrides.state {
                    apply_state_overrides(state_overrides.into_iter().collect(), &mut cache_db)?;
                }
                if let Some(block_overrides) = overrides.block {
                    cache_db.apply_block_overrides(*block_overrides, &mut block);
                }
                self.call_with_state(&cache_db, request, fee_details, block)
            }?;
            trace!(target: "backend", "call return {:?} out: {:?} gas {} on block {}", exit, out, gas, block_number);
            Ok((exit, out, gas, state))
        }).await?
    }

    /// ## EVM settings
    ///
    /// This modifies certain EVM settings to mirror geth's `SkipAccountChecks` when transacting requests, see also: <https://github.com/ethereum/go-ethereum/blob/380688c636a654becc8f114438c2a5d93d2db032/core/state_transition.go#L145-L148>:
    ///
    ///  - `disable_eip3607` is set to `true`
    ///  - `disable_base_fee` is set to `true`
    ///  - `tx_gas_limit_cap` is set to `Some(u64::MAX)` indicating no gas limit cap
    ///  - `nonce` check is skipped
    fn build_call_env(
        &self,
        request: WithOtherFields<TransactionRequest>,
        fee_details: FeeDetails,
        block_env: BlockEnv,
    ) -> Env {
        let tx_type = request.minimal_tx_type() as u8;

        let WithOtherFields::<TransactionRequest> {
            inner:
                TransactionRequest {
                    from,
                    to,
                    gas,
                    value,
                    input,
                    access_list,
                    blob_versioned_hashes,
                    authorization_list,
                    nonce,
                    sidecar: _,
                    chain_id,
                    transaction_type,
                    .. // Rest of the gas fees related fields are taken from `fee_details`
                },
            other,
        } = request;

        let FeeDetails {
            gas_price,
            max_fee_per_gas,
            max_priority_fee_per_gas,
            max_fee_per_blob_gas,
        } = fee_details;

        let gas_limit = gas.unwrap_or(block_env.gas_limit);
        let mut env = self.env.read().clone();
        env.evm_env.block_env = block_env;
        // we want to disable this in eth_call, since this is common practice used by other node
        // impls and providers <https://github.com/foundry-rs/foundry/issues/4388>
        env.evm_env.cfg_env.disable_block_gas_limit = true;
        env.evm_env.cfg_env.tx_gas_limit_cap = Some(u64::MAX);

        // The basefee should be ignored for calls against state for
        // - eth_call
        // - eth_estimateGas
        // - eth_createAccessList
        // - tracing
        env.evm_env.cfg_env.disable_base_fee = true;

        // Disable nonce check in revm
        env.evm_env.cfg_env.disable_nonce_check = true;

        let gas_price = gas_price.or(max_fee_per_gas).unwrap_or_else(|| {
            self.fees().raw_gas_price().saturating_add(MIN_SUGGESTED_PRIORITY_FEE)
        });
        let caller = from.unwrap_or_default();
        let to = to.as_ref().and_then(TxKind::to);
        let blob_hashes = blob_versioned_hashes.unwrap_or_default();
        let mut base = TxEnv {
            caller,
            gas_limit,
            gas_price,
            gas_priority_fee: max_priority_fee_per_gas,
            max_fee_per_blob_gas: max_fee_per_blob_gas
                .or_else(|| {
                    if !blob_hashes.is_empty() {
                        env.evm_env.block_env.blob_gasprice()
                    } else {
                        Some(0)
                    }
                })
                .unwrap_or_default(),
            kind: match to {
                Some(addr) => TxKind::Call(*addr),
                None => TxKind::Create,
            },
            tx_type,
            value: value.unwrap_or_default(),
            data: input.into_input().unwrap_or_default(),
            chain_id: Some(chain_id.unwrap_or(self.env.read().evm_env.cfg_env.chain_id)),
            access_list: access_list.unwrap_or_default(),
            blob_hashes,
            ..Default::default()
        };
        base.set_signed_authorization(authorization_list.unwrap_or_default());
        env.tx = OpTransaction { base, ..Default::default() };

        if let Some(nonce) = nonce {
            env.tx.base.nonce = nonce;
        }

        if env.evm_env.block_env.basefee == 0 {
            // this is an edge case because the evm fails if `tx.effective_gas_price < base_fee`
            // 0 is only possible if it's manually set
            env.evm_env.cfg_env.disable_base_fee = true;
        }

        // Deposit transaction?
        if transaction_type == Some(DEPOSIT_TX_TYPE_ID) && has_optimism_fields(&other) {
            let deposit = DepositTransactionParts {
                source_hash: other
                    .get_deserialized::<B256>("sourceHash")
                    .map(|sh| sh.unwrap_or_default())
                    .unwrap_or_default(),
                mint: other
                    .get_deserialized::<u128>("mint")
                    .map(|m| m.unwrap_or_default())
                    .or(None),
                is_system_transaction: other
                    .get_deserialized::<bool>("isSystemTx")
                    .map(|st| st.unwrap_or_default())
                    .unwrap_or_default(),
            };
            env.tx.deposit = deposit;
        }

        env
    }

    /// Builds [`Inspector`] with the configured options.
    fn build_inspector(&self) -> AnvilInspector {
        let mut inspector = AnvilInspector::default();

        if self.print_logs {
            inspector = inspector.with_log_collector();
        }
        if self.print_traces {
            inspector = inspector.with_trace_printer();
        }

        inspector
    }

    /// Simulates the payload by executing the calls in request.
    pub async fn simulate(
        &self,
        request: SimulatePayload,
        block_request: Option<BlockRequest>,
    ) -> Result<Vec<SimulatedBlock<AnyRpcBlock>>, BlockchainError> {
        self.with_database_at(block_request, |state, mut block_env| {
            let SimulatePayload {
                block_state_calls,
                trace_transfers,
                validation,
                return_full_transactions,
            } = request;
            let mut cache_db = CacheDB::new(state);
            let mut block_res = Vec::with_capacity(block_state_calls.len());

            // execute the blocks
            for block in block_state_calls {
                let SimBlock { block_overrides, state_overrides, calls } = block;
                let mut call_res = Vec::with_capacity(calls.len());
                let mut log_index = 0;
                let mut gas_used = 0;
                let mut transactions = Vec::with_capacity(calls.len());
                let mut logs= Vec::new();

                // apply state overrides before executing the transactions
                if let Some(state_overrides) = state_overrides {
                    apply_state_overrides(state_overrides, &mut cache_db)?;
                }
                if let Some(block_overrides) = block_overrides {
                    cache_db.apply_block_overrides(block_overrides, &mut block_env);
                }

                // execute all calls in that block
                for (req_idx, request) in calls.into_iter().enumerate() {
                    let fee_details = FeeDetails::new(
                        request.gas_price,
                        request.max_fee_per_gas,
                        request.max_priority_fee_per_gas,
                        request.max_fee_per_blob_gas,
                    )?
                    .or_zero_fees();

                    let mut env = self.build_call_env(
                        WithOtherFields::new(request.clone()),
                        fee_details,
                        block_env.clone(),
                    );

                    // Always disable EIP-3607
                    env.evm_env.cfg_env.disable_eip3607 = true;

                    if !validation {
                        env.evm_env.cfg_env.disable_base_fee = !validation;
                        env.evm_env.block_env.basefee = 0;
                    }

                    let mut inspector = self.build_inspector();

                    // transact
                    let ResultAndState { result, state } = if trace_transfers {
                        // prepare inspector to capture transfer inside the evm so they are
                        // recorded and included in logs
                        inspector = inspector.with_transfers();
                        let mut evm= self.new_evm_with_inspector_ref(
                            &cache_db,
                            &env,
                            &mut inspector,
                        );

                        trace!(target: "backend", env=?env.evm_env, spec=?env.evm_env.spec_id(),"simulate evm env");
                        evm.transact(env.tx)?
                    } else {
                        let mut evm = self.new_evm_with_inspector_ref(
                            &cache_db,
                            &env,
                            &mut inspector,
                        );
                        trace!(target: "backend", env=?env.evm_env, spec=?env.evm_env.spec_id(),"simulate evm env");
                        evm.transact(env.tx)?
                    };
                    trace!(target: "backend", ?result, ?request, "simulate call");

                    inspector.print_logs();
                    if self.print_traces {
                        inspector.into_print_traces(self.call_trace_decoder.clone());
                    }

                    // commit the transaction
                    cache_db.commit(state);
                    gas_used += result.gas_used();

                    // create the transaction from a request
                    let from = request.from.unwrap_or_default();
                    let request = transaction_request_to_typed(WithOtherFields::new(request))
                        .ok_or(BlockchainError::MissingRequiredFields)?;
                    let tx = build_typed_transaction(
                        request,
                        Signature::new(Default::default(), Default::default(), false),
                    )?;
                    let tx_hash = tx.hash();
                    let rpc_tx = transaction_build(
                        None,
                        MaybeImpersonatedTransaction::impersonated(tx, from),
                        None,
                        None,
                        Some(block_env.basefee),
                    );
                    transactions.push(rpc_tx);

                    let return_data = result.output().cloned().unwrap_or_default();
                    let sim_res = SimCallResult {
                        return_data,
                        gas_used: result.gas_used(),
                        status: result.is_success(),
                        error: result.is_success().not().then(|| {
                            alloy_rpc_types::simulate::SimulateError {
                                code: -3200,
                                message: "execution failed".to_string(),
                            }
                        }),
                        logs: result.clone()
                            .into_logs()
                            .into_iter()
                            .enumerate()
                            .map(|(idx, log)| Log {
                                inner: log,
                                block_number: Some(block_env.number.saturating_to()),
                                block_timestamp: Some(block_env.timestamp.saturating_to()),
                                transaction_index: Some(req_idx as u64),
                                log_index: Some((idx + log_index) as u64),
                                removed: false,

                                block_hash: None,
                                transaction_hash: Some(tx_hash),
                            })
                            .collect(),
                    };
                    logs.extend(sim_res.logs.clone().iter().map(|log| log.inner.clone()));
                    log_index += sim_res.logs.len();
                    call_res.push(sim_res);
                }

                let transactions_envelopes: Vec<AnyTxEnvelope> = transactions
                .iter()
                .map(|tx| AnyTxEnvelope::from(tx.clone()))
                .collect();
                let header = Header {
                    logs_bloom: logs_bloom(logs.iter()),
                    transactions_root: calculate_transaction_root(&transactions_envelopes),
                    receipts_root: calculate_receipt_root(&transactions_envelopes),
                    parent_hash: Default::default(),
                    beneficiary: block_env.beneficiary,
                    state_root: Default::default(),
                    difficulty: Default::default(),
                    number: block_env.number.saturating_to(),
                    gas_limit: block_env.gas_limit,
                    gas_used,
                    timestamp: block_env.timestamp.saturating_to(),
                    extra_data: Default::default(),
                    mix_hash: Default::default(),
                    nonce: Default::default(),
                    base_fee_per_gas: Some(block_env.basefee),
                    withdrawals_root: None,
                    blob_gas_used: None,
                    excess_blob_gas: None,
                    parent_beacon_block_root: None,
                    requests_hash: None,
                    ..Default::default()
                };
                let mut block = alloy_rpc_types::Block {
                    header: AnyRpcHeader {
                        hash: header.hash_slow(),
                        inner: header.into(),
                        total_difficulty: None,
                        size: None,
                    },
                    uncles: vec![],
                    transactions: BlockTransactions::Full(transactions),
                    withdrawals: None,
                };

                if !return_full_transactions {
                    block.transactions.convert_to_hashes();
                }

                for res in &mut call_res {
                    res.logs.iter_mut().for_each(|log| {
                        log.block_hash = Some(block.header.hash);
                    });
                }

                let simulated_block = SimulatedBlock {
                    inner: AnyRpcBlock::new(WithOtherFields::new(block)),
                    calls: call_res,
                };

                // update block env
                block_env.number += U256::from(1);
                block_env.timestamp += U256::from(12);
                block_env.basefee = simulated_block
                    .inner
                    .header
                    .next_block_base_fee(BaseFeeParams::ethereum())
                    .unwrap_or_default();

                block_res.push(simulated_block);
            }

            Ok(block_res)
        })
        .await?
    }

    pub fn call_with_state(
        &self,
        state: &dyn DatabaseRef,
        request: WithOtherFields<TransactionRequest>,
        fee_details: FeeDetails,
        block_env: BlockEnv,
    ) -> Result<(InstructionResult, Option<Output>, u128, State), BlockchainError> {
        let mut inspector = self.build_inspector();

        let env = self.build_call_env(request, fee_details, block_env);
        let mut evm = self.new_evm_with_inspector_ref(state, &env, &mut inspector);
        let ResultAndState { result, state } = evm.transact(env.tx)?;
        let (exit_reason, gas_used, out) = match result {
            ExecutionResult::Success { reason, gas_used, output, .. } => {
                (reason.into(), gas_used, Some(output))
            }
            ExecutionResult::Revert { gas_used, output } => {
                (InstructionResult::Revert, gas_used, Some(Output::Call(output)))
            }
            ExecutionResult::Halt { reason, gas_used } => {
                (op_haltreason_to_instruction_result(reason), gas_used, None)
            }
        };
        drop(evm);
        inspector.print_logs();

        if self.print_traces {
            inspector.into_print_traces(self.call_trace_decoder.clone());
        }

        Ok((exit_reason, out, gas_used as u128, state))
    }

    pub async fn call_with_tracing(
        &self,
        request: WithOtherFields<TransactionRequest>,
        fee_details: FeeDetails,
        block_request: Option<BlockRequest>,
        opts: GethDebugTracingCallOptions,
    ) -> Result<GethTrace, BlockchainError> {
        let GethDebugTracingCallOptions {
            tracing_options, block_overrides, state_overrides, ..
        } = opts;
        let GethDebugTracingOptions { config, tracer, tracer_config, .. } = tracing_options;

        self.with_database_at(block_request, |state, mut block| {
            let block_number = block.number;

            let mut cache_db = CacheDB::new(state);
            if let Some(state_overrides) = state_overrides {
                apply_state_overrides(state_overrides, &mut cache_db)?;
            }
            if let Some(block_overrides) = block_overrides {
                cache_db.apply_block_overrides(block_overrides, &mut block);
            }

            if let Some(tracer) = tracer {
                return match tracer {
                    GethDebugTracerType::BuiltInTracer(tracer) => match tracer {
                        GethDebugBuiltInTracerType::CallTracer => {
                            let call_config = tracer_config
                                .into_call_config()
                                .map_err(|e| RpcError::invalid_params(e.to_string()))?;

                            let mut inspector = self.build_inspector().with_tracing_config(
                                TracingInspectorConfig::from_geth_call_config(&call_config),
                            );

                            let env = self.build_call_env(request, fee_details, block);
                            let mut evm =
                                self.new_evm_with_inspector_ref(&cache_db, &env, &mut inspector);
                            let ResultAndState { result, state: _ } = evm.transact(env.tx)?;

                            drop(evm);

                            inspector.print_logs();
                            if self.print_traces {
                                inspector.print_traces(self.call_trace_decoder.clone());
                            }

                            let tracing_inspector = inspector.tracer.expect("tracer disappeared");

                            Ok(tracing_inspector
                                .into_geth_builder()
                                .geth_call_traces(call_config, result.gas_used())
                                .into())
                        }
                        GethDebugBuiltInTracerType::PreStateTracer => {
                            let pre_state_config = tracer_config
                                .into_pre_state_config()
                                .map_err(|e| RpcError::invalid_params(e.to_string()))?;

                            let mut inspector = TracingInspector::new(
                                TracingInspectorConfig::from_geth_prestate_config(
                                    &pre_state_config,
                                ),
                            );

                            let env = self.build_call_env(request, fee_details, block);
                            let mut evm =
                                self.new_evm_with_inspector_ref(&cache_db, &env, &mut inspector);
                            let result = evm.transact(env.tx)?;

                            drop(evm);

                            Ok(inspector
                                .into_geth_builder()
                                .geth_prestate_traces(&result, &pre_state_config, cache_db)?
                                .into())
                        }
                        GethDebugBuiltInTracerType::NoopTracer => Ok(NoopFrame::default().into()),
                        GethDebugBuiltInTracerType::FourByteTracer
                        | GethDebugBuiltInTracerType::MuxTracer
                        | GethDebugBuiltInTracerType::FlatCallTracer => {
                            Err(RpcError::invalid_params("unsupported tracer type").into())
                        }
                    },
                    #[cfg(not(feature = "js-tracer"))]
                    GethDebugTracerType::JsTracer(_) => {
                        Err(RpcError::invalid_params("unsupported tracer type").into())
                    }
                    #[cfg(feature = "js-tracer")]
                    GethDebugTracerType::JsTracer(code) => {
                        use alloy_evm::IntoTxEnv;
                        let config = tracer_config.into_json();
                        let mut inspector =
                            revm_inspectors::tracing::js::JsInspector::new(code, config)
                                .map_err(|err| BlockchainError::Message(err.to_string()))?;

                        let env = self.build_call_env(request, fee_details, block.clone());
                        let mut evm =
                            self.new_evm_with_inspector_ref(&cache_db, &env, &mut inspector);
                        let result = evm.transact(env.tx.clone())?;
                        let res = evm
                            .inspector_mut()
                            .json_result(result, &env.tx.into_tx_env(), &block, &cache_db)
                            .map_err(|err| BlockchainError::Message(err.to_string()))?;

                        Ok(GethTrace::JS(res))
                    }
                };
            }

            // defaults to StructLog tracer used since no tracer is specified
            let mut inspector = self
                .build_inspector()
                .with_tracing_config(TracingInspectorConfig::from_geth_config(&config));

            let env = self.build_call_env(request, fee_details, block);
            let mut evm = self.new_evm_with_inspector_ref(&cache_db, &env, &mut inspector);
            let ResultAndState { result, state: _ } = evm.transact(env.tx)?;

            let (exit_reason, gas_used, out) = match result {
                ExecutionResult::Success { reason, gas_used, output, .. } => {
                    (reason.into(), gas_used, Some(output))
                }
                ExecutionResult::Revert { gas_used, output } => {
                    (InstructionResult::Revert, gas_used, Some(Output::Call(output)))
                }
                ExecutionResult::Halt { reason, gas_used } => {
                    (op_haltreason_to_instruction_result(reason), gas_used, None)
                }
            };

            drop(evm);
            let tracing_inspector = inspector.tracer.expect("tracer disappeared");
            let return_value = out.as_ref().map(|o| o.data().clone()).unwrap_or_default();

            trace!(target: "backend", ?exit_reason, ?out, %gas_used, %block_number, "trace call");

            let res = tracing_inspector
                .into_geth_builder()
                .geth_traces(gas_used, return_value, config)
                .into();

            Ok(res)
        })
        .await?
    }

    pub fn build_access_list_with_state(
        &self,
        state: &dyn DatabaseRef,
        request: WithOtherFields<TransactionRequest>,
        fee_details: FeeDetails,
        block_env: BlockEnv,
    ) -> Result<(InstructionResult, Option<Output>, u64, AccessList), BlockchainError> {
        let mut inspector =
            AccessListInspector::new(request.access_list.clone().unwrap_or_default());

        let env = self.build_call_env(request, fee_details, block_env);
        let mut evm = self.new_evm_with_inspector_ref(state, &env, &mut inspector);
        let ResultAndState { result, state: _ } = evm.transact(env.tx)?;
        let (exit_reason, gas_used, out) = match result {
            ExecutionResult::Success { reason, gas_used, output, .. } => {
                (reason.into(), gas_used, Some(output))
            }
            ExecutionResult::Revert { gas_used, output } => {
                (InstructionResult::Revert, gas_used, Some(Output::Call(output)))
            }
            ExecutionResult::Halt { reason, gas_used } => {
                (op_haltreason_to_instruction_result(reason), gas_used, None)
            }
        };
        drop(evm);
        let access_list = inspector.access_list();
        Ok((exit_reason, out, gas_used, access_list))
    }

    /// returns all receipts for the given transactions
    fn get_receipts(&self, tx_hashes: impl IntoIterator<Item = TxHash>) -> Vec<TypedReceipt> {
        let storage = self.blockchain.storage.read();
        let mut receipts = vec![];

        for hash in tx_hashes {
            if let Some(tx) = storage.transactions.get(&hash) {
                receipts.push(tx.receipt.clone());
            }
        }

        receipts
    }

    /// Returns the logs of the block that match the filter
    async fn logs_for_block(
        &self,
        filter: Filter,
        hash: B256,
    ) -> Result<Vec<Log>, BlockchainError> {
        if let Some(block) = self.blockchain.get_block_by_hash(&hash) {
            return Ok(self.mined_logs_for_block(filter, block, hash));
        }

        if let Some(fork) = self.get_fork() {
            return Ok(fork.logs(&filter).await?);
        }

        Ok(Vec::new())
    }

    /// Returns all `Log`s mined by the node that were emitted in the `block` and match the `Filter`
    fn mined_logs_for_block(&self, filter: Filter, block: Block, block_hash: B256) -> Vec<Log> {
        let mut all_logs = Vec::new();
        let mut block_log_index = 0u32;

        let storage = self.blockchain.storage.read();

        for tx in block.body.transactions {
            let Some(tx) = storage.transactions.get(&tx.hash()) else {
                continue;
            };

            let logs = tx.receipt.logs();
            let transaction_hash = tx.info.transaction_hash;

            for log in logs {
                if filter.matches(log) {
                    all_logs.push(Log {
                        inner: log.clone(),
                        block_hash: Some(block_hash),
                        block_number: Some(block.header.number),
                        block_timestamp: Some(block.header.timestamp),
                        transaction_hash: Some(transaction_hash),
                        transaction_index: Some(tx.info.transaction_index),
                        log_index: Some(block_log_index as u64),
                        removed: false,
                    });
                }
                block_log_index += 1;
            }
        }
        all_logs
    }

    /// Returns the logs that match the filter in the given range of blocks
    async fn logs_for_range(
        &self,
        filter: &Filter,
        mut from: u64,
        to: u64,
    ) -> Result<Vec<Log>, BlockchainError> {
        let mut all_logs = Vec::new();

        // get the range that predates the fork if any
        if let Some(fork) = self.get_fork() {
            let mut to_on_fork = to;

            if !fork.predates_fork(to) {
                // adjust the ranges
                to_on_fork = fork.block_number();
            }

            if fork.predates_fork_inclusive(from) {
                // this data is only available on the forked client
                let filter = filter.clone().from_block(from).to_block(to_on_fork);
                all_logs = fork.logs(&filter).await?;

                // update the range
                from = fork.block_number() + 1;
            }
        }

        for number in from..=to {
            if let Some((block, hash)) = self.get_block_with_hash(number) {
                all_logs.extend(self.mined_logs_for_block(filter.clone(), block, hash));
            }
        }

        Ok(all_logs)
    }

    /// Returns the logs according to the filter
    pub async fn logs(&self, filter: Filter) -> Result<Vec<Log>, BlockchainError> {
        trace!(target: "backend", "get logs [{:?}]", filter);
        if let Some(hash) = filter.get_block_hash() {
            self.logs_for_block(filter, hash).await
        } else {
            let best = self.best_number();
            let to_block =
                self.convert_block_number(filter.block_option.get_to_block().copied()).min(best);
            let from_block =
                self.convert_block_number(filter.block_option.get_from_block().copied());
            if from_block > best {
                // requested log range does not exist yet
                return Ok(vec![]);
            }

            self.logs_for_range(&filter, from_block, to_block).await
        }
    }

    pub async fn block_by_hash(&self, hash: B256) -> Result<Option<AnyRpcBlock>, BlockchainError> {
        trace!(target: "backend", "get block by hash {:?}", hash);
        if let tx @ Some(_) = self.mined_block_by_hash(hash) {
            return Ok(tx);
        }

        if let Some(fork) = self.get_fork() {
            let is_offline = self.node_config.read().await.offline;
            if is_offline {
                // In offline mode, only return blocks from local storage
                // Check if the block exists in our loaded state
                if let Some(block) = self.blockchain.get_block_by_hash(&hash) {
                    return Ok(Some(self.convert_block(block)));
                }
                return Ok(None);
            }
            return Ok(fork.block_by_hash(hash).await?);
        }

        Ok(None)
    }

    pub async fn block_by_hash_full(
        &self,
        hash: B256,
    ) -> Result<Option<AnyRpcBlock>, BlockchainError> {
        trace!(target: "backend", "get block by hash {:?}", hash);
        if let tx @ Some(_) = self.get_full_block(hash) {
            return Ok(tx);
        }

        if let Some(fork) = self.get_fork() {
            let is_offline = self.node_config.read().await.offline;
            if is_offline {
                // In offline mode, check local storage for the block
                if let Some(block) = self.blockchain.get_block_by_hash(&hash) {
                    let transactions = self.mined_transactions_in_block(&block).unwrap_or_default();
                    let mut rpc_block = self.convert_block(block);
                    rpc_block.transactions = BlockTransactions::Full(transactions);
                    return Ok(Some(rpc_block));
                }
                return Ok(None);
            }
            return Ok(fork.block_by_hash_full(hash).await?);
        }

        Ok(None)
    }

    fn mined_block_by_hash(&self, hash: B256) -> Option<AnyRpcBlock> {
        let block = self.blockchain.get_block_by_hash(&hash)?;
        Some(self.convert_block_with_hash(block, Some(hash)))
    }

    pub(crate) async fn mined_transactions_by_block_number(
        &self,
        number: BlockNumber,
    ) -> Option<Vec<AnyRpcTransaction>> {
        if let Some(block) = self.get_block(number) {
            return self.mined_transactions_in_block(&block);
        }
        None
    }

    /// Returns all transactions given a block
    pub(crate) fn mined_transactions_in_block(
        &self,
        block: &Block,
    ) -> Option<Vec<AnyRpcTransaction>> {
        let mut transactions = Vec::with_capacity(block.body.transactions.len());
        let base_fee = block.header.base_fee_per_gas;
        let storage = self.blockchain.storage.read();
        for hash in block.body.transactions.iter().map(|tx| tx.hash()) {
            let info = storage.transactions.get(&hash)?.info.clone();
            let tx = block.body.transactions.get(info.transaction_index as usize)?.clone();

            let tx = transaction_build(Some(hash), tx, Some(block), Some(info), base_fee);
            transactions.push(tx);
        }
        Some(transactions)
    }

    pub async fn block_by_number(
        &self,
        number: BlockNumber,
    ) -> Result<Option<AnyRpcBlock>, BlockchainError> {
        trace!(target: "backend", "get block by number {:?}", number);
        if let tx @ Some(_) = self.mined_block_by_number(number) {
            return Ok(tx);
        }

        if let Some(fork) = self.get_fork() {
            let number = self.convert_block_number(Some(number));
            if fork.predates_fork_inclusive(number) {
                let is_offline = self.node_config.read().await.offline;
                if is_offline {
                    // In offline mode, check local storage for the block
                    if let Some(hash) =
                        self.blockchain.hash(BlockId::Number(BlockNumber::Number(number)))
                    {
                        if let Some(block) = self.blockchain.get_block_by_hash(&hash) {
                            let mut rpc_block = self.convert_block(block);
                            rpc_block.transactions.convert_to_hashes();
                            return Ok(Some(rpc_block));
                        }
                    }
                    return Ok(None);
                }
                return Ok(fork.block_by_number(number).await?);
            }
        }

        Ok(None)
    }

    pub async fn block_by_number_full(
        &self,
        number: BlockNumber,
    ) -> Result<Option<AnyRpcBlock>, BlockchainError> {
        trace!(target: "backend", "get block by number {:?}", number);
        if let tx @ Some(_) = self.get_full_block(number) {
            return Ok(tx);
        }

        if let Some(fork) = self.get_fork() {
            let number = self.convert_block_number(Some(number));
            if fork.predates_fork_inclusive(number) {
                let is_offline = self.node_config.read().await.offline;
                if is_offline {
                    // In offline mode, check local storage for the block
                    if let Some(hash) =
                        self.blockchain.hash(BlockId::Number(BlockNumber::Number(number)))
                    {
                        if let Some(block) = self.blockchain.get_block_by_hash(&hash) {
                            let transactions =
                                self.mined_transactions_in_block(&block).unwrap_or_default();
                            let mut rpc_block = self.convert_block(block);
                            rpc_block.transactions = BlockTransactions::Full(transactions);
                            return Ok(Some(rpc_block));
                        }
                    }
                    return Ok(None);
                }
                return Ok(fork.block_by_number_full(number).await?);
            }
        }

        Ok(None)
    }

    /// Returns the block and its hash for the given id
    fn get_block_with_hash(&self, id: impl Into<BlockId>) -> Option<(Block, B256)> {
        let hash = match id.into() {
            BlockId::Hash(hash) => hash.block_hash,
            BlockId::Number(number) => {
                let storage = self.blockchain.storage.read();
                let slots_in_an_epoch = self.slots_in_an_epoch;
                match number {
                    BlockNumber::Latest => storage.best_hash,
                    BlockNumber::Earliest => storage.genesis_hash,
                    BlockNumber::Pending => return None,
                    BlockNumber::Number(num) => *storage.hashes.get(&num)?,
                    BlockNumber::Safe => {
                        if storage.best_number > (slots_in_an_epoch) {
                            *storage.hashes.get(&(storage.best_number - (slots_in_an_epoch)))?
                        } else {
                            storage.genesis_hash // treat the genesis block as safe "by definition"
                        }
                    }
                    BlockNumber::Finalized => {
                        if storage.best_number > (slots_in_an_epoch * 2) {
                            *storage.hashes.get(&(storage.best_number - (slots_in_an_epoch * 2)))?
                        } else {
                            storage.genesis_hash
                        }
                    }
                }
            }
        };
        let block = self.get_block_by_hash(hash)?;
        Some((block, hash))
    }

    pub fn get_block(&self, id: impl Into<BlockId>) -> Option<Block> {
        self.get_block_with_hash(id).map(|(block, _)| block)
    }

    pub fn get_block_by_hash(&self, hash: B256) -> Option<Block> {
        self.blockchain.get_block_by_hash(&hash)
    }

    pub fn mined_block_by_number(&self, number: BlockNumber) -> Option<AnyRpcBlock> {
        let (block, hash) = self.get_block_with_hash(number)?;
        let mut block = self.convert_block_with_hash(block, Some(hash));
        block.transactions.convert_to_hashes();
        Some(block)
    }

    pub fn get_full_block(&self, id: impl Into<BlockId>) -> Option<AnyRpcBlock> {
        let (block, hash) = self.get_block_with_hash(id)?;
        let transactions = self.mined_transactions_in_block(&block)?;
        let mut block = self.convert_block_with_hash(block, Some(hash));
        block.inner.transactions = BlockTransactions::Full(transactions);
        Some(block)
    }

    /// Takes a block as it's stored internally and returns the eth api conform block format.
    pub fn convert_block(&self, block: Block) -> AnyRpcBlock {
        self.convert_block_with_hash(block, None)
    }

    /// Takes a block as it's stored internally and returns the eth api conform block format.
    /// If `known_hash` is provided, it will be used instead of computing `hash_slow()`.
    pub fn convert_block_with_hash(&self, block: Block, known_hash: Option<B256>) -> AnyRpcBlock {
        let size = U256::from(alloy_rlp::encode(&block).len() as u32);

        let header = block.header.clone();
        let transactions = block.body.transactions;

        let hash = known_hash.unwrap_or_else(|| header.hash_slow());
        let Header { number, withdrawals_root, .. } = header;

        let block = AlloyBlock {
            header: AlloyHeader {
                inner: AnyHeader::from(header),
                hash,
                total_difficulty: Some(self.total_difficulty()),
                size: Some(size),
            },
            transactions: alloy_rpc_types::BlockTransactions::Hashes(
                transactions.into_iter().map(|tx| tx.hash()).collect(),
            ),
            uncles: vec![],
            withdrawals: withdrawals_root.map(|_| Default::default()),
        };

        let mut block = WithOtherFields::new(block);

        // If Arbitrum, apply chain specifics to converted block.
        if is_arbitrum(self.env.read().evm_env.cfg_env.chain_id) {
            // Set `l1BlockNumber` field.
            block.other.insert("l1BlockNumber".to_string(), number.into());
        }

        AnyRpcBlock::from(block)
    }

    /// Converts the `BlockNumber` into a numeric value
    ///
    /// # Errors
    ///
    /// returns an error if the requested number is larger than the current height
    pub async fn ensure_block_number<T: Into<BlockId>>(
        &self,
        block_id: Option<T>,
    ) -> Result<u64, BlockchainError> {
        let current = self.best_number();
        let requested =
            match block_id.map(Into::into).unwrap_or(BlockId::Number(BlockNumber::Latest)) {
                BlockId::Hash(hash) => {
                    self.block_by_hash(hash.block_hash)
                        .await?
                        .ok_or(BlockchainError::BlockNotFound)?
                        .header
                        .number
                }
                BlockId::Number(num) => match num {
                    BlockNumber::Latest | BlockNumber::Pending => current,
                    BlockNumber::Earliest => U64::ZERO.to::<u64>(),
                    BlockNumber::Number(num) => num,
                    BlockNumber::Safe => current.saturating_sub(self.slots_in_an_epoch),
                    BlockNumber::Finalized => current.saturating_sub(self.slots_in_an_epoch * 2),
                },
            };

        if requested > current {
            Err(BlockchainError::BlockOutOfRange(current, requested))
        } else {
            Ok(requested)
        }
    }

    pub fn convert_block_number(&self, block: Option<BlockNumber>) -> u64 {
        let current = self.best_number();
        match block.unwrap_or(BlockNumber::Latest) {
            BlockNumber::Latest | BlockNumber::Pending => current,
            BlockNumber::Earliest => 0,
            BlockNumber::Number(num) => num,
            BlockNumber::Safe => current.saturating_sub(self.slots_in_an_epoch),
            BlockNumber::Finalized => current.saturating_sub(self.slots_in_an_epoch * 2),
        }
    }

    /// Helper function to execute a closure with the database at a specific block
    pub async fn with_database_at<F, T>(
        &self,
        block_request: Option<BlockRequest>,
        f: F,
    ) -> Result<T, BlockchainError>
    where
        F: FnOnce(Box<dyn MaybeFullDatabase + '_>, BlockEnv) -> T,
    {
        let block_number = match block_request {
            Some(BlockRequest::Pending(pool_transactions)) => {
                let result = self
                    .with_pending_block(pool_transactions, |state, block| {
                        let block = block.block;
                        let block = BlockEnv {
                            number: U256::from(block.header.number),
                            beneficiary: block.header.beneficiary,
                            timestamp: U256::from(block.header.timestamp),
                            difficulty: block.header.difficulty,
                            prevrandao: Some(block.header.mix_hash),
                            basefee: block.header.base_fee_per_gas.unwrap_or_default(),
                            gas_limit: block.header.gas_limit,
                            ..Default::default()
                        };
                        f(state, block)
                    })
                    .await;
                return Ok(result);
            }
            Some(BlockRequest::Number(bn)) => Some(BlockNumber::Number(bn)),
            None => None,
        };
        let block_number = self.convert_block_number(block_number);
        let current_block = self.env.read().evm_env.block_env.number.saturating_to::<u64>();
        let is_offline = self.node_config.read().await.offline;

        if block_number < current_block {
            if let Some((block_hash, block)) = self
                .block_by_number(BlockNumber::Number(block_number))
                .await?
                .map(|block| (block.header.hash, block))
            {
                let read_guard = self.states.upgradable_read();
                if let Some(state_db) = read_guard.get_state(&block_hash) {
                    return Ok(get_block_env(state_db, block_number, block, is_offline, f));
                } else {
                    let mut write_guard = RwLockUpgradableReadGuard::upgrade(read_guard);
                    if let Some(state) = write_guard.get_on_disk_state(&block_hash) {
                        return Ok(get_block_env(state, block_number, block, is_offline, f));
                    }
                }
            }

            warn!(target: "backend", "Not historic state found for block={}", block_number);
            return Err(BlockchainError::BlockOutOfRange(
                self.env.read().evm_env.block_env.number.saturating_to(),
                block_number,
            ));
        }

        let db = self.db.read().await;
        let block = self.env.read().evm_env.block_env.clone();
        Ok(f(Box::new(&**db), block))
    }

    pub async fn storage_at(
        &self,
        address: Address,
        index: U256,
        block_request: Option<BlockRequest>,
    ) -> Result<B256, BlockchainError> {
        self.with_database_at(block_request, |db, _| {
            trace!(target: "backend", "get storage for {:?} at {:?}", address, index);
            let val = db.storage_ref(address, index)?;
            Ok(val.into())
        })
        .await?
    }

    /// Returns the code of the address
    ///
    /// If the code is not present and fork mode is enabled then this will try to fetch it from the
    /// forked client
    pub async fn get_code(
        &self,
        address: Address,
        block_request: Option<BlockRequest>,
    ) -> Result<Bytes, BlockchainError> {
        self.with_database_at(block_request, |db, _| self.get_code_with_state(&db, address)).await?
    }

    pub fn get_code_with_state(
        &self,
        state: &dyn DatabaseRef,
        address: Address,
    ) -> Result<Bytes, BlockchainError> {
        trace!(target: "backend", "get code for {:?}", address);
        let account = state.basic_ref(address)?.unwrap_or_default();
        if account.code_hash == KECCAK_EMPTY {
            // if the code hash is `KECCAK_EMPTY`, we check no further
            return Ok(Default::default());
        }
        let code = if let Some(code) = account.code {
            code
        } else {
            state.code_by_hash_ref(account.code_hash)?
        };
        Ok(code.bytes()[..code.len()].to_vec().into())
    }

    /// Returns the balance of the address
    ///
    /// If the requested number predates the fork then this will fetch it from the endpoint
    pub async fn get_balance(
        &self,
        address: Address,
        block_request: Option<BlockRequest>,
    ) -> Result<U256, BlockchainError> {
        self.with_database_at(block_request, |db, _| self.get_balance_with_state(db, address))
            .await?
    }

    pub async fn get_account_at_block(
        &self,
        address: Address,
        block_request: Option<BlockRequest>,
    ) -> Result<Account, BlockchainError> {
        self.with_database_at(block_request, |block_db, _| {
            let db = block_db.maybe_as_full_db().ok_or(BlockchainError::DataUnavailable)?;
            let account = db.get(&address).cloned().unwrap_or_default();
            let storage_root = storage_root(&account.storage);
            let code_hash = account.info.code_hash;
            let balance = account.info.balance;
            let nonce = account.info.nonce;
            Ok(Account { balance, nonce, code_hash, storage_root })
        })
        .await?
    }

    pub fn get_balance_with_state<D>(
        &self,
        state: D,
        address: Address,
    ) -> Result<U256, BlockchainError>
    where
        D: DatabaseRef,
    {
        trace!(target: "backend", "get balance for {:?}", address);
        Ok(state.basic_ref(address)?.unwrap_or_default().balance)
    }

    /// Returns the nonce of the address
    ///
    /// If the requested number predates the fork then this will fetch it from the endpoint
    pub async fn get_nonce(
        &self,
        address: Address,
        block_request: BlockRequest,
    ) -> Result<u64, BlockchainError> {
        if let BlockRequest::Pending(pool_transactions) = &block_request
            && let Some(value) = get_pool_transactions_nonce(pool_transactions, address)
        {
            return Ok(value);
        }
        let final_block_request = match block_request {
            BlockRequest::Pending(_) => BlockRequest::Number(self.best_number()),
            BlockRequest::Number(bn) => BlockRequest::Number(bn),
        };

        self.with_database_at(Some(final_block_request), |db, _| {
            trace!(target: "backend", "get nonce for {:?}", address);
            Ok(db.basic_ref(address)?.unwrap_or_default().nonce)
        })
        .await?
    }

    /// Returns the traces for the given transaction
    pub async fn trace_transaction(
        &self,
        hash: B256,
    ) -> Result<Vec<LocalizedTransactionTrace>, BlockchainError> {
        if let Some(traces) = self.mined_parity_trace_transaction(hash) {
            return Ok(traces);
        }

        if let Some(fork) = self.get_fork() {
            return Ok(fork.trace_transaction(hash).await?);
        }

        Ok(vec![])
    }

    /// Returns the traces for the given transaction
    pub(crate) fn mined_parity_trace_transaction(
        &self,
        hash: B256,
    ) -> Option<Vec<LocalizedTransactionTrace>> {
        self.blockchain.storage.read().transactions.get(&hash).map(|tx| tx.parity_traces())
    }

    /// Returns the traces for the given transaction
    pub(crate) fn mined_transaction(&self, hash: B256) -> Option<MinedTransaction> {
        self.blockchain.storage.read().transactions.get(&hash).cloned()
    }

    /// Returns the traces for the given block
    pub(crate) fn mined_parity_trace_block(
        &self,
        block: u64,
    ) -> Option<Vec<LocalizedTransactionTrace>> {
        let block = self.get_block(block)?;
        let mut traces = vec![];
        let storage = self.blockchain.storage.read();
        for tx in block.body.transactions {
            traces.extend(storage.transactions.get(&tx.hash())?.parity_traces());
        }
        Some(traces)
    }

    /// Returns the traces for the given transaction
    pub async fn debug_trace_transaction(
        &self,
        hash: B256,
        opts: GethDebugTracingOptions,
    ) -> Result<GethTrace, BlockchainError> {
        #[cfg(feature = "js-tracer")]
        if let Some(tracer_type) = opts.tracer.as_ref()
            && tracer_type.is_js()
        {
            return self
                .trace_tx_with_js_tracer(hash, tracer_type.as_str().to_string(), opts.clone())
                .await;
        }

        if let Some(trace) = self.mined_geth_trace_transaction(hash, opts.clone()).await {
            return trace;
        }

        if let Some(fork) = self.get_fork() {
            return Ok(fork.debug_trace_transaction(hash, opts).await?);
        }

        Ok(GethTrace::Default(Default::default()))
    }

    fn replay_tx_with_inspector<I, F, T>(
        &self,
        hash: B256,
        mut inspector: I,
        f: F,
    ) -> Result<T, BlockchainError>
    where
        for<'a> I: Inspector<EthEvmContext<WrapDatabaseRef<&'a CacheDB<Box<&'a StateDb>>>>>
            + Inspector<OpContext<WrapDatabaseRef<&'a CacheDB<Box<&'a StateDb>>>>>
            + 'a,
        for<'a> F:
            FnOnce(ResultAndState<OpHaltReason>, CacheDB<Box<&'a StateDb>>, I, TxEnv, Env) -> T,
    {
        let block = {
            let storage = self.blockchain.storage.read();
            let MinedTransaction { block_hash, .. } = storage
                .transactions
                .get(&hash)
                .cloned()
                .ok_or(BlockchainError::TransactionNotFound)?;

            storage.blocks.get(&block_hash).cloned().ok_or(BlockchainError::BlockNotFound)?
        };

        let index = block
            .body
            .transactions
            .iter()
            .position(|tx| tx.hash() == hash)
            .expect("transaction not found in block");

        let pool_txs: Vec<Arc<PoolTransaction>> = block.body.transactions[..index]
            .iter()
            .map(|tx| {
                let pending_tx =
                    PendingTransaction::from_maybe_impersonated(tx.clone()).expect("is valid");
                Arc::new(PoolTransaction {
                    pending_transaction: pending_tx,
                    requires: vec![],
                    provides: vec![],
                    priority: crate::eth::pool::transactions::TransactionPriority(0),
                })
            })
            .collect();

        let trace = |parent_state: &StateDb| -> Result<T, BlockchainError> {
            let mut cache_db = CacheDB::new(Box::new(parent_state));

            // configure the blockenv for the block of the transaction
            let mut env = self.env.read().clone();

            env.evm_env.block_env = BlockEnv {
                number: U256::from(block.header.number),
                beneficiary: block.header.beneficiary,
                timestamp: U256::from(block.header.timestamp),
                difficulty: block.header.difficulty,
                prevrandao: Some(block.header.mix_hash),
                basefee: block.header.base_fee_per_gas.unwrap_or_default(),
                gas_limit: block.header.gas_limit,
                ..Default::default()
            };

            let executor = TransactionExecutor {
                db: &mut cache_db,
                validator: self,
                pending: pool_txs.into_iter(),
                block_env: env.evm_env.block_env.clone(),
                cfg_env: env.evm_env.cfg_env.clone(),
                parent_hash: block.header.parent_hash,
                gas_used: 0,
                blob_gas_used: 0,
                enable_steps_tracing: self.enable_steps_tracing,
                print_logs: self.print_logs,
                print_traces: self.print_traces,
                call_trace_decoder: self.call_trace_decoder.clone(),
                precompile_factory: self.precompile_factory.clone(),
                networks: self.env.read().networks,
                blob_params: self.blob_params(),
                cheats: self.cheats().clone(),
            };

            let _ = executor.execute();

            let target_tx = block.body.transactions[index].clone();
            let target_tx = PendingTransaction::from_maybe_impersonated(target_tx)?;
            let mut tx_env: OpTransaction<TxEnv> = FromRecoveredTx::from_recovered_tx(
                target_tx.transaction.as_ref(),
                *target_tx.sender(),
            );
            if env.networks.is_optimism() {
                tx_env.enveloped_tx = Some(target_tx.transaction.encoded_2718().into());
            }

            let mut evm = self.new_evm_with_inspector_ref(&cache_db, &env, &mut inspector);

            let result = evm
                .transact(tx_env.clone())
                .map_err(|err| BlockchainError::Message(err.to_string()))?;

            Ok(f(result, cache_db, inspector, tx_env.base, env))
        };

        let read_guard = self.states.upgradable_read();
        if let Some(state) = read_guard.get_state(&block.header.parent_hash) {
            trace(state)
        } else {
            let mut write_guard = RwLockUpgradableReadGuard::upgrade(read_guard);
            let state = write_guard
                .get_on_disk_state(&block.header.parent_hash)
                .ok_or(BlockchainError::BlockNotFound)?;
            trace(state)
        }
    }

    /// Traces the transaction with the js tracer
    #[cfg(feature = "js-tracer")]
    pub async fn trace_tx_with_js_tracer(
        &self,
        hash: B256,
        code: String,
        opts: GethDebugTracingOptions,
    ) -> Result<GethTrace, BlockchainError> {
        let GethDebugTracingOptions { tracer_config, .. } = opts;
        let config = tracer_config.into_json();
        let inspector = revm_inspectors::tracing::js::JsInspector::new(code, config)
            .map_err(|err| BlockchainError::Message(err.to_string()))?;
        let trace = self.replay_tx_with_inspector(
            hash,
            inspector,
            |result, cache_db, mut inspector, tx_env, env| {
                inspector
                    .json_result(
                        result,
                        &alloy_evm::IntoTxEnv::into_tx_env(tx_env),
                        &env.evm_env.block_env,
                        &cache_db,
                    )
                    .map_err(|e| BlockchainError::Message(e.to_string()))
            },
        )??;
        Ok(GethTrace::JS(trace))
    }

    /// Returns code by its hash
    pub async fn debug_code_by_hash(
        &self,
        code_hash: B256,
        block_id: Option<BlockId>,
    ) -> Result<Option<Bytes>, BlockchainError> {
        if let Ok(code) = self.db.read().await.code_by_hash_ref(code_hash) {
            return Ok(Some(code.original_bytes()));
        }
        if let Some(fork) = self.get_fork() {
            return Ok(fork.debug_code_by_hash(code_hash, block_id).await?);
        }

        Ok(None)
    }

    /// Returns the value associated with a key from the database
    /// Currently only supports bytecode lookups.
    ///
    /// Based on Reth implementation: <https://github.com/paradigmxyz/reth/blob/66cfa9ed1a8c4bc2424aacf6fb2c1e67a78ee9a2/crates/rpc/rpc/src/debug.rs#L1146-L1178>
    ///
    /// Key should be: 0x63 (1-byte prefix) + 32 bytes (code_hash)
    /// Total key length must be 33 bytes.
    pub async fn debug_db_get(&self, key: String) -> Result<Option<Bytes>, BlockchainError> {
        let key_bytes = if key.starts_with("0x") {
            hex::decode(&key)
                .map_err(|_| BlockchainError::Message("Invalid hex key".to_string()))?
        } else {
            key.into_bytes()
        };

        // Validate key length: must be 33 bytes (1 byte prefix + 32 bytes code hash)
        if key_bytes.len() != 33 {
            return Err(BlockchainError::Message(format!(
                "Invalid key length: expected 33 bytes, got {}",
                key_bytes.len()
            )));
        }

        // Check for bytecode prefix (0x63 = 'c' in ASCII)
        if key_bytes[0] != 0x63 {
            return Err(BlockchainError::Message(
                "Key prefix must be 0x63 for code hash lookups".to_string(),
            ));
        }

        let code_hash = B256::from_slice(&key_bytes[1..33]);

        // Use the existing debug_code_by_hash method to retrieve the bytecode
        self.debug_code_by_hash(code_hash, None).await
    }

    fn geth_trace(
        &self,
        tx: &MinedTransaction,
        opts: GethDebugTracingOptions,
    ) -> Result<GethTrace, BlockchainError> {
        let GethDebugTracingOptions { config, tracer, tracer_config, .. } = opts;

        if let Some(tracer) = tracer {
            match tracer {
                GethDebugTracerType::BuiltInTracer(tracer) => match tracer {
                    GethDebugBuiltInTracerType::FourByteTracer => {
                        let inspector = FourByteInspector::default();
                        let res = self.replay_tx_with_inspector(
                            tx.info.transaction_hash,
                            inspector,
                            |_, _, inspector, _, _| FourByteFrame::from(inspector).into(),
                        )?;
                        return Ok(res);
                    }
                    GethDebugBuiltInTracerType::CallTracer => {
                        return match tracer_config.into_call_config() {
                            Ok(call_config) => {
                                let inspector = TracingInspector::new(
                                    TracingInspectorConfig::from_geth_call_config(&call_config),
                                );
                                let frame = self.replay_tx_with_inspector(
                                    tx.info.transaction_hash,
                                    inspector,
                                    |_, _, inspector, _, _| {
                                        inspector
                                            .geth_builder()
                                            .geth_call_traces(
                                                call_config,
                                                tx.receipt.cumulative_gas_used(),
                                            )
                                            .into()
                                    },
                                )?;
                                Ok(frame)
                            }
                            Err(e) => Err(RpcError::invalid_params(e.to_string()).into()),
                        };
                    }
                    GethDebugBuiltInTracerType::PreStateTracer => {
                        return match tracer_config.into_pre_state_config() {
                            Ok(pre_state_config) => {
                                let inspector = TracingInspector::new(
                                    TracingInspectorConfig::from_geth_prestate_config(
                                        &pre_state_config,
                                    ),
                                );
                                let frame = self.replay_tx_with_inspector(
                                    tx.info.transaction_hash,
                                    inspector,
                                    |state, db, inspector, _, _| {
                                        inspector.geth_builder().geth_prestate_traces(
                                            &state,
                                            &pre_state_config,
                                            db,
                                        )
                                    },
                                )??;
                                Ok(frame.into())
                            }
                            Err(e) => Err(RpcError::invalid_params(e.to_string()).into()),
                        };
                    }
                    GethDebugBuiltInTracerType::NoopTracer
                    | GethDebugBuiltInTracerType::MuxTracer
                    | GethDebugBuiltInTracerType::FlatCallTracer => {}
                },
                GethDebugTracerType::JsTracer(_code) => {}
            }

            return Ok(NoopFrame::default().into());
        }

        // default structlog tracer
        Ok(GethTraceBuilder::new(tx.info.traces.clone())
            .geth_traces(
                tx.receipt.cumulative_gas_used(),
                tx.info.out.clone().unwrap_or_default(),
                config,
            )
            .into())
    }

    async fn mined_geth_trace_transaction(
        &self,
        hash: B256,
        opts: GethDebugTracingOptions,
    ) -> Option<Result<GethTrace, BlockchainError>> {
        self.blockchain.storage.read().transactions.get(&hash).map(|tx| self.geth_trace(tx, opts))
    }

    /// Returns geth-style traces for all transactions in a block by hash
    pub async fn debug_trace_block_by_hash(
        &self,
        block_hash: B256,
        opts: GethDebugTracingOptions,
    ) -> Result<Vec<TraceResult>, BlockchainError> {
        // Get block by hash
        if let Some(block) = self.blockchain.get_block_by_hash(&block_hash) {
            // Get all transactions in the block
            let mut traces = Vec::new();
            for tx in &block.transactions {
                let tx_hash = tx.hash();
                match self.debug_trace_transaction(tx_hash, opts.clone()).await {
                    Ok(trace) => {
                        traces.push(TraceResult::Success { result: trace, tx_hash: Some(tx_hash) });
                    }
                    Err(error) => {
                        traces.push(TraceResult::Error {
                            error: error.to_string(),
                            tx_hash: Some(tx_hash),
                        });
                    }
                }
            }
            return Ok(traces);
        }

        // Block not in local storage - try fork
        if let Some(fork) = self.get_fork() {
            let is_offline = self.node_config.read().await.offline;
            if is_offline {
                // In offline mode, block not found
                return Err(BlockchainError::BlockNotFound);
            }
            // In online mode, forward to RPC
            return Ok(fork.debug_trace_block_by_hash(block_hash, opts).await?);
        }

        // No fork and block not found
        Err(BlockchainError::BlockNotFound)
    }

    /// Returns geth-style traces for all transactions in a block by number
    pub async fn debug_trace_block_by_number(
        &self,
        block_number: BlockNumber,
        opts: GethDebugTracingOptions,
    ) -> Result<Vec<TraceResult>, BlockchainError> {
        let number = self.convert_block_number(Some(block_number));

        // Get block by number
        if let Some(block) = self.get_block(BlockId::Number(BlockNumber::Number(number))) {
            // Get all transactions in the block
            let mut traces = Vec::new();
            for tx in &block.transactions {
                let tx_hash = tx.hash();
                match self.debug_trace_transaction(tx_hash, opts.clone()).await {
                    Ok(trace) => {
                        traces.push(TraceResult::Success { result: trace, tx_hash: Some(tx_hash) });
                    }
                    Err(error) => {
                        traces.push(TraceResult::Error {
                            error: error.to_string(),
                            tx_hash: Some(tx_hash),
                        });
                    }
                }
            }
            return Ok(traces);
        }

        // Block not in local storage - try fork
        if let Some(fork) = self.get_fork() {
            if fork.predates_fork_inclusive(number) {
                let is_offline = self.node_config.read().await.offline;
                if is_offline {
                    // In offline mode, block not found
                    return Err(BlockchainError::BlockNotFound);
                }
                // In online mode, forward to RPC
                return Ok(fork.debug_trace_block_by_number(number, opts).await?);
            }
        }

        // No fork and block not found
        Err(BlockchainError::BlockNotFound)
    }

    /// Returns the traces for the given block
    pub async fn trace_block(
        &self,
        block: BlockNumber,
    ) -> Result<Vec<LocalizedTransactionTrace>, BlockchainError> {
        let number = self.convert_block_number(Some(block));
        if let Some(traces) = self.mined_parity_trace_block(number) {
            return Ok(traces);
        }

        if let Some(fork) = self.get_fork()
            && fork.predates_fork(number)
        {
            return Ok(fork.trace_block(number).await?);
        }

        Ok(vec![])
    }

    pub async fn transaction_receipt(
        &self,
        hash: B256,
    ) -> Result<Option<ReceiptResponse>, BlockchainError> {
        if let Some(receipt) = self.mined_transaction_receipt(hash) {
            return Ok(Some(receipt.inner));
        }

        if let Some(fork) = self.get_fork() {
            let receipt = fork.transaction_receipt(hash).await?;
            let number = self.convert_block_number(
                receipt.clone().and_then(|r| r.block_number).map(BlockNumber::from),
            );

            if fork.predates_fork_inclusive(number) {
                return Ok(receipt);
            }
        }

        Ok(None)
    }

    // Returns the traces matching a given filter
    pub async fn trace_filter(
        &self,
        filter: TraceFilter,
    ) -> Result<Vec<LocalizedTransactionTrace>, BlockchainError> {
        let matcher = filter.matcher();
        let start = filter.from_block.unwrap_or(0);
        let end = filter.to_block.unwrap_or_else(|| self.best_number());

        if start > end {
            return Err(BlockchainError::RpcError(RpcError::invalid_params(
                "invalid block range, ensure that to block is greater than from block".to_string(),
            )));
        }

        let dist = end - start;
        if dist > 300 {
            return Err(BlockchainError::RpcError(RpcError::invalid_params(
                "block range too large, currently limited to 300".to_string(),
            )));
        }

        // Accumulate tasks for block range
        let mut trace_tasks = vec![];
        for num in start..=end {
            trace_tasks.push(self.trace_block(num.into()));
        }

        // Execute tasks and filter traces
        let traces = futures::future::try_join_all(trace_tasks).await?;
        let filtered_traces =
            traces.into_iter().flatten().filter(|trace| matcher.matches(&trace.trace));

        // Apply after and count
        let filtered_traces: Vec<_> = if let Some(after) = filter.after {
            filtered_traces.skip(after as usize).collect()
        } else {
            filtered_traces.collect()
        };

        let filtered_traces: Vec<_> = if let Some(count) = filter.count {
            filtered_traces.into_iter().take(count as usize).collect()
        } else {
            filtered_traces
        };

        Ok(filtered_traces)
    }

    /// Returns all receipts of the block
    pub fn mined_receipts(&self, hash: B256) -> Option<Vec<TypedReceipt>> {
        let block = self.mined_block_by_hash(hash)?;
        let mut receipts = Vec::new();
        let storage = self.blockchain.storage.read();
        for tx in block.transactions.hashes() {
            let receipt = storage.transactions.get(&tx)?.receipt.clone();
            receipts.push(receipt);
        }
        Some(receipts)
    }

    /// Returns all transaction receipts of the block
    pub fn mined_block_receipts(&self, id: impl Into<BlockId>) -> Option<Vec<ReceiptResponse>> {
        let mut receipts = Vec::new();
        let block = self.get_block(id)?;

        for transaction in block.body.transactions {
            let receipt = self.mined_transaction_receipt(transaction.hash())?;
            receipts.push(receipt.inner);
        }

        Some(receipts)
    }

    /// Returns the transaction receipt for the given hash
    pub(crate) fn mined_transaction_receipt(&self, hash: B256) -> Option<MinedTransactionReceipt> {
        let MinedTransaction { info, receipt: tx_receipt, block_hash, .. } =
            self.blockchain.get_transaction_by_hash(&hash)?;

        let index = info.transaction_index as usize;
        let block = self.blockchain.get_block_by_hash(&block_hash)?;
        let transaction = block.body.transactions[index].clone();

        // Cancun specific
        let excess_blob_gas = block.header.excess_blob_gas;
        let blob_gas_price =
            alloy_eips::eip4844::calc_blob_gasprice(excess_blob_gas.unwrap_or_default());
        let blob_gas_used = transaction.blob_gas_used();

        let effective_gas_price = transaction.effective_gas_price(block.header.base_fee_per_gas);

        let receipts = self.get_receipts(block.body.transactions.iter().map(|tx| tx.hash()));
        let next_log_index = receipts[..index].iter().map(|r| r.logs().len()).sum::<usize>();

        // Build a ReceiptWithBloom<rpc_types::Log> from the typed receipt, handling Deposit
        // specially
        let (status, cumulative_gas_used, logs_source, logs_bloom) = match &tx_receipt {
            TypedReceipt::Deposit(r) => (
                r.receipt.inner.status,
                r.receipt.inner.cumulative_gas_used,
                r.receipt.inner.logs.to_vec(),
                r.logs_bloom,
            ),
            _ => {
                let receipt_ref = tx_receipt.as_receipt_with_bloom();
                (
                    receipt_ref.receipt.status,
                    receipt_ref.receipt.cumulative_gas_used,
                    receipt_ref.receipt.logs.to_vec(),
                    receipt_ref.logs_bloom,
                )
            }
        };

        let receipt: alloy_consensus::Receipt<alloy_rpc_types::Log> = Receipt {
            status,
            cumulative_gas_used,
            logs: logs_source
                .into_iter()
                .enumerate()
                .map(|(index, log)| alloy_rpc_types::Log {
                    inner: log,
                    block_hash: Some(block_hash),
                    block_number: Some(block.header.number),
                    block_timestamp: Some(block.header.timestamp),
                    transaction_hash: Some(info.transaction_hash),
                    transaction_index: Some(info.transaction_index),
                    log_index: Some((next_log_index + index) as u64),
                    removed: false,
                })
                .collect(),
        };
        let receipt_with_bloom = ReceiptWithBloom { receipt, logs_bloom };

        let inner = match tx_receipt {
            TypedReceipt::EIP1559(_) => TypedReceiptRpc::EIP1559(receipt_with_bloom),
            TypedReceipt::Legacy(_) => TypedReceiptRpc::Legacy(receipt_with_bloom),
            TypedReceipt::EIP2930(_) => TypedReceiptRpc::EIP2930(receipt_with_bloom),
            TypedReceipt::EIP4844(_) => TypedReceiptRpc::EIP4844(receipt_with_bloom),
            TypedReceipt::EIP7702(_) => TypedReceiptRpc::EIP7702(receipt_with_bloom),
            TypedReceipt::Deposit(r) => {
                TypedReceiptRpc::Deposit(op_alloy_consensus::OpDepositReceiptWithBloom {
                    receipt: op_alloy_consensus::OpDepositReceipt {
                        inner: Receipt {
                            status: receipt_with_bloom.receipt.status,
                            cumulative_gas_used: receipt_with_bloom.receipt.cumulative_gas_used,
                            logs: receipt_with_bloom
                                .receipt
                                .logs
                                .into_iter()
                                .map(|l| l.inner)
                                .collect(),
                        },
                        deposit_nonce: r.receipt.deposit_nonce,
                        deposit_receipt_version: r.receipt.deposit_receipt_version,
                    },
                    logs_bloom: receipt_with_bloom.logs_bloom,
                })
            }
        };

        let inner = TransactionReceipt {
            inner,
            transaction_hash: info.transaction_hash,
            transaction_index: Some(info.transaction_index),
            block_number: Some(block.header.number),
            gas_used: info.gas_used,
            contract_address: info.contract_address,
            effective_gas_price,
            block_hash: Some(block_hash),
            from: info.from,
            to: info.to,
            blob_gas_price: Some(blob_gas_price),
            blob_gas_used,
        };

        let inner = WithOtherFields { inner, other: Default::default() };
        Some(MinedTransactionReceipt { inner, out: info.out })
    }

    /// Returns the blocks receipts for the given number
    pub async fn block_receipts(
        &self,
        number: BlockId,
    ) -> Result<Option<Vec<ReceiptResponse>>, BlockchainError> {
        if let Some(receipts) = self.mined_block_receipts(number) {
            return Ok(Some(receipts));
        }

        if let Some(fork) = self.get_fork() {
            let number = match self.ensure_block_number(Some(number)).await {
                Err(_) => return Ok(None),
                Ok(n) => n,
            };

            if fork.predates_fork_inclusive(number) {
                let receipts = fork.block_receipts(number).await?;

                return Ok(receipts);
            }
        }

        Ok(None)
    }

    pub async fn transaction_by_block_number_and_index(
        &self,
        number: BlockNumber,
        index: Index,
    ) -> Result<Option<AnyRpcTransaction>, BlockchainError> {
        if let Some(block) = self.mined_block_by_number(number) {
            return Ok(self.mined_transaction_by_block_hash_and_index(block.header.hash, index));
        }

        if let Some(fork) = self.get_fork() {
            let number = self.convert_block_number(Some(number));
            if fork.predates_fork(number) {
                return Ok(fork
                    .transaction_by_block_number_and_index(number, index.into())
                    .await?);
            }
        }

        Ok(None)
    }

    pub async fn transaction_by_block_hash_and_index(
        &self,
        hash: B256,
        index: Index,
    ) -> Result<Option<AnyRpcTransaction>, BlockchainError> {
        if let tx @ Some(_) = self.mined_transaction_by_block_hash_and_index(hash, index) {
            return Ok(tx);
        }

        if let Some(fork) = self.get_fork() {
            return Ok(fork.transaction_by_block_hash_and_index(hash, index.into()).await?);
        }

        Ok(None)
    }

    pub fn mined_transaction_by_block_hash_and_index(
        &self,
        block_hash: B256,
        index: Index,
    ) -> Option<AnyRpcTransaction> {
        let (info, block, tx) = {
            let storage = self.blockchain.storage.read();
            let block = storage.blocks.get(&block_hash).cloned()?;
            let index: usize = index.into();
            let tx = block.body.transactions.get(index)?.clone();
            let info = storage.transactions.get(&tx.hash())?.info.clone();
            (info, block, tx)
        };

        Some(transaction_build(
            Some(info.transaction_hash),
            tx,
            Some(&block),
            Some(info),
            block.header.base_fee_per_gas,
        ))
    }

    pub async fn transaction_by_hash(
        &self,
        hash: B256,
    ) -> Result<Option<AnyRpcTransaction>, BlockchainError> {
        trace!(target: "backend", "transaction_by_hash={:?}", hash);
        if let tx @ Some(_) = self.mined_transaction_by_hash(hash) {
            return Ok(tx);
        }

        if let Some(fork) = self.get_fork() {
            return fork
                .transaction_by_hash(hash)
                .await
                .map_err(BlockchainError::AlloyForkProvider);
        }

        Ok(None)
    }

    pub fn mined_transaction_by_hash(&self, hash: B256) -> Option<AnyRpcTransaction> {
        let (info, block) = {
            let storage = self.blockchain.storage.read();
            let MinedTransaction { info, block_hash, .. } =
                storage.transactions.get(&hash)?.clone();
            let block = storage.blocks.get(&block_hash).cloned()?;
            (info, block)
        };
        let tx = block.body.transactions.get(info.transaction_index as usize)?.clone();

        Some(transaction_build(
            Some(info.transaction_hash),
            tx,
            Some(&block),
            Some(info),
            block.header.base_fee_per_gas,
        ))
    }

    pub fn get_blob_by_tx_hash(&self, hash: B256) -> Result<Option<Vec<alloy_consensus::Blob>>> {
        // Try to get the mined transaction by hash
        if let Some(tx) = self.mined_transaction_by_hash(hash)
            && let Ok(typed_tx) = FoundryTxEnvelope::try_from(tx)
            && let Some(sidecar) = typed_tx.sidecar()
        {
            return Ok(Some(sidecar.sidecar.blobs.clone()));
        }

        Ok(None)
    }

    pub fn get_blobs_by_block_id(
        &self,
        id: impl Into<BlockId>,
        versioned_hashes: Vec<B256>,
    ) -> Result<Option<Vec<alloy_consensus::Blob>>> {
        Ok(self.get_block(id).map(|block| {
            block
                .body
                .transactions
                .iter()
                .filter_map(|tx| tx.as_ref().sidecar())
                .flat_map(|sidecar| {
                    sidecar.sidecar.blobs.iter().zip(sidecar.sidecar.commitments.iter())
                })
                .filter(|(_, commitment)| {
                    // Filter blobs by versioned_hashes if provided
                    versioned_hashes.is_empty()
                        || versioned_hashes.contains(&kzg_to_versioned_hash(commitment.as_slice()))
                })
                .map(|(blob, _)| *blob)
                .collect()
        }))
    }

    pub fn get_blob_sidecars_by_block_id(
        &self,
        block_id: BlockId,
    ) -> Result<Option<BlobTransactionSidecar>> {
        if let Some(full_block) = self.get_full_block(block_id) {
            let sidecar = full_block
                .into_transactions_iter()
                .map(FoundryTxEnvelope::try_from)
                .filter_map(|typed_tx_result| {
                    typed_tx_result.ok()?.sidecar().map(|sidecar| sidecar.sidecar().clone())
                })
                .fold(BlobTransactionSidecar::default(), |mut acc, sidecar| {
                    acc.blobs.extend(sidecar.blobs);
                    acc.commitments.extend(sidecar.commitments);
                    acc.proofs.extend(sidecar.proofs);
                    acc
                });
            Ok(Some(sidecar))
        } else {
            Ok(None)
        }
    }

    pub fn get_blob_by_versioned_hash(&self, hash: B256) -> Result<Option<Blob>> {
        let storage = self.blockchain.storage.read();
        for block in storage.blocks.values() {
            for tx in &block.body.transactions {
                let typed_tx = tx.as_ref();
                if let Some(sidecar) = typed_tx.sidecar() {
                    for versioned_hash in sidecar.sidecar.versioned_hashes() {
                        if versioned_hash == hash
                            && let Some(index) =
                                sidecar.sidecar.commitments.iter().position(|commitment| {
                                    kzg_to_versioned_hash(commitment.as_slice()) == *hash
                                })
                            && let Some(blob) = sidecar.sidecar.blobs.get(index)
                        {
                            return Ok(Some(*blob));
                        }
                    }
                }
            }
        }
        Ok(None)
    }

    /// Overrides the given signature to impersonate the specified address during ecrecover.
    pub async fn impersonate_signature(
        &self,
        signature: Bytes,
        address: Address,
    ) -> Result<(), BlockchainError> {
        self.cheats.add_recover_override(signature, address);
        Ok(())
    }

    /// Prove an account's existence or nonexistence in the state trie.
    ///
    /// Returns a merkle proof of the account's trie node, `account_key` == keccak(address)
    pub async fn prove_account_at(
        &self,
        address: Address,
        keys: Vec<B256>,
        block_request: Option<BlockRequest>,
    ) -> Result<AccountProof, BlockchainError> {
        let block_number = block_request.as_ref().map(|r| r.block_number());

        self.with_database_at(block_request, |block_db, _| {
            trace!(target: "backend", "get proof for {:?} at {:?}", address, block_number);
            let db = block_db.maybe_as_full_db().ok_or(BlockchainError::DataUnavailable)?;
            let account = db.get(&address).cloned().unwrap_or_default();

            let mut builder = HashBuilder::default()
                .with_proof_retainer(ProofRetainer::new(vec![Nibbles::unpack(keccak256(address))]));

            for (key, account) in trie_accounts(db) {
                builder.add_leaf(key, &account);
            }

            let _ = builder.root();

            let proof = builder
                .take_proof_nodes()
                .into_nodes_sorted()
                .into_iter()
                .map(|(_, v)| v)
                .collect();
            let storage_proofs = prove_storage(&account.storage, &keys);

            let account_proof = AccountProof {
                address,
                balance: account.info.balance,
                nonce: account.info.nonce,
                code_hash: account.info.code_hash,
                storage_hash: storage_root(&account.storage),
                account_proof: proof,
                storage_proof: keys
                    .into_iter()
                    .zip(storage_proofs)
                    .map(|(key, proof)| {
                        let storage_key: U256 = key.into();
                        let value = account.storage.get(&storage_key).copied().unwrap_or_default();
                        StorageProof { key: JsonStorageKey::Hash(key), value, proof }
                    })
                    .collect(),
            };

            Ok(account_proof)
        })
        .await?
    }

    /// Returns a new block event stream
    pub fn new_block_notifications(&self) -> NewBlockNotifications {
        let (tx, rx) = unbounded();
        self.new_block_listeners.lock().push(tx);
        trace!(target: "backed", "added new block listener");
        rx
    }

    /// Notifies all `new_block_listeners` about the new block
    fn notify_on_new_block(&self, header: Header, hash: B256) {
        // cleanup closed notification streams first, if the channel is closed we can remove the
        // sender half for the set
        self.new_block_listeners.lock().retain(|tx| !tx.is_closed());

        let notification = NewBlockNotification { hash, header: Arc::new(header) };

        self.new_block_listeners
            .lock()
            .retain(|tx| tx.unbounded_send(notification.clone()).is_ok());
    }

    /// Reorg the chain to a common height and execute blocks to build new chain.
    ///
    /// The state of the chain is rewound using `rewind` to the common block, including the db,
    /// storage, and env.
    ///
    /// Finally, `do_mine_block` is called to create the new chain.
    pub async fn reorg(
        &self,
        depth: u64,
        tx_pairs: HashMap<u64, Vec<Arc<PoolTransaction>>>,
        common_block: Block,
    ) -> Result<(), BlockchainError> {
        self.rollback(common_block).await?;
        // Create the new reorged chain, filling the blocks with transactions if supplied
        for i in 0..depth {
            let to_be_mined = tx_pairs.get(&i).cloned().unwrap_or_else(Vec::new);
            let outcome = self.do_mine_block(to_be_mined).await;
            node_info!(
                "    Mined reorg block number {}. With {} valid txs and with invalid {} txs",
                outcome.block_number,
                outcome.included.len(),
                outcome.invalid.len()
            );
        }

        Ok(())
    }

    /// Rollback the chain to a common height.
    ///
    /// The state of the chain is rewound using `rewind` to the common block, including the db,
    /// storage, and env.
    pub async fn rollback(&self, common_block: Block) -> Result<(), BlockchainError> {
        // Get the database at the common block
        let common_state = {
            let return_state_or_throw_err =
                |db: Option<&StateDb>| -> Result<HashMap<Address, DbAccount>, BlockchainError> {
                    let state_db = db.ok_or(BlockchainError::DataUnavailable)?;
                    let db_full =
                        state_db.maybe_as_full_db().ok_or(BlockchainError::DataUnavailable)?;
                    Ok(db_full.clone())
                };

            let hash = &common_block.header.hash_slow();
            let read_guard = self.states.upgradable_read();
            if let Some(db) = read_guard.get_state(hash) {
                return_state_or_throw_err(Some(db))?
            } else {
                let mut write_guard = RwLockUpgradableReadGuard::upgrade(read_guard);
                return_state_or_throw_err(write_guard.get_on_disk_state(hash))?
            }
        };

        {
            // Set state to common state
            self.db.write().await.clear();
            for (address, acc) in common_state {
                for (key, value) in acc.storage {
                    self.db.write().await.set_storage_at(address, key.into(), value.into())?;
                }
                self.db.write().await.insert_account(address, acc.info);
            }
        }

        {
            // Unwind the storage back to the common ancestor
            self.blockchain
                .storage
                .write()
                .unwind_to(common_block.header.number, common_block.header.hash_slow());

            // Set environment back to common block
            let mut env = self.env.write();
            env.evm_env.block_env.number = U256::from(common_block.header.number);
            env.evm_env.block_env.timestamp = U256::from(common_block.header.timestamp);
            env.evm_env.block_env.gas_limit = common_block.header.gas_limit;
            env.evm_env.block_env.difficulty = common_block.header.difficulty;
            env.evm_env.block_env.prevrandao = Some(common_block.header.mix_hash);

            self.time.reset(env.evm_env.block_env.timestamp.saturating_to());
        }
        Ok(())
    }
}

fn get_block_env<F, T>(
    state: &StateDb,
    block_number: u64,
    block: AnyRpcBlock,
    is_offline: bool,
    f: F,
) -> T
where
    F: FnOnce(Box<dyn MaybeFullDatabase + '_>, BlockEnv) -> T,
{
    let block = BlockEnv {
        number: U256::from(block_number),
        beneficiary: block.header.beneficiary,
        timestamp: U256::from(block.header.timestamp),
        difficulty: block.header.difficulty,
        prevrandao: block.header.mix_hash,
        basefee: block.header.base_fee_per_gas.unwrap_or_default(),
        gas_limit: block.header.gas_limit,
        ..Default::default()
    };

    // In offline mode, wrap state to prevent RPC calls for missing data
    if is_offline {
        use crate::eth::backend::db::OfflineStateDb;
        let offline_wrapper = OfflineStateDb::new_ref(state);
        f(Box::new(offline_wrapper), block)
    } else {
        f(Box::new(state), block)
    }
}

/// Get max nonce from transaction pool by address.
fn get_pool_transactions_nonce(
    pool_transactions: &[Arc<PoolTransaction>],
    address: Address,
) -> Option<u64> {
    if let Some(highest_nonce) = pool_transactions
        .iter()
        .filter(|tx| *tx.pending_transaction.sender() == address)
        .map(|tx| tx.pending_transaction.nonce())
        .max()
    {
        let tx_count = highest_nonce.saturating_add(1);
        return Some(tx_count);
    }
    None
}

#[async_trait::async_trait]
impl TransactionValidator for Backend {
    async fn validate_pool_transaction(
        &self,
        tx: &PendingTransaction,
    ) -> Result<(), BlockchainError> {
        let address = *tx.sender();
        let account = self.get_account(address).await?;
        let env = self.next_env();
        Ok(self.validate_pool_transaction_for(tx, &account, &env)?)
    }

    fn validate_pool_transaction_for(
        &self,
        pending: &PendingTransaction,
        account: &AccountInfo,
        env: &Env,
    ) -> Result<(), InvalidTransactionError> {
        let tx = &pending.transaction;

        if let Some(tx_chain_id) = tx.chain_id() {
            let chain_id = self.chain_id();
            if chain_id.to::<u64>() != tx_chain_id {
                if let FoundryTxEnvelope::Legacy(tx) = tx.as_ref() {
                    // <https://github.com/ethereum/EIPs/blob/master/EIPS/eip-155.md>
                    if env.evm_env.cfg_env.spec >= SpecId::SPURIOUS_DRAGON
                        && tx.chain_id().is_none()
                    {
                        warn!(target: "backend", ?chain_id, ?tx_chain_id, "incompatible EIP155-based V");
                        return Err(InvalidTransactionError::IncompatibleEIP155);
                    }
                } else {
                    warn!(target: "backend", ?chain_id, ?tx_chain_id, "invalid chain id");
                    return Err(InvalidTransactionError::InvalidChainId);
                }
            }
        }

        // Nonce validation
        let is_deposit_tx = matches!(pending.transaction.as_ref(), FoundryTxEnvelope::Deposit(_));
        let nonce = tx.nonce();
        if nonce < account.nonce && !is_deposit_tx {
            warn!(target: "backend", "[{:?}] nonce too low", tx.hash());
            return Err(InvalidTransactionError::NonceTooLow);
        }

        // EIP-4844 structural validation
        if env.evm_env.cfg_env.spec >= SpecId::CANCUN && tx.is_eip4844() {
            // Heavy (blob validation) checks
            let blob_tx = match tx.as_ref() {
                FoundryTxEnvelope::EIP4844(tx) => tx.tx(),
                _ => unreachable!(),
            };

            let blob_count = blob_tx.tx().blob_versioned_hashes.len();

            // Ensure there are blob hashes.
            if blob_count == 0 {
                return Err(InvalidTransactionError::NoBlobHashes);
            }

            // Ensure the tx does not exceed the max blobs per block.
            let max_blob_count = self.blob_params().max_blob_count as usize;
            if blob_count > max_blob_count {
                return Err(InvalidTransactionError::TooManyBlobs(blob_count, max_blob_count));
            }

            // Check for any blob validation errors if not impersonating.
            if !self.skip_blob_validation(Some(*pending.sender()))
                && let Err(err) = blob_tx.validate(EnvKzgSettings::default().get())
            {
                return Err(InvalidTransactionError::BlobTransactionValidationError(err));
            }
        }

        // Balance and fee related checks
        if !self.disable_pool_balance_checks {
            // Gas limit validation
            if tx.gas_limit() < MIN_TRANSACTION_GAS as u64 {
                warn!(target: "backend", "[{:?}] gas too low", tx.hash());
                return Err(InvalidTransactionError::GasTooLow);
            }

            // Check tx gas limit against block gas limit, if block gas limit is set.
            if !env.evm_env.cfg_env.disable_block_gas_limit
                && tx.gas_limit() > env.evm_env.block_env.gas_limit
            {
                warn!(target: "backend", "[{:?}] gas too high", tx.hash());
                return Err(InvalidTransactionError::GasTooHigh(ErrDetail {
                    detail: String::from("tx.gas_limit > env.block.gas_limit"),
                }));
            }

            // Check tx gas limit against tx gas limit cap (Osaka hard fork and later).
            if env.evm_env.cfg_env.tx_gas_limit_cap.is_none()
                && tx.gas_limit() > env.evm_env.cfg_env().tx_gas_limit_cap()
            {
                warn!(target: "backend", "[{:?}] gas too high", tx.hash());
                return Err(InvalidTransactionError::GasTooHigh(ErrDetail {
                    detail: String::from("tx.gas_limit > env.cfg.tx_gas_limit_cap"),
                }));
            }

            // EIP-1559 fee validation (London hard fork and later).
            if env.evm_env.cfg_env.spec >= SpecId::LONDON {
                if tx.max_fee_per_gas() < env.evm_env.block_env.basefee.into() && !is_deposit_tx {
                    warn!(target: "backend", "max fee per gas={}, too low, block basefee={}", tx.max_fee_per_gas(), env.evm_env.block_env.basefee);
                    return Err(InvalidTransactionError::FeeCapTooLow);
                }

                if let (Some(max_priority_fee_per_gas), max_fee_per_gas) =
                    (tx.as_ref().max_priority_fee_per_gas(), tx.as_ref().max_fee_per_gas())
                    && max_priority_fee_per_gas > max_fee_per_gas
                {
                    warn!(target: "backend", "max priority fee per gas={}, too high, max fee per gas={}", max_priority_fee_per_gas, max_fee_per_gas);
                    return Err(InvalidTransactionError::TipAboveFeeCap);
                }
            }

            // EIP-4844 blob fee validation
            if env.evm_env.cfg_env.spec >= SpecId::CANCUN
                && tx.is_eip4844()
                && let Some(max_fee_per_blob_gas) = tx.max_fee_per_blob_gas()
                && let Some(blob_gas_and_price) = &env.evm_env.block_env.blob_excess_gas_and_price
                && max_fee_per_blob_gas < blob_gas_and_price.blob_gasprice
            {
                warn!(target: "backend", "max fee per blob gas={}, too low, block blob gas price={}", max_fee_per_blob_gas, blob_gas_and_price.blob_gasprice);
                return Err(InvalidTransactionError::BlobFeeCapTooLow(
                    max_fee_per_blob_gas,
                    blob_gas_and_price.blob_gasprice,
                ));
            }

            let max_cost =
                (tx.gas_limit() as u128).saturating_mul(tx.max_fee_per_gas()).saturating_add(
                    tx.blob_gas_used()
                        .map(|g| g as u128)
                        .unwrap_or(0)
                        .mul(tx.max_fee_per_blob_gas().unwrap_or(0)),
                );
            let value = tx.value();
            match tx.as_ref() {
                FoundryTxEnvelope::Deposit(deposit_tx) => {
                    // Deposit transactions
                    // https://specs.optimism.io/protocol/deposits.html#execution
                    // 1. no gas cost check required since already have prepaid gas from L1
                    // 2. increment account balance by deposited amount before checking for
                    //    sufficient funds `tx.value <= existing account value + deposited value`
                    if value > account.balance + U256::from(deposit_tx.mint) {
                        warn!(target: "backend", "[{:?}] insufficient balance={}, required={} account={:?}", tx.hash(), account.balance + U256::from(deposit_tx.mint), value, *pending.sender());
                        return Err(InvalidTransactionError::InsufficientFunds);
                    }
                }
                _ => {
                    // check sufficient funds: `gas * price + value`
                    let req_funds =
                        max_cost.checked_add(value.saturating_to()).ok_or_else(|| {
                            warn!(target: "backend", "[{:?}] cost too high", tx.hash());
                            InvalidTransactionError::InsufficientFunds
                        })?;
                    if account.balance < U256::from(req_funds) {
                        warn!(target: "backend", "[{:?}] insufficient allowance={}, required={} account={:?}", tx.hash(), account.balance, req_funds, *pending.sender());
                        return Err(InvalidTransactionError::InsufficientFunds);
                    }
                }
            }
        }
        Ok(())
    }

    fn validate_for(
        &self,
        tx: &PendingTransaction,
        account: &AccountInfo,
        env: &Env,
    ) -> Result<(), InvalidTransactionError> {
        self.validate_pool_transaction_for(tx, account, env)?;
        if tx.nonce() > account.nonce {
            return Err(InvalidTransactionError::NonceTooHigh);
        }
        Ok(())
    }
}

/// Creates a `AnyRpcTransaction` as it's expected for the `eth` RPC api from storage data
pub fn transaction_build(
    tx_hash: Option<B256>,
    eth_transaction: MaybeImpersonatedTransaction,
    block: Option<&Block>,
    info: Option<TransactionInfo>,
    base_fee: Option<u64>,
) -> AnyRpcTransaction {
    if let FoundryTxEnvelope::Deposit(deposit_tx) = eth_transaction.as_ref() {
        let dep_tx = deposit_tx;

        let ser = serde_json::to_value(dep_tx).expect("could not serialize TxDeposit");
        let maybe_deposit_fields = OtherFields::try_from(ser);

        match maybe_deposit_fields {
            Ok(mut fields) => {
                // Add zeroed signature fields for backwards compatibility
                // https://specs.optimism.io/protocol/deposits.html#the-deposited-transaction-type
                fields.insert("v".to_string(), serde_json::to_value("0x0").unwrap());
                fields.insert("r".to_string(), serde_json::to_value(B256::ZERO).unwrap());
                fields.insert(String::from("s"), serde_json::to_value(B256::ZERO).unwrap());
                fields.insert(String::from("nonce"), serde_json::to_value("0x0").unwrap());

                let inner = UnknownTypedTransaction {
                    ty: AnyTxType(DEPOSIT_TX_TYPE_ID),
                    fields,
                    memo: Default::default(),
                };

                let envelope = AnyTxEnvelope::Unknown(UnknownTxEnvelope {
                    hash: eth_transaction.hash(),
                    inner,
                });

                let tx = Transaction {
                    inner: Recovered::new_unchecked(envelope, deposit_tx.from),
                    block_hash: block
                        .as_ref()
                        .map(|block| B256::from(keccak256(alloy_rlp::encode(&block.header)))),
                    block_number: block.as_ref().map(|block| block.header.number),
                    transaction_index: info.as_ref().map(|info| info.transaction_index),
                    effective_gas_price: None,
                };

                return AnyRpcTransaction::from(WithOtherFields::new(tx));
            }
            Err(_) => {
                error!(target: "backend", "failed to serialize deposit transaction");
            }
        }
    }

    let transaction = eth_transaction.into_rpc_transaction();
    let effective_gas_price = transaction.effective_gas_price(base_fee);

    let envelope = transaction.inner;
    let from = envelope.signer();

    // if a specific hash was provided we update the transaction's hash
    // This is important for impersonated transactions since they all use the
    // `BYPASS_SIGNATURE` which would result in different hashes
    // Note: for impersonated transactions this only concerns pending transactions because
    // there's // no `info` yet.
    let hash = tx_hash.unwrap_or(*envelope.tx_hash());

    let envelope = match envelope.into_inner() {
        TxEnvelope::Legacy(signed_tx) => {
            let (t, sig, _) = signed_tx.into_parts();
            let new_signed = Signed::new_unchecked(t, sig, hash);
            AnyTxEnvelope::Ethereum(TxEnvelope::Legacy(new_signed))
        }
        TxEnvelope::Eip1559(signed_tx) => {
            let (t, sig, _) = signed_tx.into_parts();
            let new_signed = Signed::new_unchecked(t, sig, hash);
            AnyTxEnvelope::Ethereum(TxEnvelope::Eip1559(new_signed))
        }
        TxEnvelope::Eip2930(signed_tx) => {
            let (t, sig, _) = signed_tx.into_parts();
            let new_signed = Signed::new_unchecked(t, sig, hash);
            AnyTxEnvelope::Ethereum(TxEnvelope::Eip2930(new_signed))
        }
        TxEnvelope::Eip4844(signed_tx) => {
            let (t, sig, _) = signed_tx.into_parts();
            let new_signed = Signed::new_unchecked(t, sig, hash);
            AnyTxEnvelope::Ethereum(TxEnvelope::Eip4844(new_signed))
        }
        TxEnvelope::Eip7702(signed_tx) => {
            let (t, sig, _) = signed_tx.into_parts();
            let new_signed = Signed::new_unchecked(t, sig, hash);
            AnyTxEnvelope::Ethereum(TxEnvelope::Eip7702(new_signed))
        }
    };

    let tx = Transaction {
        inner: Recovered::new_unchecked(envelope, from),
        block_hash: block.as_ref().map(|block| block.header.hash_slow()),
        block_number: block.as_ref().map(|block| block.header.number),
        transaction_index: info.as_ref().map(|info| info.transaction_index),
        // deprecated
        effective_gas_price: Some(effective_gas_price),
    };
    AnyRpcTransaction::from(WithOtherFields::new(tx))
}

/// Prove a storage key's existence or nonexistence in the account's storage trie.
///
/// `storage_key` is the hash of the desired storage key, meaning
/// this will only work correctly under a secure trie.
/// `storage_key` == keccak(key)
pub fn prove_storage(storage: &HashMap<U256, U256>, keys: &[B256]) -> Vec<Vec<Bytes>> {
    let keys: Vec<_> = keys.iter().map(|key| Nibbles::unpack(keccak256(key))).collect();

    let mut builder = HashBuilder::default().with_proof_retainer(ProofRetainer::new(keys.clone()));

    for (key, value) in trie_storage(storage) {
        builder.add_leaf(key, &value);
    }

    let _ = builder.root();

    let mut proofs = Vec::new();
    let all_proof_nodes = builder.take_proof_nodes();

    for proof_key in keys {
        // Iterate over all proof nodes and find the matching ones.
        // The filtered results are guaranteed to be in order.
        let matching_proof_nodes =
            all_proof_nodes.matching_nodes_sorted(&proof_key).into_iter().map(|(_, node)| node);
        proofs.push(matching_proof_nodes.collect());
    }

    proofs
}

pub fn is_arbitrum(chain_id: u64) -> bool {
    if let Ok(chain) = NamedChain::try_from(chain_id) {
        return chain.is_arbitrum();
    }
    false
}

pub fn op_haltreason_to_instruction_result(op_reason: OpHaltReason) -> InstructionResult {
    match op_reason {
        OpHaltReason::Base(eth_h) => eth_h.into(),
        OpHaltReason::FailedDeposit => InstructionResult::Stop,
    }
}

#[cfg(test)]
mod tests {
    use crate::{NodeConfig, spawn};

    #[tokio::test]
    async fn test_deterministic_block_mining() {
        // Test that mine_block produces deterministic block hashes with same initial conditions
        let genesis_timestamp = 1743944919u64;

        // Create two identical backends
        let config_a = NodeConfig::test().with_genesis_timestamp(genesis_timestamp.into());
        let config_b = NodeConfig::test().with_genesis_timestamp(genesis_timestamp.into());

        let (api_a, _handle_a) = spawn(config_a).await;
        let (api_b, _handle_b) = spawn(config_b).await;

        // Mine empty blocks (no transactions) on both backends
        let outcome_a_1 = api_a.backend.mine_block(vec![]).await;
        let outcome_b_1 = api_b.backend.mine_block(vec![]).await;

        // Both should mine the same block number
        assert_eq!(outcome_a_1.block_number, outcome_b_1.block_number);

        // Get the actual blocks to compare hashes
        let block_a_1 =
            api_a.block_by_number(outcome_a_1.block_number.into()).await.unwrap().unwrap();
        let block_b_1 =
            api_b.block_by_number(outcome_b_1.block_number.into()).await.unwrap().unwrap();

        // The block hashes should be identical
        assert_eq!(
            block_a_1.header.hash, block_b_1.header.hash,
            "Block hashes should be deterministic. Got {} vs {}",
            block_a_1.header.hash, block_b_1.header.hash
        );

        // Mine another block to ensure it remains deterministic
        let outcome_a_2 = api_a.backend.mine_block(vec![]).await;
        let outcome_b_2 = api_b.backend.mine_block(vec![]).await;

        let block_a_2 =
            api_a.block_by_number(outcome_a_2.block_number.into()).await.unwrap().unwrap();
        let block_b_2 =
            api_b.block_by_number(outcome_b_2.block_number.into()).await.unwrap().unwrap();

        assert_eq!(
            block_a_2.header.hash, block_b_2.header.hash,
            "Second block hashes should also be deterministic. Got {} vs {}",
            block_a_2.header.hash, block_b_2.header.hash
        );

        // Ensure the blocks are different (sanity check)
        assert_ne!(
            block_a_1.header.hash, block_a_2.header.hash,
            "Different blocks should have different hashes"
        );
    }
}
