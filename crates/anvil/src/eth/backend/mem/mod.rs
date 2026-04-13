//! In-memory blockchain backend.
use self::state::trie_storage;

use crate::{
    ForkChoice, NodeConfig, PrecompileFactory,
    config::PruneStateHistoryConfig,
    eth::{
        backend::{
            cheats::{CheatEcrecover, CheatsManager},
            db::{AnvilCacheDB, Db, MaybeFullDatabase, SerializableState, StateDb},
            executor::{
                AnvilBlockExecutor, ExecutedPoolTransactions, PoolTxGasConfig,
                execute_pool_transactions,
            },
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
        sign::build_impersonated,
    },
    mem::{
        inspector::{AnvilInspector, InspectorTxConfig},
        storage::{BlockchainStorage, InMemoryBlockStates, MinedBlockOutcome},
    },
};
use alloy_chains::NamedChain;
use alloy_consensus::{
    Blob, BlockHeader, EnvKzgSettings, Header, Signed, Transaction as TransactionTrait,
    TrieAccount, TxEnvelope, TxReceipt, Typed2718,
    constants::EMPTY_WITHDRAWALS,
    proofs::{calculate_receipt_root, calculate_transaction_root},
    transaction::Recovered,
};
use alloy_eips::{
    BlockNumHash, Encodable2718, eip2935, eip4844::kzg_to_versioned_hash,
    eip7685::EMPTY_REQUESTS_HASH, eip7840::BlobParams, eip7910::SystemContract,
};
use alloy_evm::{
    Database, EthEvmFactory, Evm, EvmEnv, EvmFactory, FromTxWithEncoded,
    block::{BlockExecutionResult, BlockExecutor, StateDB},
    eth::EthEvmContext,
    overrides::{OverrideBlockHashes, apply_state_overrides},
    precompiles::{DynPrecompile, Precompile, PrecompilesMap},
};
use alloy_network::{
    AnyHeader, AnyRpcBlock, AnyRpcHeader, AnyRpcTransaction, AnyTxEnvelope, AnyTxType, Network,
    ReceiptResponse, TransactionBuilder, UnknownTxEnvelope, UnknownTypedTransaction,
};
use alloy_op_evm::OpEvmFactory;
use alloy_primitives::{
    Address, B256, Bloom, Bytes, TxHash, TxKind, U64, U256, hex, keccak256, logs_bloom,
    map::{AddressMap, HashMap, HashSet},
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
        },
        parity::{LocalizedTransactionTrace, TraceResultsWithTransactionHash, TraceType},
    },
};
use alloy_serde::{OtherFields, WithOtherFields};
use alloy_trie::{HashBuilder, Nibbles, proof::ProofRetainer};
use anvil_core::eth::{
    block::{Block, BlockInfo, create_block},
    transaction::{MaybeImpersonatedTransaction, PendingTransaction, TransactionInfo},
};
use anvil_rpc::error::RpcError;
use chrono::Datelike;
use eyre::{Context, Result};
use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use foundry_evm::{
    backend::{DatabaseError, DatabaseResult, RevertStateSnapshotAction},
    constants::DEFAULT_CREATE2_DEPLOYER_RUNTIME_CODE,
    core::precompiles::EC_RECOVER,
    decode::RevertDecoder,
    hardfork::FoundryHardfork,
    inspectors::AccessListInspector,
    traces::{
        CallTraceDecoder, FourByteInspector, GethTraceBuilder, TracingInspector,
        TracingInspectorConfig,
    },
    utils::{
        block_env_from_header, get_blob_base_fee_update_fraction,
        get_blob_base_fee_update_fraction_by_spec_id,
    },
};
use foundry_evm_networks::NetworkConfigs;
use foundry_primitives::{
    FoundryNetwork, FoundryReceiptEnvelope, FoundryTransactionRequest, FoundryTxEnvelope,
    FoundryTxReceipt, get_deposit_tx_parts,
};
use futures::channel::mpsc::{UnboundedSender, unbounded};
use op_alloy_consensus::{DEPOSIT_TX_TYPE_ID, OpTransaction as OpTransactionTrait};
use op_revm::{OpContext, OpHaltReason, OpSpecId, OpTransaction};
use parking_lot::{Mutex, RwLock, RwLockUpgradableReadGuard};
use revm::{
    DatabaseCommit, Inspector,
    context::{Block as RevmBlock, BlockEnv, Cfg, TxEnv},
    context_interface::{
        block::BlobExcessGasAndPrice,
        result::{ExecutionResult, HaltReason, Output, ResultAndState},
    },
    database::{CacheDB, DbAccount, WrapDatabaseRef},
    interpreter::InstructionResult,
    precompile::{PrecompileSpecId, Precompiles},
    primitives::{KECCAK_EMPTY, hardfork::SpecId},
    state::AccountInfo,
};
use std::{
    collections::BTreeMap,
    fmt::{self, Debug},
    io::{Read, Write},
    ops::{Mul, Not},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use storage::{Blockchain, DEFAULT_HISTORY_LIMIT, MinedTransaction};
use tempo_chainspec::hardfork::TempoHardfork;
use tempo_evm::evm::TempoEvmFactory;
use tempo_primitives::TEMPO_TX_TYPE_ID;
use tempo_revm::{
    TempoBatchCallEnv, TempoBlockEnv, TempoHaltReason, TempoTxEnv, evm::TempoContext,
    gas_params::tempo_gas_params,
};
use tokio::sync::RwLock as AsyncRwLock;

pub mod cache;
pub mod fork_db;
pub mod in_memory_db;
pub mod inspector;
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

pub type State = foundry_evm::utils::StateChangeset;

/// A block request, which includes the Pool Transactions if it's Pending
pub enum BlockRequest<T> {
    Pending(Vec<Arc<PoolTransaction<T>>>),
    Number(u64),
}

impl<T> fmt::Debug for BlockRequest<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pending(txs) => f.debug_tuple("Pending").field(&txs.len()).finish(),
            Self::Number(n) => f.debug_tuple("Number").field(n).finish(),
        }
    }
}

impl<T> BlockRequest<T> {
    pub fn block_number(&self) -> BlockNumber {
        match *self {
            Self::Pending(_) => BlockNumber::Pending,
            Self::Number(n) => BlockNumber::Number(n),
        }
    }
}

/// Gives access to the [revm::Database]
pub struct Backend<N: Network> {
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
    blockchain: Blockchain<N>,
    /// Historic states of previous blocks.
    states: Arc<RwLock<InMemoryBlockStates>>,
    /// EVM environment data of the chain (block env, cfg env).
    evm_env: Arc<RwLock<EvmEnv>>,
    /// Network configuration (optimism, custom precompiles, etc.)
    networks: NetworkConfigs,
    /// The active hardfork.
    hardfork: FoundryHardfork,
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
    /// Disable pool balance checks
    disable_pool_balance_checks: bool,
}

impl<N: Network> Clone for Backend<N> {
    fn clone(&self) -> Self {
        Self {
            db: self.db.clone(),
            blockchain: self.blockchain.clone(),
            states: self.states.clone(),
            evm_env: self.evm_env.clone(),
            networks: self.networks,
            hardfork: self.hardfork,
            fork: self.fork.clone(),
            time: self.time.clone(),
            cheats: self.cheats.clone(),
            fees: self.fees.clone(),
            genesis: self.genesis.clone(),
            new_block_listeners: self.new_block_listeners.clone(),
            active_state_snapshots: self.active_state_snapshots.clone(),
            enable_steps_tracing: self.enable_steps_tracing,
            print_logs: self.print_logs,
            print_traces: self.print_traces,
            call_trace_decoder: self.call_trace_decoder.clone(),
            prune_state_history_config: self.prune_state_history_config,
            transaction_block_keeper: self.transaction_block_keeper,
            node_config: self.node_config.clone(),
            slots_in_an_epoch: self.slots_in_an_epoch,
            precompile_factory: self.precompile_factory.clone(),
            mining: self.mining.clone(),
            disable_pool_balance_checks: self.disable_pool_balance_checks,
        }
    }
}

impl<N: Network> fmt::Debug for Backend<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Backend").finish_non_exhaustive()
    }
}

// Methods that are generic over any Network.
impl<N: Network> Backend<N> {
    /// Sets the account to impersonate
    ///
    /// Returns `true` if the account is already impersonated
    pub fn impersonate(&self, addr: Address) -> bool {
        if self.cheats.impersonated_accounts().contains(&addr) {
            return true;
        }
        // Ensure EIP-3607 is disabled
        self.evm_env.write().cfg_env.disable_eip3607 = true;
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

    /// Writes the CREATE2 deployer code directly to the database at the address provided.
    pub async fn set_create2_deployer(&self, address: Address) -> DatabaseResult<()> {
        self.set_code(address, Bytes::from_static(DEFAULT_CREATE2_DEPLOYER_RUNTIME_CODE)).await?;
        Ok(())
    }

    /// Updates memory limits that should be more strict when auto-mine is enabled
    pub(crate) fn update_interval_mine_block_time(&self, block_time: Duration) {
        self.states.write().update_interval_mine_block_time(block_time)
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

    /// The EVM environment data of the blockchain
    pub fn evm_env(&self) -> &Arc<RwLock<EvmEnv>> {
        &self.evm_env
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
        self.evm_env.write().block_env.number = U256::from(number);
    }

    /// Returns the client coinbase address.
    pub fn coinbase(&self) -> Address {
        self.evm_env.read().block_env.beneficiary
    }

    /// Returns the client coinbase address.
    pub fn chain_id(&self) -> U256 {
        U256::from(self.evm_env.read().cfg_env.chain_id)
    }

    pub fn set_chain_id(&self, chain_id: u64) {
        self.evm_env.write().cfg_env.chain_id = chain_id;
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
        self.evm_env.write().block_env.beneficiary = address;
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
        *self.evm_env.read().spec_id()
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
        self.networks.is_optimism()
    }

    /// Returns true if Tempo network mode is active
    pub fn is_tempo(&self) -> bool {
        self.networks.is_tempo()
    }

    /// Returns the active hardfork.
    pub fn hardfork(&self) -> FoundryHardfork {
        self.hardfork
    }

    /// Returns the precompiles for the current spec.
    pub fn precompiles(&self) -> BTreeMap<String, Address> {
        let spec_id = self.spec_id();
        let precompiles = Precompiles::new(PrecompileSpecId::from_spec_id(spec_id));

        let mut precompiles_map = BTreeMap::<String, Address>::default();
        for (address, precompile) in precompiles.inner() {
            precompiles_map.insert(precompile.id().name().to_string(), *address);
        }

        // Extend with configured network precompiles.
        precompiles_map.extend(self.networks.precompiles());

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

        let spec_id = self.spec_id();

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
        let spec_id = self.spec_id();

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

    /// Returns an error if Tempo transactions are not active
    pub fn ensure_tempo_active(&self) -> Result<(), BlockchainError> {
        if self.is_tempo() {
            return Ok(());
        }
        Err(BlockchainError::TempoTransactionUnsupported)
    }

    /// Builds the [`InspectorTxConfig`] from the backend's current settings.
    fn inspector_tx_config(&self) -> InspectorTxConfig {
        InspectorTxConfig {
            print_traces: self.print_traces,
            print_logs: self.print_logs,
            enable_steps_tracing: self.enable_steps_tracing,
            call_trace_decoder: self.call_trace_decoder.clone(),
        }
    }

    /// Builds the [`PoolTxGasConfig`] from the given EVM environment.
    fn pool_tx_gas_config(&self, evm_env: &EvmEnv) -> PoolTxGasConfig {
        let spec_id = *evm_env.spec_id();
        let is_cancun = spec_id >= SpecId::CANCUN;
        let blob_params = self.blob_params();
        PoolTxGasConfig {
            disable_block_gas_limit: evm_env.cfg_env.disable_block_gas_limit,
            tx_gas_limit_cap: evm_env.cfg_env.tx_gas_limit_cap,
            tx_gas_limit_cap_resolved: evm_env.cfg_env.tx_gas_limit_cap(),
            max_blob_gas_per_block: blob_params.max_blob_gas_per_block(),
            is_cancun,
        }
    }

    /// Returns the block gas limit
    pub fn gas_limit(&self) -> u64 {
        self.evm_env.read().block_env.gas_limit
    }

    /// Sets the block gas limit
    pub fn set_gas_limit(&self, gas_limit: u64) {
        self.evm_env.write().block_env.gas_limit = gas_limit;
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

    pub fn list_state_snapshots(&self) -> BTreeMap<U256, (u64, B256)> {
        self.active_state_snapshots.lock().clone().into_iter().collect()
    }

    /// Returns the environment for the next block
    fn next_evm_env(&self) -> EvmEnv {
        let mut evm_env = self.evm_env.read().clone();
        // increase block number for this block
        evm_env.block_env.number = evm_env.block_env.number.saturating_add(U256::from(1));
        evm_env.block_env.basefee = self.base_fee();
        evm_env.block_env.blob_excess_gas_and_price = self.excess_blob_gas_and_price();
        evm_env.block_env.timestamp = U256::from(self.time.current_call_timestamp());
        evm_env
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

    /// Builds an inspector configured for block mining (tracing always enabled).
    fn build_mining_inspector(&self) -> AnvilInspector {
        let mut inspector = AnvilInspector::default().with_tracing();
        if self.enable_steps_tracing {
            inspector = inspector.with_steps_tracing();
        }
        if self.print_logs {
            inspector = inspector.with_log_collector();
        }
        if self.print_traces {
            inspector = inspector.with_trace_printer();
        }
        inspector
    }

    /// Returns a new block event stream that yields Notifications when a new block was added
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

    /// Returns the block number for the given block id
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

    /// Returns the traces for the given transaction
    pub(crate) fn mined_parity_trace_transaction(
        &self,
        hash: B256,
    ) -> Option<Vec<LocalizedTransactionTrace>> {
        self.blockchain.storage.read().transactions.get(&hash).map(|tx| tx.parity_traces())
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
            if let Some(mined_tx) = storage.transactions.get(&tx.hash()) {
                traces.extend(mined_tx.parity_traces());
            }
        }
        Some(traces)
    }

    /// Returns the mined transaction for the given hash
    pub(crate) fn mined_transaction(&self, hash: B256) -> Option<MinedTransaction<N>> {
        self.blockchain.storage.read().transactions.get(&hash).cloned()
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
        let base_fee = block.header.base_fee_per_gas();
        let storage = self.blockchain.storage.read();
        for hash in block.body.transactions.iter().map(|tx| tx.hash()) {
            let info = storage.transactions.get(&hash)?.info.clone();
            let tx = block.body.transactions.get(info.transaction_index as usize)?.clone();

            let tx = transaction_build(Some(hash), tx, Some(block), Some(info), base_fee);
            transactions.push(tx);
        }
        Some(transactions)
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
        if is_arbitrum(self.chain_id().to::<u64>()) {
            // Set `l1BlockNumber` field.
            block.other.insert("l1BlockNumber".to_string(), number.into());
        }

        // Add Tempo-specific header fields for compatibility with TempoNetwork provider.
        if self.is_tempo() {
            let timestamp = block.header.timestamp();
            let gas_limit = block.header.gas_limit();
            block.other.insert(
                "timestampMillis".to_string(),
                serde_json::Value::String(format!("0x{:x}", timestamp.saturating_mul(1000))),
            );
            block.other.insert(
                "mainBlockGeneralGasLimit".to_string(),
                serde_json::Value::String(format!("0x{gas_limit:x}")),
            );
            block
                .other
                .insert("sharedGasLimit".to_string(), serde_json::Value::String("0x0".to_string()));
            block.other.insert(
                "timestampMillisPart".to_string(),
                serde_json::Value::String("0x0".to_string()),
            );
        }

        AnyRpcBlock::from(block)
    }

    pub async fn block_by_hash(&self, hash: B256) -> Result<Option<AnyRpcBlock>, BlockchainError> {
        trace!(target: "backend", "get block by hash {:?}", hash);
        if let tx @ Some(_) = self.mined_block_by_hash(hash) {
            return Ok(tx);
        }

        if let Some(fork) = self.get_fork() {
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
            return Ok(fork.block_by_hash_full(hash).await?);
        }

        Ok(None)
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
                return Ok(fork.block_by_number_full(number).await?);
            }
        }

        Ok(None)
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

    /// Injects all configured precompiles into the given precompile map.
    ///
    /// This applies three layers:
    /// 1. Network-specific precompiles (e.g. Tempo, OP)
    /// 2. User-provided precompiles via [`PrecompileFactory`]
    /// 3. Cheatcode ecrecover overrides (if active)
    fn inject_precompiles(&self, precompiles: &mut PrecompilesMap) {
        self.networks.inject_precompiles(precompiles);

        if let Some(factory) = &self.precompile_factory {
            precompiles.extend_precompiles(factory.precompiles());
        }

        let cheats = Arc::new(self.cheats.clone());
        if cheats.has_recover_overrides() {
            let cheat_ecrecover = CheatEcrecover::new(Arc::clone(&cheats));
            precompiles.apply_precompile(&EC_RECOVER, move |_| {
                Some(DynPrecompile::new_stateful(
                    cheat_ecrecover.precompile_id().clone(),
                    move |input| cheat_ecrecover.call(input),
                ))
            });
        }
    }

    /// Creates a concrete EVM, injects precompiles, transacts, and returns the result mapped
    /// to [`HaltReason`] so all call sites share a single halt-reason type.
    fn transact_with_inspector_ref<'db, I, DB>(
        &self,
        db: &'db DB,
        evm_env: &EvmEnv,
        inspector: &mut I,
        tx_env: OpTransaction<TxEnv>,
    ) -> Result<ResultAndState<HaltReason>, BlockchainError>
    where
        DB: DatabaseRef + ?Sized,
        I: Inspector<EthEvmContext<WrapDatabaseRef<&'db DB>>>
            + Inspector<OpContext<WrapDatabaseRef<&'db DB>>>
            + Inspector<TempoContext<WrapDatabaseRef<&'db DB>>>,
        WrapDatabaseRef<&'db DB>: Database<Error = DatabaseError>,
    {
        if self.is_optimism() {
            let op_env = EvmEnv::new(
                evm_env.cfg_env.clone().with_spec_and_mainnet_gas_params(OpSpecId::ISTHMUS),
                evm_env.block_env.clone(),
            );
            let mut evm = OpEvmFactory::default().create_evm_with_inspector(
                WrapDatabaseRef(db),
                op_env,
                inspector,
            );
            self.inject_precompiles(evm.precompiles_mut());
            let result = evm.transact(tx_env)?;
            Ok(ResultAndState {
                result: result.result.map_haltreason(|h| match h {
                    OpHaltReason::Base(eth) => eth,
                    _ => HaltReason::PrecompileError,
                }),
                state: result.state,
            })
        } else if self.is_tempo() {
            self.transact_tempo_with_inspector_ref(
                db,
                evm_env,
                inspector,
                TempoTxEnv::from(tx_env.base),
            )
        } else {
            let mut evm = EthEvmFactory::default().create_evm_with_inspector(
                WrapDatabaseRef(db),
                evm_env.clone(),
                inspector,
            );
            self.inject_precompiles(evm.precompiles_mut());
            Ok(evm.transact(tx_env.base)?)
        }
    }

    /// Builds the appropriate tx env from a [`FoundryTxEnvelope`], executes via the correct
    /// EVM backend (Op/Tempo/Eth), and returns both the result and the base [`TxEnv`].
    fn transact_envelope_with_inspector_ref<'db, I, DB>(
        &self,
        db: &'db DB,
        evm_env: &EvmEnv,
        inspector: &mut I,
        tx: &FoundryTxEnvelope,
        sender: Address,
    ) -> Result<(ResultAndState<HaltReason>, TxEnv), BlockchainError>
    where
        DB: DatabaseRef + ?Sized,
        I: Inspector<EthEvmContext<WrapDatabaseRef<&'db DB>>>
            + Inspector<OpContext<WrapDatabaseRef<&'db DB>>>
            + Inspector<TempoContext<WrapDatabaseRef<&'db DB>>>,
        WrapDatabaseRef<&'db DB>: Database<Error = DatabaseError>,
    {
        if tx.is_tempo() {
            let tx_env: TempoTxEnv =
                FromTxWithEncoded::from_encoded_tx(tx, sender, tx.encoded_2718().into());
            let base = tx_env.inner.clone();
            let result = self.transact_tempo_with_inspector_ref(db, evm_env, inspector, tx_env)?;
            Ok((result, base))
        } else {
            let tx_env: OpTransaction<TxEnv> =
                FromTxWithEncoded::from_encoded_tx(tx, sender, tx.encoded_2718().into());
            let base = tx_env.base.clone();
            let result = self.transact_with_inspector_ref(db, evm_env, inspector, tx_env)?;
            Ok((result, base))
        }
    }

    /// Creates a Tempo EVM, injects precompiles, and transacts with a native [`TempoTxEnv`].
    fn transact_tempo_with_inspector_ref<'db, I, DB>(
        &self,
        db: &'db DB,
        evm_env: &EvmEnv,
        inspector: &mut I,
        tx_env: TempoTxEnv,
    ) -> Result<ResultAndState<HaltReason>, BlockchainError>
    where
        DB: DatabaseRef + ?Sized,
        I: Inspector<TempoContext<WrapDatabaseRef<&'db DB>>>,
        WrapDatabaseRef<&'db DB>: Database<Error = DatabaseError>,
    {
        let hardfork = TempoHardfork::from(self.hardfork);
        let tempo_env = EvmEnv::new(
            evm_env.cfg_env.clone().with_spec_and_gas_params(hardfork, tempo_gas_params(hardfork)),
            TempoBlockEnv { inner: evm_env.block_env.clone(), timestamp_millis_part: 0 },
        );
        let mut evm = TempoEvmFactory::default().create_evm_with_inspector(
            WrapDatabaseRef(db),
            tempo_env,
            inspector,
        );
        self.inject_precompiles(evm.precompiles_mut());
        let result = evm.transact(tx_env)?;
        Ok(ResultAndState {
            result: result.result.map_haltreason(|h| match h {
                TempoHaltReason::Ethereum(eth) => eth,
                _ => HaltReason::PrecompileError,
            }),
            state: result.state,
        })
    }

    /// Creates a concrete EVM + [`AnvilBlockExecutor`], runs pre-execution changes, and
    /// executes pool transactions. Returns the execution results and drops the EVM.
    #[allow(clippy::too_many_arguments, clippy::type_complexity)]
    fn execute_with_block_executor<DB>(
        &self,
        db: DB,
        evm_env: &EvmEnv,
        parent_hash: B256,
        spec_id: SpecId,
        pool_transactions: &[Arc<PoolTransaction<FoundryTxEnvelope>>],
        gas_config: &PoolTxGasConfig,
        inspector_tx_config: &InspectorTxConfig,
        validator: &dyn Fn(
            &PendingTransaction<FoundryTxEnvelope>,
            &AccountInfo,
        ) -> Result<(), InvalidTransactionError>,
    ) -> (ExecutedPoolTransactions<FoundryTxEnvelope>, BlockExecutionResult<FoundryReceiptEnvelope>)
    where
        DB: StateDB<Error = DatabaseError>,
    {
        let inspector = self.build_mining_inspector();

        macro_rules! run {
            ($evm:expr) => {{
                self.inject_precompiles($evm.precompiles_mut());
                let mut executor = AnvilBlockExecutor::new($evm, parent_hash, spec_id);
                executor.apply_pre_execution_changes().expect("pre-execution changes failed");
                let pool_result = execute_pool_transactions(
                    &mut executor,
                    pool_transactions,
                    gas_config,
                    inspector_tx_config,
                    self.cheats(),
                    validator,
                );
                let (evm, block_result) = executor.finish().expect("executor finish failed");
                drop(evm);
                (pool_result, block_result)
            }};
        }

        if self.is_optimism() {
            let op_env = EvmEnv::new(
                evm_env.cfg_env.clone().with_spec_and_mainnet_gas_params(OpSpecId::ISTHMUS),
                evm_env.block_env.clone(),
            );
            let mut evm = OpEvmFactory::default().create_evm_with_inspector(db, op_env, inspector);
            run!(evm)
        } else if self.is_tempo() {
            let hardfork = TempoHardfork::from(self.hardfork);
            let tempo_env = EvmEnv::new(
                evm_env
                    .cfg_env
                    .clone()
                    .with_spec_and_gas_params(hardfork, tempo_gas_params(hardfork)),
                TempoBlockEnv { inner: evm_env.block_env.clone(), timestamp_millis_part: 0 },
            );
            let mut evm =
                TempoEvmFactory::default().create_evm_with_inspector(db, tempo_env, inspector);
            run!(evm)
        } else {
            let mut evm =
                EthEvmFactory::default().create_evm_with_inspector(db, evm_env.clone(), inspector);
            run!(evm)
        }
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
    ) -> (EvmEnv, OpTransaction<TxEnv>) {
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
        let mut evm_env = self.evm_env.read().clone();
        evm_env.block_env = block_env;
        // we want to disable this in eth_call, since this is common practice used by other node
        // impls and providers <https://github.com/foundry-rs/foundry/issues/4388>
        evm_env.cfg_env.disable_block_gas_limit = true;
        evm_env.cfg_env.tx_gas_limit_cap = Some(u64::MAX);

        // The basefee should be ignored for calls against state for
        // - eth_call
        // - eth_estimateGas
        // - eth_createAccessList
        // - tracing
        evm_env.cfg_env.disable_base_fee = true;

        // Disable nonce check in revm
        evm_env.cfg_env.disable_nonce_check = true;

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
                    if blob_hashes.is_empty() { Some(0) } else { evm_env.block_env.blob_gasprice() }
                })
                .unwrap_or_default(),
            kind: match to {
                Some(addr) => TxKind::Call(*addr),
                None => TxKind::Create,
            },
            tx_type,
            value: value.unwrap_or_default(),
            data: input.into_input().unwrap_or_default(),
            chain_id: Some(chain_id.unwrap_or(self.chain_id().to::<u64>())),
            access_list: access_list.unwrap_or_default(),
            blob_hashes,
            ..Default::default()
        };
        base.set_signed_authorization(authorization_list.unwrap_or_default());
        let mut tx_env = OpTransaction { base, ..Default::default() };

        if let Some(nonce) = nonce {
            tx_env.base.nonce = nonce;
        }

        if evm_env.block_env.basefee == 0 {
            // this is an edge case because the evm fails if `tx.effective_gas_price < base_fee`
            // 0 is only possible if it's manually set
            evm_env.cfg_env.disable_base_fee = true;
        }

        // Deposit transaction?
        if let Ok(deposit) = get_deposit_tx_parts(&other) {
            tx_env.deposit = deposit;
        }

        (evm_env, tx_env)
    }

    pub fn call_with_state(
        &self,
        state: &dyn DatabaseRef,
        request: WithOtherFields<TransactionRequest>,
        fee_details: FeeDetails,
        block_env: BlockEnv,
    ) -> Result<(InstructionResult, Option<Output>, u128, State), BlockchainError> {
        let mut inspector = self.build_inspector();

        // Extract Tempo-specific fields before `build_call_env` consumes `other`.
        let tempo_overrides = self.is_tempo().then(|| {
            let fee_token =
                request.other.get_deserialized::<Address>("feeToken").and_then(|r| r.ok());
            let nonce_key = request
                .other
                .get_deserialized::<U256>("nonceKey")
                .and_then(|r| r.ok())
                .unwrap_or_default();
            let valid_before = request
                .other
                .get_deserialized::<U256>("validBefore")
                .and_then(|r| r.ok())
                .map(|v| v.saturating_to::<u64>());
            let valid_after = request
                .other
                .get_deserialized::<U256>("validAfter")
                .and_then(|r| r.ok())
                .map(|v| v.saturating_to::<u64>());
            (fee_token, nonce_key, valid_before, valid_after)
        });

        let (evm_env, tx_env) = self.build_call_env(request, fee_details, block_env);

        let ResultAndState { result, state } =
            if let Some((fee_token, nonce_key, valid_before, valid_after)) = tempo_overrides {
                use tempo_primitives::transaction::Call;

                let base = tx_env.base;
                let mut tempo_tx = TempoTxEnv::from(base.clone());
                tempo_tx.fee_token = fee_token;

                if !nonce_key.is_zero() || valid_before.is_some() || valid_after.is_some() {
                    // For gas estimation we don't have a signed tx, so generate a
                    // unique hash for expiring-nonce replay protection.  The nonce
                    // manager needs a non-zero hash; the actual value doesn't matter
                    // because the state is discarded after estimation.
                    let estimation_hash = keccak256(base.data.as_ref());
                    tempo_tx.tempo_tx_env = Some(Box::new(TempoBatchCallEnv {
                        nonce_key,
                        valid_before,
                        valid_after,
                        aa_calls: vec![Call { to: base.kind, value: base.value, input: base.data }],
                        tx_hash: estimation_hash,
                        expiring_nonce_hash: Some(estimation_hash),
                        ..Default::default()
                    }));
                }
                self.transact_tempo_with_inspector_ref(state, &evm_env, &mut inspector, tempo_tx)?
            } else {
                self.transact_with_inspector_ref(state, &evm_env, &mut inspector, tx_env)?
            };

        let (exit_reason, gas_used, out, _logs) = unpack_execution_result(result);
        inspector.print_logs();

        if self.print_traces {
            inspector.into_print_traces(self.call_trace_decoder.clone());
        }

        Ok((exit_reason, out, gas_used as u128, state))
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

        let (evm_env, tx_env) = self.build_call_env(request, fee_details, block_env);
        let ResultAndState { result, state: _ } =
            self.transact_with_inspector_ref(state, &evm_env, &mut inspector, tx_env)?;
        let (exit_reason, gas_used, out, _logs) = unpack_execution_result(result);
        let access_list = inspector.access_list();
        Ok((exit_reason, out, gas_used, access_list))
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
            block.header.base_fee_per_gas(),
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
            block.header.base_fee_per_gas(),
        ))
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

    /// Replays all transactions in a block and returns the requested traces for each transaction
    pub async fn trace_replay_block_transactions(
        &self,
        block: BlockNumber,
        trace_types: HashSet<TraceType>,
    ) -> Result<Vec<TraceResultsWithTransactionHash>, BlockchainError> {
        let block_number = self.convert_block_number(Some(block));

        // Try mined blocks first
        if let Some(results) =
            self.mined_parity_trace_replay_block_transactions(block_number, &trace_types)
        {
            return Ok(results);
        }

        // Fallback to fork if block predates fork
        if let Some(fork) = self.get_fork()
            && fork.predates_fork(block_number)
        {
            return Ok(fork.trace_replay_block_transactions(block_number, trace_types).await?);
        }

        Ok(vec![])
    }

    /// Returns the trace results for all transactions in a mined block by replaying them
    fn mined_parity_trace_replay_block_transactions(
        &self,
        block_number: u64,
        trace_types: &HashSet<TraceType>,
    ) -> Option<Vec<TraceResultsWithTransactionHash>> {
        let block = self.get_block(block_number)?;

        // Execute this in the context of the parent state
        let parent_hash = block.header.parent_hash;
        let trace_config = TracingInspectorConfig::from_parity_config(trace_types);

        let read_guard = self.states.upgradable_read();
        if let Some(state) = read_guard.get_state(&parent_hash) {
            self.replay_block_transactions_with_inspector(&block, state, trace_config, trace_types)
        } else {
            let mut write_guard = RwLockUpgradableReadGuard::upgrade(read_guard);
            let state = write_guard.get_on_disk_state(&parent_hash)?;
            self.replay_block_transactions_with_inspector(&block, state, trace_config, trace_types)
        }
    }

    /// Replays all transactions in a block with the tracing inspector to generate TraceResults
    fn replay_block_transactions_with_inspector(
        &self,
        block: &Block,
        parent_state: &StateDb,
        trace_config: TracingInspectorConfig,
        trace_types: &HashSet<TraceType>,
    ) -> Option<Vec<TraceResultsWithTransactionHash>> {
        let mut cache_db = CacheDB::new(Box::new(parent_state));
        let mut results = Vec::new();

        // Configure the block environment
        let mut evm_env = self.evm_env.read().clone();
        evm_env.block_env = block_env_from_header(&block.header);

        // Execute each transaction in the block with tracing
        for tx_envelope in &block.body.transactions {
            let tx_hash = tx_envelope.hash();

            // Create a fresh inspector for this transaction
            let mut inspector = TracingInspector::new(trace_config);

            // Prepare transaction environment and execute
            let pending_tx =
                PendingTransaction::from_maybe_impersonated(tx_envelope.clone()).ok()?;
            let (result, _) = self
                .transact_envelope_with_inspector_ref(
                    &cache_db,
                    &evm_env,
                    &mut inspector,
                    pending_tx.transaction.as_ref(),
                    *pending_tx.sender(),
                )
                .ok()?;

            // Build TraceResults from the inspector and execution result
            let full_trace = inspector
                .into_parity_builder()
                .into_trace_results_with_state(&result, trace_types, &cache_db)
                .ok()?;

            results.push(TraceResultsWithTransactionHash { transaction_hash: tx_hash, full_trace });

            // Commit the state changes for the next transaction
            cache_db.commit(result.state);
        }

        Some(results)
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
                    sidecar.sidecar.blobs().iter().zip(sidecar.sidecar.commitments().iter())
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

    #[allow(clippy::large_stack_frames)]
    pub fn get_blob_by_versioned_hash(&self, hash: B256) -> Result<Option<Blob>> {
        let storage = self.blockchain.storage.read();
        for block in storage.blocks.values() {
            for tx in &block.body.transactions {
                let typed_tx = tx.as_ref();
                if let Some(sidecar) = typed_tx.sidecar() {
                    for versioned_hash in sidecar.sidecar.versioned_hashes() {
                        if versioned_hash == hash
                            && let Some(index) =
                                sidecar.sidecar.commitments().iter().position(|commitment| {
                                    kzg_to_versioned_hash(commitment.as_slice()) == *hash
                                })
                            && let Some(blob) = sidecar.sidecar.blobs().get(index)
                        {
                            return Ok(Some(*blob));
                        }
                    }
                }
            }
        }
        Ok(None)
    }

    /// Initialises the balance of the given accounts
    #[expect(clippy::too_many_arguments)]
    pub async fn with_genesis(
        db: Arc<AsyncRwLock<Box<dyn Db>>>,
        env: Arc<RwLock<EvmEnv>>,
        networks: NetworkConfigs,
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
            Blockchain::new(
                &env.read(),
                fees.is_eip1559().then(|| fees.base_fee()),
                genesis.timestamp,
                genesis.number,
            )
        };

        // Sync EVM block.number with genesis for non-fork mode.
        // Fork mode syncs in setup_fork_db_config() instead.
        if fork.read().is_none() {
            env.write().block_env.number = U256::from(genesis.number);
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

        let (slots_in_an_epoch, precompile_factory, disable_pool_balance_checks, hardfork) = {
            let cfg = node_config.read().await;
            (
                cfg.slots_in_an_epoch,
                cfg.precompile_factory.clone(),
                cfg.disable_pool_balance_checks,
                cfg.get_hardfork(),
            )
        };

        let backend = Self {
            db,
            blockchain,
            states: Arc::new(RwLock::new(states)),
            evm_env: env,
            networks,
            hardfork,
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
            disable_pool_balance_checks,
        };

        if let Some(interval_block_time) = automine_block_time {
            backend.update_interval_mine_block_time(interval_block_time);
        }

        // Note: this can only fail in forking mode, in which case we can't recover
        backend.apply_genesis().await.wrap_err("failed to create genesis")?;
        Ok(backend)
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

            // Deploy EIP-2935 blockhash history storage contract if Prague is active.
            if self.spec_id() >= SpecId::PRAGUE {
                db.set_code(
                    eip2935::HISTORY_STORAGE_ADDRESS,
                    eip2935::HISTORY_STORAGE_CODE.clone(),
                )?;
            }
        }

        let db = self.db.write().await;
        // apply the genesis.json alloc
        self.genesis.apply_genesis_json_alloc(db)?;

        // Initialize Tempo precompiles and fee tokens when in Tempo mode (not in fork mode).
        // In fork mode, precompiles are inherited from the forked origin.
        if self.networks.is_tempo() && !self.is_fork() {
            let chain_id = self.evm_env.read().cfg_env.chain_id;
            let timestamp = self.genesis.timestamp;
            let test_accounts: Vec<Address> = self.genesis.accounts.clone();
            let hardfork = TempoHardfork::from(self.hardfork);
            let mut db = self.db.write().await;
            crate::eth::backend::tempo::initialize_tempo_precompiles(
                &mut **db,
                chain_id,
                timestamp,
                &test_accounts,
                hardfork,
            )
            .map_err(|e| {
                tracing::error!(target: "backend", "failed to initialize Tempo precompiles: {e}");
                DatabaseError::AnyRequest(Arc::new(eyre::eyre!("{e}")))
            })?;
            trace!(target: "backend", "initialized Tempo precompiles and fee tokens for {} accounts", test_accounts.len());
        }

        trace!(target: "backend", "set genesis balances");

        Ok(())
    }

    /// Resets the fork to a fresh state
    pub async fn reset_fork(&self, forking: Forking) -> Result<(), BlockchainError> {
        if !self.is_fork() {
            if let Some(eth_rpc_url) = forking.json_rpc_url.clone() {
                let mut evm_env = self.evm_env.read().clone();

                let (db, config) = {
                    let mut node_config = self.node_config.write().await;

                    // we want to force the correct base fee for the next block during
                    // `setup_fork_db_config`
                    node_config.base_fee.take();

                    node_config.setup_fork_db_config(eth_rpc_url, &mut evm_env, &self.fees).await?
                };

                *self.db.write().await = Box::new(db);

                let fork = ClientFork::new(config, Arc::clone(&self.db));

                *self.evm_env.write() = evm_env;
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
                    let mut env = self.evm_env.write();

                    env.cfg_env.chain_id = fork.chain_id();
                    env.block_env = BlockEnv {
                        number: U256::from(fork_block_number),
                        timestamp: U256::from(fork_block.header.timestamp()),
                        gas_limit,
                        difficulty: fork_block.header.difficulty(),
                        prevrandao: Some(fork_block.header.mix_hash().unwrap_or_default()),
                        // Keep previous `beneficiary` and `basefee` value
                        beneficiary: env.block_env.beneficiary,
                        basefee: env.block_env.basefee,
                        ..env.block_env.clone()
                    };

                    // this is the base fee of the current block, but we need the base fee of
                    // the next block
                    let next_block_base_fee = self.fees.get_next_block_base_fee_per_gas(
                        fork_block.header.gas_used(),
                        gas_limit,
                        fork_block.header.base_fee_per_gas().unwrap_or_default(),
                    );

                    self.fees.set_base_fee(next_block_base_fee);
                }

                // reset the time to the timestamp of the forked block
                self.time.reset(fork_block.header.timestamp());

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

        let genesis_timestamp = self.genesis.timestamp;
        let genesis_number = self.genesis.number;

        // Reset environment to genesis state
        {
            let mut env = self.evm_env.write();
            env.block_env.number = U256::from(genesis_number);
            env.block_env.timestamp = U256::from(genesis_timestamp);
            // Reset other block env fields to their defaults
            env.block_env.basefee = self.fees.base_fee();
            env.block_env.prevrandao = Some(B256::ZERO);
        }

        // Clear all storage and reinitialize with genesis
        let base_fee = self.fees.is_eip1559().then(|| self.fees.base_fee());
        *self.blockchain.storage.write() = BlockchainStorage::new(
            &self.evm_env.read(),
            base_fee,
            genesis_timestamp,
            genesis_number,
        );
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

        let mut evm_env = self.evm_env.read().clone();
        let (forked_db, client_fork_config) =
            node_config.setup_fork_db_config(fork_url, &mut evm_env, &self.fees).await?;

        *self.db.write().await = Box::new(forked_db);
        let fork = ClientFork::new(client_fork_config, Arc::clone(&self.db));
        *self.fork.write() = Some(fork);
        *self.evm_env.write() = evm_env;

        Ok(())
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

            let reset_time = block.header.timestamp();
            self.time.reset(reset_time);

            let mut env = self.evm_env.write();
            env.block_env = BlockEnv {
                number: U256::from(num),
                timestamp: U256::from(block.header.timestamp()),
                difficulty: block.header.difficulty(),
                // ensures prevrandao is set
                prevrandao: Some(block.header.mix_hash().unwrap_or_default()),
                gas_limit: block.header.gas_limit(),
                // Keep previous `beneficiary` and `basefee` value
                beneficiary: env.block_env.beneficiary,
                basefee: env.block_env.basefee,
                ..Default::default()
            }
        }
        Ok(self.db.write().await.revert_state(id, RevertStateSnapshotAction::RevertRemove))
    }

    /// executes the transactions without writing to the underlying database
    pub async fn inspect_tx(
        &self,
        tx: Arc<PoolTransaction<FoundryTxEnvelope>>,
    ) -> Result<
        (InstructionResult, Option<Output>, u64, State, Vec<revm::primitives::Log>),
        BlockchainError,
    > {
        let evm_env = self.next_evm_env();
        let db = self.db.read().await;
        let mut inspector = self.build_inspector();
        let (ResultAndState { result, state }, _) = self.transact_envelope_with_inspector_ref(
            &**db,
            &evm_env,
            &mut inspector,
            tx.pending_transaction.transaction.as_ref(),
            *tx.pending_transaction.sender(),
        )?;
        let (exit_reason, gas_used, out, logs) = unpack_execution_result(result);

        inspector.print_logs();

        if self.print_traces {
            inspector.print_traces(self.call_trace_decoder.clone());
        }

        Ok((exit_reason, out, gas_used, state, logs))
    }
}

impl<N: Network> Backend<N>
where
    N::ReceiptEnvelope: TxReceipt<Log = alloy_primitives::Log>,
{
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
                        block_number: Some(block.header.number()),
                        block_timestamp: Some(block.header.timestamp()),
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
                return Err(BlockchainError::BlockOutOfRange(best, from_block));
            }

            self.logs_for_range(&filter, from_block, to_block).await
        }
    }

    /// Returns all receipts of the block
    pub fn mined_receipts(&self, hash: B256) -> Option<Vec<N::ReceiptEnvelope>> {
        let block = self.mined_block_by_hash(hash)?;
        let mut receipts = Vec::new();
        let storage = self.blockchain.storage.read();
        for tx in block.transactions.hashes() {
            let receipt = storage.transactions.get(&tx)?.receipt.clone();
            receipts.push(receipt);
        }
        Some(receipts)
    }
}

// Mining methods — generic over N: Network, with Foundry-associated-type bounds for now.
impl<N: Network> Backend<N>
where
    Self: TransactionValidator<FoundryTxEnvelope>,
    N: Network<TxEnvelope = FoundryTxEnvelope, ReceiptEnvelope = FoundryReceiptEnvelope>,
{
    /// Mines a new block and stores it.
    ///
    /// this will execute all transaction in the order they come in and return all the markers they
    /// provide.
    pub async fn mine_block(
        &self,
        pool_transactions: Vec<Arc<PoolTransaction<FoundryTxEnvelope>>>,
    ) -> MinedBlockOutcome<FoundryTxEnvelope> {
        self.do_mine_block(pool_transactions).await
    }

    /// Builds a [`BlockInfo`] from the EVM environment, execution results, and transactions.
    fn build_block_info(
        evm_env: &EvmEnv,
        parent_hash: B256,
        number: u64,
        state_root: B256,
        block_result: BlockExecutionResult<FoundryReceiptEnvelope>,
        transactions: Vec<MaybeImpersonatedTransaction<FoundryTxEnvelope>>,
        transaction_infos: Vec<TransactionInfo>,
    ) -> BlockInfo<N> {
        let spec_id = *evm_env.spec_id();
        let is_shanghai = spec_id >= SpecId::SHANGHAI;
        let is_cancun = spec_id >= SpecId::CANCUN;
        let is_prague = spec_id >= SpecId::PRAGUE;

        let receipts_root = calculate_receipt_root(&block_result.receipts);
        let cumulative_blob_gas_used = is_cancun.then_some(block_result.blob_gas_used);
        let bloom = block_result.receipts.iter().fold(Bloom::default(), |mut b, r| {
            b.accrue_bloom(r.logs_bloom());
            b
        });

        let header = Header {
            parent_hash,
            ommers_hash: Default::default(),
            beneficiary: evm_env.block_env.beneficiary,
            state_root,
            transactions_root: Default::default(),
            receipts_root,
            logs_bloom: bloom,
            difficulty: evm_env.block_env.difficulty,
            number,
            gas_limit: evm_env.block_env.gas_limit,
            gas_used: block_result.gas_used,
            timestamp: evm_env.block_env.timestamp.saturating_to(),
            extra_data: Default::default(),
            mix_hash: evm_env.block_env.prevrandao.unwrap_or_default(),
            nonce: Default::default(),
            base_fee_per_gas: (spec_id >= SpecId::LONDON).then_some(evm_env.block_env.basefee),
            parent_beacon_block_root: is_cancun.then_some(Default::default()),
            blob_gas_used: cumulative_blob_gas_used,
            excess_blob_gas: if is_cancun { evm_env.block_env.blob_excess_gas() } else { None },
            withdrawals_root: is_shanghai.then_some(EMPTY_WITHDRAWALS),
            requests_hash: is_prague.then_some(EMPTY_REQUESTS_HASH),
        };

        let block = create_block(header, transactions);
        BlockInfo { block, transactions: transaction_infos, receipts: block_result.receipts }
    }

    async fn do_mine_block(
        &self,
        pool_transactions: Vec<Arc<PoolTransaction<FoundryTxEnvelope>>>,
    ) -> MinedBlockOutcome<FoundryTxEnvelope> {
        let _mining_guard = self.mining.lock().await;
        trace!(target: "backend", "creating new block with {} transactions", pool_transactions.len());

        let (outcome, header, block_hash) = {
            let current_base_fee = self.base_fee();
            let current_excess_blob_gas_and_price = self.excess_blob_gas_and_price();

            let mut evm_env = self.evm_env.read().clone();

            if evm_env.block_env.basefee == 0 {
                // this is an edge case because the evm fails if `tx.effective_gas_price < base_fee`
                // 0 is only possible if it's manually set
                evm_env.cfg_env.disable_base_fee = true;
            }

            let block_number = self.blockchain.storage.read().best_number.saturating_add(1);

            // increase block number for this block
            if is_arbitrum(evm_env.cfg_env.chain_id) {
                // Temporary set `env.block.number` to `block_number` for Arbitrum chains.
                evm_env.block_env.number = U256::from(block_number);
            } else {
                evm_env.block_env.number = evm_env.block_env.number.saturating_add(U256::from(1));
            }

            evm_env.block_env.basefee = current_base_fee;
            evm_env.block_env.blob_excess_gas_and_price = current_excess_blob_gas_and_price;

            let best_hash = self.blockchain.storage.read().best_hash;

            let mut input = Vec::with_capacity(40);
            input.extend_from_slice(best_hash.as_slice());
            input.extend_from_slice(&block_number.to_le_bytes());
            evm_env.block_env.prevrandao = Some(keccak256(&input));

            if self.prune_state_history_config.is_state_history_supported() {
                let db = self.db.read().await.current_state();
                // store current state before executing all transactions
                self.states.write().insert(best_hash, db);
            }

            let (block_info, included, invalid, not_yet_valid, block_hash) = {
                let mut db = self.db.write().await;

                // finally set the next block timestamp, this is done just before execution, because
                // there can be concurrent requests that can delay acquiring the db lock and we want
                // to ensure the timestamp is as close as possible to the actual execution.
                evm_env.block_env.timestamp = U256::from(self.time.next_timestamp());

                let spec_id = *evm_env.spec_id();

                let inspector_tx_config = self.inspector_tx_config();
                let gas_config = self.pool_tx_gas_config(&evm_env);

                let (pool_result, block_result) = self.execute_with_block_executor(
                    &mut **db,
                    &evm_env,
                    best_hash,
                    spec_id,
                    &pool_transactions,
                    &gas_config,
                    &inspector_tx_config,
                    &|pending, account| {
                        self.validate_pool_transaction_for(pending, account, &evm_env)
                    },
                );

                let included = pool_result.included;
                let invalid = pool_result.invalid;
                let not_yet_valid = pool_result.not_yet_valid;

                let state_root = db.maybe_state_root().unwrap_or_default();
                let block_info = Self::build_block_info(
                    &evm_env,
                    best_hash,
                    block_number,
                    state_root,
                    block_result,
                    pool_result.txs,
                    pool_result.tx_info,
                );

                // update the new blockhash in the db itself
                let block_hash = block_info.block.header.hash_slow();
                db.insert_block_hash(U256::from(block_info.block.header.number()), block_hash);

                (block_info, included, invalid, not_yet_valid, block_hash)
            };

            // create the new block with the current timestamp
            let BlockInfo { block, transactions, receipts } = block_info;

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
            evm_env.block_env.difficulty = U256::from(0);

            // update env with new values
            *self.evm_env.write() = evm_env;

            let timestamp = utc_from_secs(header.timestamp);

            node_info!("    Block Number: {}", block_number);
            node_info!("    Block Hash: {:?}", block_hash);
            if timestamp.year() > 9999 {
                // rf2822 panics with more than 4 digits
                node_info!("    Block Time: {:?}\n", timestamp.to_rfc3339());
            } else {
                node_info!("    Block Time: {:?}\n", timestamp.to_rfc2822());
            }

            let outcome = MinedBlockOutcome { block_number, included, invalid, not_yet_valid };

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
            get_blob_base_fee_update_fraction_by_spec_id(*self.evm_env.read().spec_id()),
        ));

        // notify all listeners
        self.notify_on_new_block(header, block_hash);

        outcome
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
        tx_pairs: HashMap<u64, Vec<Arc<PoolTransaction<FoundryTxEnvelope>>>>,
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

    /// Creates the pending block
    ///
    /// This will execute all transaction in the order they come but will not mine the block
    pub async fn pending_block(
        &self,
        pool_transactions: Vec<Arc<PoolTransaction<FoundryTxEnvelope>>>,
    ) -> BlockInfo<N> {
        self.with_pending_block(pool_transactions, |_, block| block).await
    }

    /// Creates the pending block
    ///
    /// This will execute all transaction in the order they come but will not mine the block
    pub async fn with_pending_block<F, T>(
        &self,
        pool_transactions: Vec<Arc<PoolTransaction<FoundryTxEnvelope>>>,
        f: F,
    ) -> T
    where
        F: FnOnce(Box<dyn MaybeFullDatabase + '_>, BlockInfo<N>) -> T,
    {
        let db = self.db.read().await;
        let evm_env = self.next_evm_env();

        let mut cache_db = AnvilCacheDB::new(&*db);

        let parent_hash = self.blockchain.storage.read().best_hash;

        let spec_id = *evm_env.spec_id();

        let inspector_tx_config = self.inspector_tx_config();
        let gas_config = self.pool_tx_gas_config(&evm_env);

        let (pool_result, block_result) = self.execute_with_block_executor(
            &mut cache_db,
            &evm_env,
            parent_hash,
            spec_id,
            &pool_transactions,
            &gas_config,
            &inspector_tx_config,
            &|pending, account| self.validate_pool_transaction_for(pending, account, &evm_env),
        );

        // Extract inner CacheDB (which implements MaybeFullDatabase)
        let cache_db = cache_db.0;

        let state_root = cache_db.maybe_state_root().unwrap_or_default();
        let block_number = evm_env.block_env.number.saturating_to();
        let block_info = Self::build_block_info(
            &evm_env,
            parent_hash,
            block_number,
            state_root,
            block_result,
            pool_result.txs,
            pool_result.tx_info,
        );

        f(Box::new(cache_db), block_info)
    }

    /// Returns the ERC20/TIP20 token balance for an account.
    ///
    /// Calls `balanceOf(address)` on the token contract. Returns `U256::ZERO` if
    /// the call fails (e.g. the token contract doesn't exist).
    pub async fn get_fee_token_balance(
        &self,
        token: Address,
        account: Address,
    ) -> Result<U256, BlockchainError> {
        // balanceOf(address) selector: 0x70a08231
        let mut calldata = vec![0x70, 0xa0, 0x82, 0x31];
        // ABI-encode the address (left-padded to 32 bytes)
        calldata.extend_from_slice(&[0u8; 12]);
        calldata.extend_from_slice(account.as_slice());

        let request = WithOtherFields::new(TransactionRequest {
            from: Some(Address::ZERO),
            to: Some(TxKind::Call(token)),
            input: calldata.into(),
            ..Default::default()
        });

        let fee_details = FeeDetails::zero();
        let (exit, out, _, _) = self.call(request, fee_details, None, Default::default()).await?;

        // Check if call succeeded
        if exit != InstructionResult::Return && exit != InstructionResult::Stop {
            // Return zero balance if call failed (token might not exist)
            return Ok(U256::ZERO);
        }

        // Decode U256 from output
        match out {
            Some(Output::Call(data)) if data.len() >= 32 => Ok(U256::from_be_slice(&data[..32])),
            _ => Ok(U256::ZERO),
        }
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
        block_request: Option<BlockRequest<FoundryTxEnvelope>>,
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

    pub async fn call_with_tracing(
        &self,
        request: WithOtherFields<TransactionRequest>,
        fee_details: FeeDetails,
        block_request: Option<BlockRequest<FoundryTxEnvelope>>,
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

                            let (evm_env, tx_env) =
                                self.build_call_env(request, fee_details, block);
                            let ResultAndState { result, state: _ } = self
                                .transact_with_inspector_ref(
                                    &cache_db,
                                    &evm_env,
                                    &mut inspector,
                                    tx_env,
                                )?;

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

                            let (evm_env, tx_env) =
                                self.build_call_env(request, fee_details, block);
                            let result = self.transact_with_inspector_ref(
                                &cache_db,
                                &evm_env,
                                &mut inspector,
                                tx_env,
                            )?;

                            Ok(inspector
                                .into_geth_builder()
                                .geth_prestate_traces(&result, &pre_state_config, cache_db)?
                                .into())
                        }
                        GethDebugBuiltInTracerType::NoopTracer => Ok(NoopFrame::default().into()),
                        GethDebugBuiltInTracerType::FourByteTracer
                        | GethDebugBuiltInTracerType::MuxTracer
                        | GethDebugBuiltInTracerType::FlatCallTracer
                        | GethDebugBuiltInTracerType::Erc7562Tracer => {
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

                        let (evm_env, tx_env) =
                            self.build_call_env(request, fee_details, block.clone());
                        let result = self.transact_with_inspector_ref(
                            &cache_db,
                            &evm_env,
                            &mut inspector,
                            tx_env.clone(),
                        )?;
                        let res = inspector
                            .json_result(result, &tx_env.into_tx_env(), &block, &cache_db)
                            .map_err(|err| BlockchainError::Message(err.to_string()))?;

                        Ok(GethTrace::JS(res))
                    }
                };
            }

            // defaults to StructLog tracer used since no tracer is specified
            let mut inspector = self
                .build_inspector()
                .with_tracing_config(TracingInspectorConfig::from_geth_config(&config));

            let (evm_env, tx_env) = self.build_call_env(request, fee_details, block);
            let ResultAndState { result, state: _ } =
                self.transact_with_inspector_ref(&cache_db, &evm_env, &mut inspector, tx_env)?;

            let (exit_reason, gas_used, out, _logs) = unpack_execution_result(result);

            let tracing_inspector = inspector.tracer.expect("tracer disappeared");
            let return_value = out.as_ref().map(|o| o.data()).cloned().unwrap_or_default();

            trace!(target: "backend", ?exit_reason, ?out, %gas_used, %block_number, "trace call");

            let res = tracing_inspector
                .into_geth_builder()
                .geth_traces(gas_used, return_value, config)
                .into();

            Ok(res)
        })
        .await?
    }

    /// Helper function to execute a closure with the database at a specific block
    pub async fn with_database_at<F, T>(
        &self,
        block_request: Option<BlockRequest<FoundryTxEnvelope>>,
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
                        f(state, block_env_from_header(&block.header))
                    })
                    .await;
                return Ok(result);
            }
            Some(BlockRequest::Number(bn)) => Some(BlockNumber::Number(bn)),
            None => None,
        };
        let block_number = self.convert_block_number(block_number);
        let current_number = self.best_number();

        // Reject requests for future blocks that don't exist yet
        if block_number > current_number {
            return Err(BlockchainError::BlockOutOfRange(current_number, block_number));
        }

        if block_number < current_number {
            if let Some((block_hash, block)) = self
                .block_by_number(BlockNumber::Number(block_number))
                .await?
                .map(|block| (block.header.hash, block))
            {
                let read_guard = self.states.upgradable_read();
                if let Some(state_db) = read_guard.get_state(&block_hash) {
                    return Ok(f(Box::new(state_db), block_env_from_header(&block.header)));
                }

                let mut write_guard = RwLockUpgradableReadGuard::upgrade(read_guard);
                if let Some(state) = write_guard.get_on_disk_state(&block_hash) {
                    return Ok(f(Box::new(state), block_env_from_header(&block.header)));
                }
            }

            warn!(target: "backend", "Not historic state found for block={}", block_number);
            return Err(BlockchainError::BlockOutOfRange(current_number, block_number));
        }

        let db = self.db.read().await;
        let block = self.evm_env.read().block_env.clone();
        Ok(f(Box::new(&**db), block))
    }

    pub async fn storage_at(
        &self,
        address: Address,
        index: U256,
        block_request: Option<BlockRequest<FoundryTxEnvelope>>,
    ) -> Result<B256, BlockchainError> {
        self.with_database_at(block_request, |db, _| {
            trace!(target: "backend", "get storage for {:?} at {:?}", address, index);
            let val = db.storage_ref(address, index)?;
            Ok(val.into())
        })
        .await?
    }

    /// Returns storage values for multiple accounts and slots in a single call.
    pub async fn storage_values(
        &self,
        requests: HashMap<Address, Vec<B256>>,
        block_request: Option<BlockRequest<FoundryTxEnvelope>>,
    ) -> Result<HashMap<Address, Vec<B256>>, BlockchainError> {
        self.with_database_at(block_request, |db, _| {
            trace!(target: "backend", "get storage values for {} addresses", requests.len());
            let mut result: HashMap<Address, Vec<B256>> = HashMap::default();
            for (address, slots) in &requests {
                let mut values = Vec::with_capacity(slots.len());
                for slot in slots {
                    let val = db.storage_ref(*address, (*slot).into())?;
                    values.push(val.into());
                }
                result.insert(*address, values);
            }
            Ok(result)
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
        block_request: Option<BlockRequest<FoundryTxEnvelope>>,
    ) -> Result<Bytes, BlockchainError> {
        self.with_database_at(block_request, |db, _| self.get_code_with_state(&db, address)).await?
    }

    /// Returns the balance of the address
    ///
    /// If the requested number predates the fork then this will fetch it from the endpoint
    pub async fn get_balance(
        &self,
        address: Address,
        block_request: Option<BlockRequest<FoundryTxEnvelope>>,
    ) -> Result<U256, BlockchainError> {
        self.with_database_at(block_request, |db, _| self.get_balance_with_state(db, address))
            .await?
    }

    pub async fn get_account_at_block(
        &self,
        address: Address,
        block_request: Option<BlockRequest<FoundryTxEnvelope>>,
    ) -> Result<TrieAccount, BlockchainError> {
        self.with_database_at(block_request, |block_db, _| {
            let db = block_db.maybe_as_full_db().ok_or(BlockchainError::DataUnavailable)?;
            let account = db.get(&address).cloned().unwrap_or_default();
            let storage_root = storage_root(&account.storage);
            let code_hash = account.info.code_hash;
            let balance = account.info.balance;
            let nonce = account.info.nonce;
            Ok(TrieAccount { balance, nonce, code_hash, storage_root })
        })
        .await?
    }

    /// Returns the nonce of the address
    ///
    /// If the requested number predates the fork then this will fetch it from the endpoint
    pub async fn get_nonce(
        &self,
        address: Address,
        block_request: BlockRequest<FoundryTxEnvelope>,
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

    fn replay_tx_with_inspector<I, F, T>(
        &self,
        hash: B256,
        mut inspector: I,
        f: F,
    ) -> Result<T, BlockchainError>
    where
        for<'a> I: Inspector<EthEvmContext<WrapDatabaseRef<&'a CacheDB<Box<&'a StateDb>>>>>
            + Inspector<OpContext<WrapDatabaseRef<&'a CacheDB<Box<&'a StateDb>>>>>
            + Inspector<TempoContext<WrapDatabaseRef<&'a CacheDB<Box<&'a StateDb>>>>>
            + 'a,
        for<'a> F:
            FnOnce(ResultAndState<HaltReason>, CacheDB<Box<&'a StateDb>>, I, TxEnv, EvmEnv) -> T,
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

        let pool_txs: Vec<Arc<PoolTransaction<FoundryTxEnvelope>>> = block.body.transactions
            [..index]
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
            let mut cache_db = AnvilCacheDB::new(Box::new(parent_state));

            // configure the blockenv for the block of the transaction
            let mut evm_env = self.evm_env.read().clone();

            evm_env.block_env = block_env_from_header(&block.header);

            let spec_id = *evm_env.spec_id();

            let inspector_tx_config = self.inspector_tx_config();
            let gas_config = self.pool_tx_gas_config(&evm_env);

            self.execute_with_block_executor(
                &mut cache_db,
                &evm_env,
                block.header.parent_hash,
                spec_id,
                &pool_txs,
                &gas_config,
                &inspector_tx_config,
                &|pending, account| self.validate_pool_transaction_for(pending, account, &evm_env),
            );

            // Extract inner CacheDB to match the expected types for the target tx execution
            let cache_db = cache_db.0;

            let target_tx = block.body.transactions[index].clone();
            let target_tx = PendingTransaction::from_maybe_impersonated(target_tx)?;
            let (result, base_tx_env) = self.transact_envelope_with_inspector_ref(
                &cache_db,
                &evm_env,
                &mut inspector,
                target_tx.transaction.as_ref(),
                *target_tx.sender(),
            )?;

            Ok(f(result, cache_db, inspector, base_tx_env, evm_env))
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
            |result, cache_db, mut inspector, tx_env, evm_env| {
                inspector
                    .json_result(
                        result,
                        &alloy_evm::IntoTxEnv::into_tx_env(tx_env),
                        &evm_env.block_env,
                        &cache_db,
                    )
                    .map_err(|e| BlockchainError::Message(e.to_string()))
            },
        )??;
        Ok(GethTrace::JS(trace))
    }

    /// Prove an account's existence or nonexistence in the state trie.
    ///
    /// Returns a merkle proof of the account's trie node, `account_key` == keccak(address)
    pub async fn prove_account_at(
        &self,
        address: Address,
        keys: Vec<B256>,
        block_request: Option<BlockRequest<FoundryTxEnvelope>>,
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
            let (storage_hash, storage_proofs) = prove_storage(&account.storage, &keys);

            let account_proof = AccountProof {
                address,
                balance: account.info.balance,
                nonce: account.info.nonce,
                code_hash: account.info.code_hash,
                storage_hash,
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
}

impl<N: Network> Backend<N>
where
    N: Network<TxEnvelope = FoundryTxEnvelope, ReceiptEnvelope = FoundryReceiptEnvelope>,
{
    /// Rollback the chain to a common height.
    ///
    /// The state of the chain is rewound using `rewind` to the common block, including the db,
    /// storage, and env.
    pub async fn rollback(&self, common_block: Block) -> Result<(), BlockchainError> {
        let hash = common_block.header.hash_slow();

        // Get the database at the common block
        let common_state = {
            let return_state_or_throw_err =
                |db: Option<&StateDb>| -> Result<AddressMap<DbAccount>, BlockchainError> {
                    let state_db = db.ok_or(BlockchainError::DataUnavailable)?;
                    let db_full =
                        state_db.maybe_as_full_db().ok_or(BlockchainError::DataUnavailable)?;
                    Ok(db_full.clone())
                };

            let read_guard = self.states.upgradable_read();
            if let Some(db) = read_guard.get_state(&hash) {
                return_state_or_throw_err(Some(db))?
            } else {
                let mut write_guard = RwLockUpgradableReadGuard::upgrade(read_guard);
                return_state_or_throw_err(write_guard.get_on_disk_state(&hash))?
            }
        };

        {
            // Unwind the storage back to the common ancestor first
            let removed_blocks =
                self.blockchain.storage.write().unwind_to(common_block.header.number(), hash);

            // Clean up in-memory and on-disk states for removed blocks
            let removed_hashes: Vec<_> =
                removed_blocks.iter().map(|b| b.header.hash_slow()).collect();
            self.states.write().remove_block_states(&removed_hashes);

            // Set environment back to common block
            let mut env = self.evm_env.write();
            env.block_env.number = U256::from(common_block.header.number());
            env.block_env.timestamp = U256::from(common_block.header.timestamp());
            env.block_env.gas_limit = common_block.header.gas_limit();
            env.block_env.difficulty = common_block.header.difficulty();
            env.block_env.prevrandao = common_block.header.mix_hash();

            self.time.reset(env.block_env.timestamp.saturating_to());
        }

        {
            // Collect block hashes before acquiring db lock to avoid holding blockchain storage
            // lock across await. Only collect the last 256 blocks since that's all BLOCKHASH can
            // access.
            let block_hashes: Vec<_> = {
                let storage = self.blockchain.storage.read();
                let min_block = common_block.header.number().saturating_sub(256);
                storage
                    .hashes
                    .iter()
                    .filter(|(num, _)| **num >= min_block)
                    .map(|(&num, &hash)| (num, hash))
                    .collect()
            };

            // Acquire db lock once for the entire restore operation to reduce lock churn.
            let mut db = self.db.write().await;
            db.clear();

            // Insert account info before storage to prevent fork-mode RPC fetches after clear.
            for (address, acc) in common_state {
                db.insert_account(address, acc.info);
                for (key, value) in acc.storage {
                    db.set_storage_at(address, key.into(), value.into())?;
                }
            }

            // Restore block hashes from blockchain storage (now unwound, contains only valid
            // blocks).
            for (block_num, hash) in block_hashes {
                db.insert_block_hash(U256::from(block_num), hash);
            }
        }

        Ok(())
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

    fn geth_trace(
        &self,
        tx: &MinedTransaction<N>,
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
                    | GethDebugBuiltInTracerType::Erc7562Tracer
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

    /// returns all receipts for the given transactions
    fn get_receipts(
        &self,
        tx_hashes: impl IntoIterator<Item = TxHash>,
    ) -> Vec<FoundryReceiptEnvelope> {
        let storage = self.blockchain.storage.read();
        let mut receipts = vec![];

        for hash in tx_hashes {
            if let Some(tx) = storage.transactions.get(&hash) {
                receipts.push(tx.receipt.clone());
            }
        }

        receipts
    }

    pub async fn transaction_receipt(
        &self,
        hash: B256,
    ) -> Result<Option<FoundryTxReceipt>, BlockchainError> {
        if let Some(receipt) = self.mined_transaction_receipt(hash) {
            return Ok(Some(receipt.inner));
        }

        if let Some(fork) = self.get_fork() {
            let receipt = fork.transaction_receipt(hash).await?;
            let number = self.convert_block_number(
                receipt.clone().and_then(|r| r.block_number()).map(BlockNumber::from),
            );

            if fork.predates_fork_inclusive(number) {
                return Ok(receipt);
            }
        }

        Ok(None)
    }

    /// Returns all transaction receipts of the block
    pub fn mined_block_receipts(&self, id: impl Into<BlockId>) -> Option<Vec<FoundryTxReceipt>> {
        let mut receipts = Vec::new();
        let block = self.get_block(id)?;

        for transaction in block.body.transactions {
            let receipt = self.mined_transaction_receipt(transaction.hash())?;
            receipts.push(receipt.inner);
        }

        Some(receipts)
    }

    /// Returns the transaction receipt for the given hash
    pub(crate) fn mined_transaction_receipt(
        &self,
        hash: B256,
    ) -> Option<MinedTransactionReceipt<FoundryNetwork>> {
        let MinedTransaction { info, receipt: tx_receipt, block_hash, .. } =
            self.blockchain.get_transaction_by_hash(&hash)?;

        let index = info.transaction_index as usize;
        let block = self.blockchain.get_block_by_hash(&block_hash)?;
        let transaction = block.body.transactions[index].clone();

        // Cancun specific
        let excess_blob_gas = block.header.excess_blob_gas();
        let blob_gas_price =
            alloy_eips::eip4844::calc_blob_gasprice(excess_blob_gas.unwrap_or_default());
        let blob_gas_used = transaction.blob_gas_used();

        let effective_gas_price = transaction.effective_gas_price(block.header.base_fee_per_gas());

        let receipts = self.get_receipts(block.body.transactions.iter().map(|tx| tx.hash()));
        let next_log_index = receipts[..index].iter().map(|r| r.logs().len()).sum::<usize>();

        let tx_receipt = tx_receipt.convert_logs_rpc(
            BlockNumHash::new(block.header.number(), block_hash),
            block.header.timestamp(),
            info.transaction_hash,
            info.transaction_index,
            next_log_index,
        );

        let receipt = TransactionReceipt {
            inner: tx_receipt,
            transaction_hash: info.transaction_hash,
            transaction_index: Some(info.transaction_index),
            block_number: Some(block.header.number()),
            gas_used: info.gas_used,
            contract_address: info.contract_address,
            effective_gas_price,
            block_hash: Some(block_hash),
            from: info.from,
            to: info.to,
            blob_gas_price: Some(blob_gas_price),
            blob_gas_used,
        };

        // Include timestamp in receipt to avoid extra block lookups (e.g., in Otterscan API)
        let mut inner = FoundryTxReceipt::with_timestamp(receipt, block.header.timestamp());
        if self.is_tempo() {
            inner = inner.with_fee_payer(info.from);
        }
        Some(MinedTransactionReceipt { inner, out: info.out })
    }

    /// Returns the blocks receipts for the given number
    pub async fn block_receipts(
        &self,
        number: BlockId,
    ) -> Result<Option<Vec<FoundryTxReceipt>>, BlockchainError> {
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
}

impl<N: Network<ReceiptEnvelope = FoundryReceiptEnvelope>> Backend<N> {
    /// Get the current state.
    pub async fn serialized_state(
        &self,
        preserve_historical_states: bool,
    ) -> Result<SerializableState, BlockchainError> {
        let at = self.evm_env.read().block_env.clone();
        let best_number = self.blockchain.storage.read().best_number;
        let blocks = self.blockchain.storage.read().serialized_blocks();
        let transactions = self.blockchain.storage.read().serialized_transactions();
        let historical_states =
            preserve_historical_states.then(|| self.states.write().serialized_states());

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
            self.evm_env.write().block_env = block.clone();

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

        if let Some(latest) = state.blocks.iter().max_by_key(|b| b.header.number()) {
            let header = &latest.header;
            let next_block_base_fee = self.fees.get_next_block_base_fee_per_gas(
                header.gas_used(),
                header.gas_limit(),
                header.base_fee_per_gas().unwrap_or_default(),
            );
            let next_block_excess_blob_gas = self.fees.get_next_block_blob_excess_gas(
                header.excess_blob_gas().unwrap_or_default(),
                header.blob_gas_used().unwrap_or_default(),
            );

            // update next base fee
            self.fees.set_base_fee(next_block_base_fee);

            self.fees.set_blob_excess_gas_and_price(BlobExcessGasAndPrice::new(
                next_block_excess_blob_gas,
                get_blob_base_fee_update_fraction(
                    self.evm_env.read().cfg_env.chain_id,
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
}

impl Backend<FoundryNetwork> {
    /// Simulates the payload by executing the calls in request.
    pub async fn simulate(
        &self,
        request: SimulatePayload,
        block_request: Option<BlockRequest<FoundryTxEnvelope>>,
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

                    let (mut evm_env, tx_env) = self.build_call_env(
                        WithOtherFields::new(request.clone()),
                        fee_details,
                        block_env.clone(),
                    );

                    // Always disable EIP-3607
                    evm_env.cfg_env.disable_eip3607 = true;

                    if !validation {
                        evm_env.cfg_env.disable_base_fee = !validation;
                        evm_env.block_env.basefee = 0;
                    }

                    let mut inspector = self.build_inspector();

                    // transact
                    if trace_transfers {
                        inspector = inspector.with_transfers();
                    }
                    trace!(target: "backend", env=?evm_env, spec=?evm_env.spec_id(),"simulate evm env");
                    let ResultAndState { result, state } =
                        self.transact_with_inspector_ref(&cache_db, &evm_env, &mut inspector, tx_env)?;
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

                    let mut request = Into::<FoundryTransactionRequest>::into(WithOtherFields::new(request));
                    request.prep_for_submission();

                    let typed_tx = request.build_unsigned().map_err(|e| BlockchainError::InvalidTransactionRequest(e.to_string()))?;

                    let tx = build_impersonated(typed_tx);
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
                                data: None,
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
                    logs.extend(sim_res.logs.iter().map(|log| log.inner.clone()));
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
                    .next_block_base_fee(self.fees.base_fee_params())
                    .unwrap_or_default();

                block_res.push(simulated_block);
            }

            Ok(block_res)
        })
        .await?
    }

    pub fn get_blob_by_tx_hash(&self, hash: B256) -> Result<Option<Vec<alloy_consensus::Blob>>> {
        // Try to get the mined transaction by hash
        if let Some(tx) = self.mined_transaction_by_hash(hash)
            && let Ok(typed_tx) = FoundryTxEnvelope::try_from(tx)
            && let Some(sidecar) = typed_tx.sidecar()
        {
            return Ok(Some(sidecar.sidecar.blobs().to_vec()));
        }

        Ok(None)
    }
}

/// Get max nonce from transaction pool by address.
fn get_pool_transactions_nonce(
    pool_transactions: &[Arc<PoolTransaction<FoundryTxEnvelope>>],
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
impl<N: Network> TransactionValidator<FoundryTxEnvelope> for Backend<N>
where
    N: Network<TxEnvelope = FoundryTxEnvelope, ReceiptEnvelope = FoundryReceiptEnvelope>,
{
    async fn validate_pool_transaction(
        &self,
        tx: &PendingTransaction<FoundryTxEnvelope>,
    ) -> Result<(), BlockchainError> {
        let address = *tx.sender();
        let account = self.get_account(address).await?;
        let evm_env = self.next_evm_env();

        // Tempo AA: validate time bounds and fee token balance (async checks)
        if let FoundryTxEnvelope::Tempo(aa_tx) = tx.transaction.as_ref() {
            let tempo_tx = aa_tx.tx();
            let current_time = evm_env.block_env.timestamp.saturating_to::<u64>();

            // Reject if valid_before is expired or too close to current time (< 3 seconds)
            const AA_VALID_BEFORE_MIN_SECS: u64 = 3;
            if let Some(valid_before) = tempo_tx.valid_before {
                let min_allowed = current_time.saturating_add(AA_VALID_BEFORE_MIN_SECS);
                if valid_before <= min_allowed {
                    return Err(InvalidTransactionError::TempoValidBeforeExpired {
                        valid_before,
                        min_allowed,
                    }
                    .into());
                }
            }

            // Reject if valid_after is too far in the future (> 1 hour)
            const AA_VALID_AFTER_MAX_SECS: u64 = 3600;
            if let Some(valid_after) = tempo_tx.valid_after {
                let max_allowed = current_time.saturating_add(AA_VALID_AFTER_MAX_SECS);
                if valid_after > max_allowed {
                    return Err(InvalidTransactionError::TempoValidAfterTooFar {
                        valid_after,
                        max_allowed,
                    }
                    .into());
                }
            }

            // Fee token balance check
            let fee_payer = tempo_tx.recover_fee_payer(address).unwrap_or(address);
            let fee_token =
                tempo_tx.fee_token.unwrap_or(foundry_evm::core::tempo::PATH_USD_ADDRESS);

            // gas_limit * max_fee_per_gas in wei, scaled to 6-decimal token units
            let required_wei =
                U256::from(tempo_tx.gas_limit).saturating_mul(U256::from(tempo_tx.max_fee_per_gas));
            let required = required_wei / U256::from(10u64.pow(12));

            let balance = self.get_fee_token_balance(fee_token, fee_payer).await?;
            if balance < required {
                return Err(InvalidTransactionError::TempoInsufficientFeeTokenBalance {
                    balance,
                    required,
                }
                .into());
            }
        }

        Ok(self.validate_pool_transaction_for(tx, &account, &evm_env)?)
    }

    fn validate_pool_transaction_for(
        &self,
        pending: &PendingTransaction<FoundryTxEnvelope>,
        account: &AccountInfo,
        evm_env: &EvmEnv,
    ) -> Result<(), InvalidTransactionError> {
        let tx = &pending.transaction;

        if let Some(tx_chain_id) = tx.chain_id() {
            let chain_id = self.chain_id();
            if chain_id.to::<u64>() != tx_chain_id {
                if let FoundryTxEnvelope::Legacy(tx) = tx.as_ref() {
                    // <https://github.com/ethereum/EIPs/blob/master/EIPS/eip-155.md>
                    if evm_env.cfg_env.spec >= SpecId::SPURIOUS_DRAGON && tx.chain_id().is_none() {
                        debug!(target: "backend", ?chain_id, ?tx_chain_id, "incompatible EIP155-based V");
                        return Err(InvalidTransactionError::IncompatibleEIP155);
                    }
                } else {
                    debug!(target: "backend", ?chain_id, ?tx_chain_id, "invalid chain id");
                    return Err(InvalidTransactionError::InvalidChainId);
                }
            }
        }

        // Reject native value transfers on Tempo networks
        if self.is_tempo() && !tx.value().is_zero() {
            warn!(target: "backend", "[{:?}] native value transfer not allowed in Tempo mode", tx.hash());
            return Err(InvalidTransactionError::TempoNativeValueTransfer);
        }

        // Tempo AA: cap authorization list size
        if let FoundryTxEnvelope::Tempo(aa_tx) = tx.as_ref() {
            const MAX_TEMPO_AUTHORIZATIONS: usize = 16;
            let auth_count = aa_tx.tx().tempo_authorization_list.len();
            if auth_count > MAX_TEMPO_AUTHORIZATIONS {
                warn!(target: "backend", "[{:?}] Tempo tx has too many authorizations: {}", tx.hash(), auth_count);
                return Err(InvalidTransactionError::TempoTooManyAuthorizations {
                    count: auth_count,
                    max: MAX_TEMPO_AUTHORIZATIONS,
                });
            }
        }

        // Nonce validation — skip for deposits (L1→L2) and Tempo txs (2D nonce system)
        let is_deposit_tx = pending.transaction.as_ref().is_deposit();
        let is_tempo_tx = pending.transaction.as_ref().is_tempo();
        let nonce = tx.nonce();
        if nonce < account.nonce && !is_deposit_tx && !is_tempo_tx {
            debug!(target: "backend", "[{:?}] nonce too low", tx.hash());
            return Err(InvalidTransactionError::NonceTooLow);
        }

        // EIP-4844 structural validation
        if evm_env.cfg_env.spec >= SpecId::CANCUN && tx.is_eip4844() {
            // Heavy (blob validation) checks
            let blob_tx = match tx.as_ref() {
                FoundryTxEnvelope::Eip4844(tx) => tx.tx(),
                _ => unreachable!(),
            };

            let blob_count = blob_tx.tx().blob_versioned_hashes.len();

            // Ensure there are blob hashes.
            if blob_count == 0 {
                return Err(InvalidTransactionError::NoBlobHashes);
            }

            // Ensure the tx does not exceed the max blobs per transaction.
            let max_blobs_per_tx = self.blob_params().max_blobs_per_tx as usize;
            if blob_count > max_blobs_per_tx {
                return Err(InvalidTransactionError::TooManyBlobs(blob_count, max_blobs_per_tx));
            }

            // Check for any blob validation errors if not impersonating.
            if !self.skip_blob_validation(Some(*pending.sender()))
                && let Err(err) = blob_tx.validate(EnvKzgSettings::default().get())
            {
                return Err(InvalidTransactionError::BlobTransactionValidationError(err));
            }
        }

        // EIP-3860 initcode size validation, respects --code-size-limit / --disable-code-size-limit
        if evm_env.cfg_env.spec >= SpecId::SHANGHAI && tx.kind() == TxKind::Create {
            let max_initcode_size = evm_env
                .cfg_env
                .limit_contract_code_size
                .map(|limit| limit.saturating_mul(2))
                .unwrap_or(revm::primitives::eip3860::MAX_INITCODE_SIZE);
            if tx.input().len() > max_initcode_size {
                return Err(InvalidTransactionError::MaxInitCodeSizeExceeded);
            }
        }

        // Balance and fee related checks
        if !self.disable_pool_balance_checks {
            // Gas limit validation
            if tx.gas_limit() < MIN_TRANSACTION_GAS as u64 {
                debug!(target: "backend", "[{:?}] gas too low", tx.hash());
                return Err(InvalidTransactionError::GasTooLow);
            }

            // Check tx gas limit against block gas limit, if block gas limit is set.
            if !evm_env.cfg_env.disable_block_gas_limit
                && tx.gas_limit() > evm_env.block_env.gas_limit
            {
                debug!(target: "backend", "[{:?}] gas too high", tx.hash());
                return Err(InvalidTransactionError::GasTooHigh(ErrDetail {
                    detail: String::from("tx.gas_limit > env.block.gas_limit"),
                }));
            }

            // Check tx gas limit against tx gas limit cap (Osaka hard fork and later).
            if evm_env.cfg_env.tx_gas_limit_cap.is_none()
                && tx.gas_limit() > evm_env.cfg_env().tx_gas_limit_cap()
            {
                debug!(target: "backend", "[{:?}] gas too high", tx.hash());
                return Err(InvalidTransactionError::GasTooHigh(ErrDetail {
                    detail: String::from("tx.gas_limit > env.cfg.tx_gas_limit_cap"),
                }));
            }

            // EIP-1559 fee validation (London hard fork and later).
            if evm_env.cfg_env.spec >= SpecId::LONDON {
                if tx.max_fee_per_gas() < evm_env.block_env.basefee.into() && !is_deposit_tx {
                    debug!(target: "backend", "max fee per gas={}, too low, block basefee={}", tx.max_fee_per_gas(), evm_env.block_env.basefee);
                    return Err(InvalidTransactionError::FeeCapTooLow);
                }

                if let (Some(max_priority_fee_per_gas), max_fee_per_gas) =
                    (tx.as_ref().max_priority_fee_per_gas(), tx.as_ref().max_fee_per_gas())
                    && max_priority_fee_per_gas > max_fee_per_gas
                {
                    debug!(target: "backend", "max priority fee per gas={}, too high, max fee per gas={}", max_priority_fee_per_gas, max_fee_per_gas);
                    return Err(InvalidTransactionError::TipAboveFeeCap);
                }
            }

            // EIP-4844 blob fee validation
            if evm_env.cfg_env.spec >= SpecId::CANCUN
                && tx.is_eip4844()
                && let Some(max_fee_per_blob_gas) = tx.max_fee_per_blob_gas()
                && let Some(blob_gas_and_price) = &evm_env.block_env.blob_excess_gas_and_price
                && max_fee_per_blob_gas < blob_gas_and_price.blob_gasprice
            {
                debug!(target: "backend", "max fee per blob gas={}, too low, block blob gas price={}", max_fee_per_blob_gas, blob_gas_and_price.blob_gasprice);
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
                        debug!(target: "backend", "[{:?}] insufficient balance={}, required={} account={:?}", tx.hash(), account.balance + U256::from(deposit_tx.mint), value, *pending.sender());
                        return Err(InvalidTransactionError::InsufficientFunds);
                    }
                }
                FoundryTxEnvelope::Tempo(_) => {
                    // Tempo AA transactions pay gas with fee tokens, not ETH.
                    // Fee token balance is validated in validate_pool_transaction (async).
                }
                _ => {
                    // check sufficient funds: `gas * price + value`
                    let req_funds =
                        max_cost.checked_add(value.saturating_to()).ok_or_else(|| {
                            debug!(target: "backend", "[{:?}] cost too high", tx.hash());
                            InvalidTransactionError::InsufficientFunds
                        })?;
                    if account.balance < U256::from(req_funds) {
                        debug!(target: "backend", "[{:?}] insufficient balance={}, required={} account={:?}", tx.hash(), account.balance, req_funds, *pending.sender());
                        return Err(InvalidTransactionError::InsufficientFunds);
                    }
                }
            }
        }
        Ok(())
    }

    fn validate_for(
        &self,
        tx: &PendingTransaction<FoundryTxEnvelope>,
        account: &AccountInfo,
        evm_env: &EvmEnv,
    ) -> Result<(), InvalidTransactionError> {
        self.validate_pool_transaction_for(tx, account, evm_env)?;
        if tx.nonce() > account.nonce {
            return Err(InvalidTransactionError::NonceTooHigh);
        }
        Ok(())
    }
}

/// Replaces the cached hash of a [`Signed`] transaction, preserving the inner tx and signature.
fn rehash<T>(signed: Signed<T>, hash: B256) -> Signed<T>
where
    T: alloy_consensus::transaction::RlpEcdsaEncodableTx,
{
    let (t, sig, _) = signed.into_parts();
    Signed::new_unchecked(t, sig, hash)
}

/// Creates a `AnyRpcTransaction` as it's expected for the `eth` RPC api from storage data
pub fn transaction_build(
    tx_hash: Option<B256>,
    eth_transaction: MaybeImpersonatedTransaction<FoundryTxEnvelope>,
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
                    block_number: block.as_ref().map(|block| block.header.number()),
                    transaction_index: info.as_ref().map(|info| info.transaction_index),
                    effective_gas_price: None,
                    block_timestamp: block.as_ref().map(|block| block.header.timestamp()),
                };

                return AnyRpcTransaction::from(WithOtherFields::new(tx));
            }
            Err(_) => {
                error!(target: "backend", "failed to serialize deposit transaction");
            }
        }
    }

    if let FoundryTxEnvelope::Tempo(tempo_tx) = eth_transaction.as_ref() {
        let from = eth_transaction.recover().unwrap_or_default();
        let ser = serde_json::to_value(tempo_tx).expect("could not serialize Tempo transaction");
        let maybe_tempo_fields = OtherFields::try_from(ser);

        match maybe_tempo_fields {
            Ok(fields) => {
                let inner = UnknownTypedTransaction {
                    ty: AnyTxType(TEMPO_TX_TYPE_ID),
                    fields,
                    memo: Default::default(),
                };

                let envelope = AnyTxEnvelope::Unknown(UnknownTxEnvelope {
                    hash: eth_transaction.hash(),
                    inner,
                });

                let tx = Transaction {
                    inner: Recovered::new_unchecked(envelope, from),
                    block_hash: block.as_ref().map(|block| block.header.hash_slow()),
                    block_number: block.as_ref().map(|block| block.header.number()),
                    transaction_index: info.as_ref().map(|info| info.transaction_index),
                    effective_gas_price: None,
                    block_timestamp: block.as_ref().map(|block| block.header.timestamp()),
                };

                return AnyRpcTransaction::from(WithOtherFields::new(tx));
            }
            Err(_) => {
                error!(target: "backend", "failed to serialize tempo transaction");
            }
        }
    }

    let from = eth_transaction.recover().unwrap_or_default();
    let effective_gas_price = eth_transaction.effective_gas_price(base_fee);

    // if a specific hash was provided we update the transaction's hash
    // This is important for impersonated transactions since they all use the
    // `BYPASS_SIGNATURE` which would result in different hashes
    // Note: for impersonated transactions this only concerns pending transactions because
    // there's no `info` yet.
    let hash = tx_hash.unwrap_or_else(|| eth_transaction.hash());

    let eth_envelope = FoundryTxEnvelope::from(eth_transaction)
        .try_into_eth()
        .expect("non-standard transactions are handled above");

    let envelope = match eth_envelope {
        TxEnvelope::Legacy(s) => AnyTxEnvelope::Ethereum(TxEnvelope::Legacy(rehash(s, hash))),
        TxEnvelope::Eip1559(s) => AnyTxEnvelope::Ethereum(TxEnvelope::Eip1559(rehash(s, hash))),
        TxEnvelope::Eip2930(s) => AnyTxEnvelope::Ethereum(TxEnvelope::Eip2930(rehash(s, hash))),
        TxEnvelope::Eip4844(s) => AnyTxEnvelope::Ethereum(TxEnvelope::Eip4844(rehash(s, hash))),
        TxEnvelope::Eip7702(s) => AnyTxEnvelope::Ethereum(TxEnvelope::Eip7702(rehash(s, hash))),
    };

    let tx = Transaction {
        inner: Recovered::new_unchecked(envelope, from),
        block_hash: block.as_ref().map(|block| block.header.hash_slow()),
        block_number: block.as_ref().map(|block| block.header.number()),
        transaction_index: info.as_ref().map(|info| info.transaction_index),
        // deprecated
        effective_gas_price: Some(effective_gas_price),
        block_timestamp: block.as_ref().map(|block| block.header.timestamp()),
    };
    AnyRpcTransaction::from(WithOtherFields::new(tx))
}

/// Prove a storage key's existence or nonexistence in the account's storage trie.
///
/// `storage_key` is the hash of the desired storage key, meaning
/// this will only work correctly under a secure trie.
/// `storage_key` == keccak(key)
pub fn prove_storage(
    storage: &alloy_primitives::map::U256Map<U256>,
    keys: &[B256],
) -> (B256, Vec<Vec<Bytes>>) {
    let keys: Vec<_> = keys.iter().map(|key| Nibbles::unpack(keccak256(key))).collect();

    let mut builder = HashBuilder::default().with_proof_retainer(ProofRetainer::new(keys.clone()));

    for (key, value) in trie_storage(storage) {
        builder.add_leaf(key, &value);
    }

    let root = builder.root();

    let mut proofs = Vec::new();
    let all_proof_nodes = builder.take_proof_nodes();

    for proof_key in keys {
        // Iterate over all proof nodes and find the matching ones.
        // The filtered results are guaranteed to be in order.
        let matching_proof_nodes =
            all_proof_nodes.matching_nodes_sorted(&proof_key).into_iter().map(|(_, node)| node);
        proofs.push(matching_proof_nodes.collect());
    }

    (root, proofs)
}

pub fn is_arbitrum(chain_id: u64) -> bool {
    if let Ok(chain) = NamedChain::try_from(chain_id) {
        return chain.is_arbitrum();
    }
    false
}

/// Unpacks an [`ExecutionResult`] into its exit reason, gas used, output, and logs.
fn unpack_execution_result<H: IntoInstructionResult>(
    result: ExecutionResult<H>,
) -> (InstructionResult, u64, Option<Output>, Vec<revm::primitives::Log>) {
    match result {
        ExecutionResult::Success { reason, gas, output, logs, .. } => {
            (reason.into(), gas.used(), Some(output), logs)
        }
        ExecutionResult::Revert { gas, output, logs, .. } => {
            (InstructionResult::Revert, gas.used(), Some(Output::Call(output)), logs)
        }
        ExecutionResult::Halt { reason, gas, logs, .. } => {
            (reason.into_instruction_result(), gas.used(), None, logs)
        }
    }
}

/// Converts a halt reason into an [`InstructionResult`].
///
/// Abstracts over network-specific halt reason types (`HaltReason`, `OpHaltReason`)
/// so that anvil code doesn't need to match on each variant directly.
pub use foundry_evm::core::evm::IntoInstructionResult;

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
