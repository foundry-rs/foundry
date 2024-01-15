//! In memory blockchain backend
use crate::{
    config::PruneStateHistoryConfig,
    eth::{
        backend::{
            cheats::CheatsManager,
            db::{AsHashDB, Db, MaybeHashDatabase, SerializableState},
            executor::{ExecutedTransactions, TransactionExecutor},
            fork::ClientFork,
            genesis::GenesisConfig,
            mem::storage::MinedTransactionReceipt,
            notifications::{NewBlockNotification, NewBlockNotifications},
            time::{utc_from_secs, TimeManager},
            validate::TransactionValidator,
        },
        error::{BlockchainError, ErrDetail, InvalidTransactionError},
        fees::{FeeDetails, FeeManager},
        macros::node_info,
        pool::transactions::PoolTransaction,
        util::get_precompiles_for,
    },
    mem::{
        inspector::Inspector,
        storage::{BlockchainStorage, InMemoryBlockStates, MinedBlockOutcome},
    },
    revm::{
        db::DatabaseRef,
        primitives::{AccountInfo, U256 as rU256},
    },
    NodeConfig,
};
use alloy_consensus::{Header, Receipt, ReceiptWithBloom};
use alloy_network::Sealable;
use alloy_primitives::{Address, Bloom, Bytes, TxHash, B256, B64, U128, U256, U64, U8};
use alloy_rpc_trace_types::{
    geth::{DefaultFrame, GethDebugTracingOptions, GethDefaultTracingOptions, GethTrace},
    parity::LocalizedTransactionTrace,
};
use alloy_rpc_types::{
    state::StateOverride, AccessList, Block as AlloyBlock, BlockId,
    BlockNumberOrTag as BlockNumber, CallRequest, Filter, FilteredParams, Header as AlloyHeader,
    Log, Transaction, TransactionReceipt,
};
use anvil_core::{
    eth::{
        alloy_block::{Block, BlockInfo},
        proof::{AccountProof, BasicAccount, StorageProof},
        transaction::alloy::{
            MaybeImpersonatedTransaction, PendingTransaction, TransactionInfo, TypedReceipt,
            TypedTransaction,
        },
        trie::RefTrieDB,
        utils::alloy_to_revm_access_list,
    },
    types::{Forking, Index},
};
use anvil_rpc::error::RpcError;
use ethers::{
    abi::ethereum_types::BigEndianHash,
    utils::{keccak256, rlp},
};
use flate2::{read::GzDecoder, write::GzEncoder, Compression};
use foundry_common::types::{ToAlloy, ToEthers};
use foundry_evm::{
    backend::{DatabaseError, DatabaseResult, RevertSnapshotAction},
    constants::DEFAULT_CREATE2_DEPLOYER_RUNTIME_CODE,
    decode::decode_revert,
    inspectors::AccessListTracer,
    revm::{
        self,
        db::CacheDB,
        interpreter::InstructionResult,
        primitives::{
            BlockEnv, CreateScheme, EVMError, Env, ExecutionResult, InvalidHeader, Output, SpecId,
            TransactTo, TxEnv, KECCAK_EMPTY,
        },
    },
    traces::{TracingInspector, TracingInspectorConfig},
    utils::{eval_to_instruction_result, halt_to_instruction_result, u256_to_h256_be},
};
use futures::channel::mpsc::{unbounded, UnboundedSender};
use hash_db::HashDB;
use itertools::Itertools;
use parking_lot::{Mutex, RwLock};
use std::{
    collections::{BTreeMap, HashMap},
    io::{Read, Write},
    ops::Deref,
    sync::Arc,
    time::Duration,
};
use storage::{Blockchain, MinedTransaction};
use tokio::sync::RwLock as AsyncRwLock;
use trie_db::{Recorder, Trie};

pub mod cache;
pub mod fork_db;
pub mod in_memory_db;
pub mod inspector;
pub mod state;
pub mod storage;

// Gas per transaction not creating a contract.
pub const MIN_TRANSACTION_GAS: U256 = U256::from_limbs([21_000, 0, 0, 0]);
// Gas per transaction creating a contract.
pub const MIN_CREATE_GAS: U256 = U256::from_limbs([53_000, 0, 0, 0]);

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
            BlockRequest::Pending(_) => BlockNumber::Pending,
            BlockRequest::Number(n) => BlockNumber::Number(n),
        }
    }
}

/// Gives access to the [revm::Database]
#[derive(Clone)]
pub struct Backend {
    /// Access to [`revm::Database`] abstraction.
    ///
    /// This will be used in combination with [`revm::Evm`] and is responsible for feeding data to
    /// the evm during its execution.
    ///
    /// At time of writing, there are two different types of `Db`:
    ///   - [`MemDb`](crate::mem::MemDb): everything is stored in memory
    ///   - [`ForkDb`](crate::mem::fork_db::ForkedDatabase): forks off a remote client, missing
    ///     data is retrieved via RPC-calls
    ///
    /// In order to commit changes to the [`revm::Database`], the [`revm::Evm`] requires mutable
    /// access, which requires a write-lock from this `db`. In forking mode, the time during
    /// which the write-lock is active depends on whether the `ForkDb` can provide all requested
    /// data from memory or whether it has to retrieve it via RPC calls first. This means that it
    /// potentially blocks for some time, even taking into account the rate limits of RPC
    /// endpoints. Therefor the `Db` is guarded by a `tokio::sync::RwLock` here so calls that
    /// need to read from it, while it's currently written to, don't block. E.g. a new block is
    /// currently mined and a new [`Self::set_storage()`] request is being executed.
    db: Arc<AsyncRwLock<Box<dyn Db>>>,
    /// stores all block related data in memory
    blockchain: Blockchain,
    /// Historic states of previous blocks
    states: Arc<RwLock<InMemoryBlockStates>>,
    /// env data of the chain
    env: Arc<RwLock<Env>>,
    /// this is set if this is currently forked off another client
    fork: Arc<RwLock<Option<ClientFork>>>,
    /// provides time related info, like timestamp
    time: TimeManager,
    /// Contains state of custom overrides
    cheats: CheatsManager,
    /// contains fee data
    fees: FeeManager,
    /// initialised genesis
    genesis: GenesisConfig,
    /// listeners for new blocks that get notified when a new block was imported
    new_block_listeners: Arc<Mutex<Vec<UnboundedSender<NewBlockNotification>>>>,
    /// keeps track of active snapshots at a specific block
    active_snapshots: Arc<Mutex<HashMap<U256, (u64, B256)>>>,
    enable_steps_tracing: bool,
    /// How to keep history state
    prune_state_history_config: PruneStateHistoryConfig,
    /// max number of blocks with transactions in memory
    transaction_block_keeper: Option<usize>,
    node_config: Arc<AsyncRwLock<NodeConfig>>,
}

impl Backend {
    /// Initialises the balance of the given accounts
    #[allow(clippy::too_many_arguments)]
    pub async fn with_genesis(
        db: Arc<AsyncRwLock<Box<dyn Db>>>,
        env: Arc<RwLock<Env>>,
        genesis: GenesisConfig,
        fees: FeeManager,
        fork: Arc<RwLock<Option<ClientFork>>>,
        enable_steps_tracing: bool,
        prune_state_history_config: PruneStateHistoryConfig,
        transaction_block_keeper: Option<usize>,
        automine_block_time: Option<Duration>,
        node_config: Arc<AsyncRwLock<NodeConfig>>,
    ) -> Self {
        // if this is a fork then adjust the blockchain storage
        let blockchain = if let Some(fork) = fork.read().as_ref() {
            trace!(target: "backend", "using forked blockchain at {}", fork.block_number());
            Blockchain::forked(fork.block_number(), fork.block_hash(), fork.total_difficulty())
        } else {
            Blockchain::new(
                &env.read(),
                fees.is_eip1559().then(|| fees.base_fee()),
                genesis.timestamp,
            )
        };

        let start_timestamp = if let Some(fork) = fork.read().as_ref() {
            fork.timestamp()
        } else {
            genesis.timestamp
        };

        let states = if prune_state_history_config.is_config_enabled() {
            // if prune state history is enabled, configure the state cache only for memory
            prune_state_history_config
                .max_memory_history
                .map(InMemoryBlockStates::new)
                .unwrap_or_default()
                .memory_only()
        } else {
            Default::default()
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
            active_snapshots: Arc::new(Mutex::new(Default::default())),
            enable_steps_tracing,
            prune_state_history_config,
            transaction_block_keeper,
            node_config,
        };

        if let Some(interval_block_time) = automine_block_time {
            backend.update_interval_mine_block_time(interval_block_time);
        }

        // Note: this can only fail in forking mode, in which case we can't recover
        backend.apply_genesis().await.expect("Failed to create genesis");
        backend
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

    /// Applies the configured genesis settings
    ///
    /// This will fund, create the genesis accounts
    async fn apply_genesis(&self) -> DatabaseResult<()> {
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

            // in fork mode we only set the balance, this way the accountinfo is fetched from the
            // remote client, preserving code and nonce. The reason for that is private keys for dev
            // accounts are commonly known and are used on testnets
            let mut fork_genesis_infos = self.genesis.fork_genesis_account_infos.lock();
            fork_genesis_infos.clear();

            for res in genesis_accounts {
                let (address, mut info) = res.map_err(DatabaseError::display)??;
                info.balance = self.genesis.balance;
                db.insert_account(address, info.clone());

                // store the fetched AccountInfo, so we can cheaply reset in [Self::reset_fork()]
                fork_genesis_infos.push(info);
            }
        } else {
            let mut db = self.db.write().await;
            for (account, info) in self.genesis.account_infos() {
                db.insert_account(account, info);
            }
        }

        let db = self.db.write().await;
        // apply the genesis.json alloc
        self.genesis.apply_genesis_json_alloc(db)?;
        Ok(())
    }

    /// Sets the account to impersonate
    ///
    /// Returns `true` if the account is already impersonated
    pub async fn impersonate(&self, addr: Address) -> DatabaseResult<bool> {
        if self.cheats.impersonated_accounts().contains(&addr) {
            return Ok(true);
        }
        // Ensure EIP-3607 is disabled
        let mut env = self.env.write();
        env.cfg.disable_eip3607 = true;
        Ok(self.cheats.impersonate(addr))
    }

    /// Removes the account that from the impersonated set
    ///
    /// If the impersonated `addr` is a contract then we also reset the code here
    pub async fn stop_impersonating(&self, addr: Address) -> DatabaseResult<()> {
        self.cheats.stop_impersonating(&addr);
        Ok(())
    }

    /// If set to true will make every account impersonated
    pub async fn auto_impersonate_account(&self, enabled: bool) {
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

    pub fn precompiles(&self) -> Vec<Address> {
        get_precompiles_for(self.env.read().cfg.spec_id)
            .into_iter()
            .map(ToAlloy::to_alloy)
            .collect_vec()
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

                    node_config.setup_fork_db_config(eth_rpc_url, &mut env, &self.fees).await
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
                .await
                .map_err(|_| BlockchainError::DataUnavailable)?
                .ok_or(BlockchainError::BlockNotFound)?;
            // update all settings related to the forked block
            {
                let mut env = self.env.write();
                env.cfg.chain_id = fork.chain_id();

                env.block = BlockEnv {
                    number: rU256::from(fork_block_number),
                    timestamp: fork_block.header.timestamp,
                    gas_limit: fork_block.header.gas_limit,
                    difficulty: fork_block.header.difficulty,
                    prevrandao: Some(fork_block.header.mix_hash.unwrap_or_default()),
                    // Keep previous `coinbase` and `basefee` value
                    coinbase: env.block.coinbase,
                    basefee: env.block.basefee,
                    ..env.block.clone()
                };

                self.time.reset(env.block.timestamp.to_ethers().as_u64());

                // this is the base fee of the current block, but we need the base fee of
                // the next block
                let next_block_base_fee = self.fees.get_next_block_base_fee_per_gas(
                    fork_block.header.gas_used,
                    fork_block.header.gas_limit,
                    fork_block.header.base_fee_per_gas.unwrap_or_default(),
                );

                self.fees.set_base_fee(U256::from(next_block_base_fee));

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

            // insert back all genesis accounts, by reusing cached `AccountInfo`s we don't need to
            // fetch the data via RPC again
            let mut db = self.db.write().await;

            // clear database
            db.clear();

            let fork_genesis_infos = self.genesis.fork_genesis_account_infos.lock();
            for (address, info) in
                self.genesis.accounts.iter().copied().zip(fork_genesis_infos.iter().cloned())
            {
                db.insert_account(address, info);
            }

            // reset the genesis.json alloc
            self.genesis.apply_genesis_json_alloc(db)?;

            Ok(())
        } else {
            Err(RpcError::invalid_params("Forking not enabled").into())
        }
    }

    /// Returns the `TimeManager` responsible for timestamps
    pub fn time(&self) -> &TimeManager {
        &self.time
    }

    /// Returns the `CheatsManager` responsible for executing cheatcodes
    pub fn cheats(&self) -> &CheatsManager {
        &self.cheats
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
        self.env.read().block.number.try_into().unwrap_or(u64::MAX)
    }

    /// Sets the block number
    pub fn set_block_number(&self, number: U256) {
        let mut env = self.env.write();
        env.block.number = number;
    }

    /// Returns the client coinbase address.
    pub fn coinbase(&self) -> Address {
        self.env.read().block.coinbase
    }

    /// Returns the client coinbase address.
    pub fn chain_id(&self) -> U256 {
        U256::from(self.env.read().cfg.chain_id)
    }

    pub fn set_chain_id(&self, chain_id: u64) {
        self.env.write().cfg.chain_id = chain_id;
    }

    /// Returns balance of the given account.
    pub async fn current_balance(&self, address: Address) -> DatabaseResult<U256> {
        Ok(self.get_account(address).await?.balance)
    }

    /// Returns balance of the given account.
    pub async fn current_nonce(&self, address: Address) -> DatabaseResult<U256> {
        Ok(U256::from(self.get_account(address).await?.nonce))
    }

    /// Sets the coinbase address
    pub fn set_coinbase(&self, address: Address) {
        self.env.write().block.coinbase = address;
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
        self.db.write().await.set_code(address, code.0.into())
    }

    /// Sets the value for the given slot of the given address
    pub async fn set_storage_at(
        &self,
        address: Address,
        slot: U256,
        val: B256,
    ) -> DatabaseResult<()> {
        self.db.write().await.set_storage_at(address, slot, val.to_ethers().into_uint().to_alloy())
    }

    /// Returns the configured specid
    pub fn spec_id(&self) -> SpecId {
        self.env.read().cfg.spec_id
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

    /// Returns true if op-stack deposits are active
    pub fn is_optimism(&self) -> bool {
        self.env.read().cfg.optimism
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

    /// Returns an error if op-stack deposits are not active
    pub fn ensure_op_deposits_active(&self) -> Result<(), BlockchainError> {
        if self.is_optimism() {
            return Ok(())
        }
        Err(BlockchainError::DepositTransactionUnsupported)
    }

    /// Returns the block gas limit
    pub fn gas_limit(&self) -> U256 {
        self.env.read().block.gas_limit
    }

    /// Sets the block gas limit
    pub fn set_gas_limit(&self, gas_limit: U256) {
        self.env.write().block.gas_limit = gas_limit;
    }

    /// Returns the current base fee
    pub fn base_fee(&self) -> U256 {
        self.fees.base_fee()
    }

    /// Sets the current basefee
    pub fn set_base_fee(&self, basefee: U256) {
        self.fees.set_base_fee(basefee)
    }

    /// Returns the current gas price
    pub fn gas_price(&self) -> U256 {
        self.fees.gas_price()
    }

    /// Returns the suggested fee cap
    pub fn max_priority_fee_per_gas(&self) -> U256 {
        self.fees.max_priority_fee_per_gas()
    }

    /// Sets the gas price
    pub fn set_gas_price(&self, price: U256) {
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

    /// Creates a new `evm_snapshot` at the current height
    ///
    /// Returns the id of the snapshot created
    pub async fn create_snapshot(&self) -> U256 {
        let num = self.best_number();
        let hash = self.best_hash();
        let id = self.db.write().await.snapshot();
        trace!(target: "backend", "creating snapshot {} at {}", id, num);
        self.active_snapshots.lock().insert(id, (num, hash));
        id
    }

    /// Reverts the state to the snapshot identified by the given `id`.
    pub async fn revert_snapshot(&self, id: U256) -> Result<bool, BlockchainError> {
        let block = { self.active_snapshots.lock().remove(&id) };
        if let Some((num, hash)) = block {
            let best_block_hash = {
                // revert the storage that's newer than the snapshot
                let current_height = self.best_number();
                let mut storage = self.blockchain.storage.write();

                for n in ((num + 1)..=current_height).rev() {
                    trace!(target: "backend", "reverting block {}", n);
                    let n = U64::from(n);
                    if let Some(hash) = storage.hashes.remove(&n) {
                        if let Some(block) = storage.blocks.remove(&hash) {
                            for tx in block.transactions {
                                let _ = storage.transactions.remove(&tx.hash());
                            }
                        }
                    }
                }

                storage.best_number = U64::from(num);
                storage.best_hash = hash;
                hash
            };
            let block =
                self.block_by_hash(best_block_hash).await?.ok_or(BlockchainError::BlockNotFound)?;

            let reset_time = block.header.timestamp.to::<u64>();
            self.time.reset(reset_time);

            let mut env = self.env.write();
            env.block = BlockEnv {
                number: rU256::from(num),
                timestamp: block.header.timestamp,
                difficulty: block.header.difficulty,
                // ensures prevrandao is set
                prevrandao: Some(block.header.mix_hash.unwrap_or_default()),
                gas_limit: block.header.gas_limit,
                // Keep previous `coinbase` and `basefee` value
                coinbase: env.block.coinbase,
                basefee: env.block.basefee,
                ..Default::default()
            };
        }
        Ok(self.db.write().await.revert(id, RevertSnapshotAction::RevertRemove))
    }

    pub fn list_snapshots(&self) -> BTreeMap<U256, (u64, B256)> {
        self.active_snapshots.lock().clone().into_iter().collect()
    }

    /// Get the current state.
    pub async fn serialized_state(&self) -> Result<SerializableState, BlockchainError> {
        let at = self.env.read().block.clone();
        let state = self.db.read().await.dump_state(at)?;
        state.ok_or_else(|| {
            RpcError::invalid_params("Dumping state not supported with the current configuration")
                .into()
        })
    }

    /// Write all chain data to serialized bytes buffer
    pub async fn dump_state(&self) -> Result<Bytes, BlockchainError> {
        let state = self.serialized_state().await?;
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder
            .write_all(&serde_json::to_vec(&state).unwrap_or_default())
            .map_err(|_| BlockchainError::DataUnavailable)?;
        Ok(encoder.finish().unwrap_or_default().into())
    }

    /// Apply [SerializableState] data to the backend storage.
    pub async fn load_state(&self, state: SerializableState) -> Result<bool, BlockchainError> {
        // reset the block env
        if let Some(block) = state.block.clone() {
            self.env.write().block = block;
        }

        if !self.db.write().await.load_state(state)? {
            Err(RpcError::invalid_params(
                "Loading state not supported with the current configuration",
            )
            .into())
        } else {
            Ok(true)
        }
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
    fn next_env(&self) -> Env {
        let mut env = self.env.read().clone();
        // increase block number for this block
        env.block.number = env.block.number.saturating_add(rU256::from(1));
        env.block.basefee = self.base_fee();
        env.block.timestamp = rU256::from(self.time.current_call_timestamp());
        env
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
        env.tx = tx.pending_transaction.to_revm_tx_env();
        let db = self.db.read().await;
        let mut inspector = Inspector::default();

        let mut evm = revm::EVM::new();
        evm.env = env;
        evm.database(&*db);
        let result_and_state = match evm.inspect_ref(&mut inspector) {
            Ok(res) => res,
            Err(e) => return Err(e.into()),
        };
        let state = result_and_state.state;
        let (exit_reason, gas_used, out, logs) = match result_and_state.result {
            ExecutionResult::Success { reason, gas_used, logs, output, .. } => {
                (eval_to_instruction_result(reason), gas_used, Some(output), Some(logs))
            }
            ExecutionResult::Revert { gas_used, output } => {
                (InstructionResult::Revert, gas_used, Some(Output::Call(output)), None)
            }
            ExecutionResult::Halt { reason, gas_used } => {
                (halt_to_instruction_result(reason), gas_used, None, None)
            }
        };

        inspector.print_logs();

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
        F: FnOnce(Box<dyn MaybeHashDatabase + '_>, BlockInfo) -> T,
    {
        let db = self.db.read().await;
        let env = self.next_env();

        let mut cache_db = CacheDB::new(&*db);

        let storage = self.blockchain.storage.read();

        let executor = TransactionExecutor {
            db: &mut cache_db,
            validator: self,
            pending: pool_transactions.into_iter(),
            block_env: env.block.clone(),
            cfg_env: env.cfg,
            parent_hash: storage.best_hash,
            gas_used: U256::ZERO,
            enable_steps_tracing: self.enable_steps_tracing,
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
        trace!(target: "backend", "creating new block with {} transactions", pool_transactions.len());

        let (outcome, header, block_hash) = {
            let current_base_fee = self.base_fee();

            let mut env = self.env.read().clone();

            if env.block.basefee.is_zero() {
                // this is an edge case because the evm fails if `tx.effective_gas_price < base_fee`
                // 0 is only possible if it's manually set
                env.cfg.disable_base_fee = true;
            }

            // increase block number for this block
            env.block.number = env.block.number.saturating_add(rU256::from(1));
            env.block.basefee = current_base_fee;
            env.block.timestamp = rU256::from(self.time.next_timestamp());

            let best_hash = self.blockchain.storage.read().best_hash;

            if self.prune_state_history_config.is_state_history_supported() {
                let db = self.db.read().await.current_state();
                // store current state before executing all transactions
                self.states.write().insert(best_hash, db);
            }

            let (executed_tx, block_hash) = {
                let mut db = self.db.write().await;
                let executor = TransactionExecutor {
                    db: &mut *db,
                    validator: self,
                    pending: pool_transactions.into_iter(),
                    block_env: env.block.clone(),
                    cfg_env: env.cfg.clone(),
                    parent_hash: best_hash,
                    gas_used: U256::ZERO,
                    enable_steps_tracing: self.enable_steps_tracing,
                };
                let executed_tx = executor.execute();

                // we also need to update the new blockhash in the db itself
                let block_hash = executed_tx.block.block.header.hash();
                db.insert_block_hash(U256::from(executed_tx.block.block.header.number), block_hash);

                (executed_tx, block_hash)
            };

            // create the new block with the current timestamp
            let ExecutedTransactions { block, included, invalid } = executed_tx;
            let BlockInfo { block, transactions, receipts } = block;

            let header = block.header.clone();
            let block_number: U64 = env.block.number.to::<U64>();

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
                    node_info!("    Contract created: {contract:?}");
                }
                node_info!("    Gas used: {}", receipt.gas_used());
                if !info.exit.is_ok() {
                    let r = decode_revert(
                        &info.out.clone().unwrap_or_default().to_vec(),
                        None,
                        Some(info.exit),
                    );
                    node_info!("    Error: reverted with: {r}");
                }
                node_info!("");

                let mined_tx = MinedTransaction {
                    info,
                    receipt,
                    block_hash,
                    block_number: block_number.to::<u64>(),
                };
                storage.transactions.insert(mined_tx.info.transaction_hash, mined_tx);
            }

            // remove old transactions that exceed the transaction block keeper
            if let Some(transaction_block_keeper) = self.transaction_block_keeper {
                if storage.blocks.len() > transaction_block_keeper {
                    let to_clear = block_number
                        .to::<u64>()
                        .saturating_sub(transaction_block_keeper.try_into().unwrap());
                    storage.remove_block_transactions_by_number(to_clear)
                }
            }

            // we intentionally set the difficulty to `0` for newer blocks
            env.block.difficulty = rU256::from(0);

            // update env with new values
            *self.env.write() = env;

            let timestamp = utc_from_secs(header.timestamp);

            node_info!("    Block Number: {}", block_number);
            node_info!("    Block Hash: {:?}", block_hash);
            node_info!("    Block Time: {:?}\n", timestamp.to_rfc2822());

            let outcome = MinedBlockOutcome { block_number, included, invalid };

            (outcome, header, block_hash)
        };
        let next_block_base_fee = self.fees.get_next_block_base_fee_per_gas(
            header.gas_used.to_alloy(),
            header.gas_limit.to_alloy(),
            header.base_fee_per_gas.unwrap_or_default().to_alloy(),
        );

        // notify all listeners
        self.notify_on_new_block(header, block_hash);

        // update next base fee
        self.fees.set_base_fee(U256::from(next_block_base_fee));

        outcome
    }

    /// Executes the [CallRequest] without writing to the DB
    ///
    /// # Errors
    ///
    /// Returns an error if the `block_number` is greater than the current height
    pub async fn call(
        &self,
        request: CallRequest,
        fee_details: FeeDetails,
        block_request: Option<BlockRequest>,
        overrides: Option<StateOverride>,
    ) -> Result<(InstructionResult, Option<Output>, u64, State), BlockchainError> {
        self.with_database_at(block_request, |state, block| {
            let block_number = (block.number.to_ethers()).as_u64();
            let (exit, out, gas, state) = match overrides {
                None => self.call_with_state(state, request, fee_details, block),
                Some(overrides) => {
                    let state = state::apply_state_override(overrides.into_iter().collect(), state)?;
                    self.call_with_state(state, request, fee_details, block)
                },
            }?;
            trace!(target: "backend", "call return {:?} out: {:?} gas {} on block {}", exit, out, gas, block_number);
            Ok((exit, out, gas, state))
        }).await?
    }

    fn build_call_env(
        &self,
        request: CallRequest,
        fee_details: FeeDetails,
        block_env: BlockEnv,
    ) -> Env {
        let CallRequest { from, to, gas, value, input, nonce, access_list, .. } = request;

        let FeeDetails { gas_price, max_fee_per_gas, max_priority_fee_per_gas } = fee_details;

        let gas_limit = gas.unwrap_or(block_env.gas_limit);
        let mut env = self.env.read().clone();
        env.block = block_env;
        // we want to disable this in eth_call, since this is common practice used by other node
        // impls and providers <https://github.com/foundry-rs/foundry/issues/4388>
        env.cfg.disable_block_gas_limit = true;

        if let Some(base) = max_fee_per_gas {
            env.block.basefee = base;
        }

        let gas_price = gas_price.or(max_fee_per_gas).unwrap_or_else(|| self.gas_price());
        let caller = from.unwrap_or_default();

        env.tx = TxEnv {
            caller,
            gas_limit: gas_limit.to::<u64>(),
            gas_price,
            gas_priority_fee: max_priority_fee_per_gas,
            transact_to: match to {
                Some(addr) => TransactTo::Call(addr),
                None => TransactTo::Create(CreateScheme::Create),
            },
            value: value.unwrap_or_default(),
            data: input.into_input().unwrap_or_default(),
            chain_id: None,
            nonce: nonce.map(|n| n.to::<u64>()),
            access_list: alloy_to_revm_access_list(access_list.unwrap_or_default().0),
            ..Default::default()
        };

        if env.block.basefee == revm::primitives::U256::ZERO {
            // this is an edge case because the evm fails if `tx.effective_gas_price < base_fee`
            // 0 is only possible if it's manually set
            env.cfg.disable_base_fee = true;
        }

        env
    }

    pub fn call_with_state<D>(
        &self,
        state: D,
        request: CallRequest,
        fee_details: FeeDetails,
        block_env: BlockEnv,
    ) -> Result<(InstructionResult, Option<Output>, u64, State), BlockchainError>
    where
        D: DatabaseRef<Error = DatabaseError>,
    {
        let mut inspector = Inspector::default();
        let mut evm = revm::EVM::new();
        evm.env = self.build_call_env(request, fee_details, block_env);
        evm.database(state);
        let result_and_state = match evm.inspect_ref(&mut inspector) {
            Ok(result_and_state) => result_and_state,
            Err(e) => match e {
                EVMError::Transaction(invalid_tx) => {
                    return Err(BlockchainError::InvalidTransaction(invalid_tx.into()))
                }
                EVMError::Database(e) => return Err(BlockchainError::DatabaseError(e)),
                EVMError::Header(e) => match e {
                    InvalidHeader::ExcessBlobGasNotSet => {
                        return Err(BlockchainError::ExcessBlobGasNotSet)
                    }
                    InvalidHeader::PrevrandaoNotSet => {
                        return Err(BlockchainError::PrevrandaoNotSet)
                    }
                },
            },
        };
        let state = result_and_state.state;
        let (exit_reason, gas_used, out) = match result_and_state.result {
            ExecutionResult::Success { reason, gas_used, output, .. } => {
                (eval_to_instruction_result(reason), gas_used, Some(output))
            }
            ExecutionResult::Revert { gas_used, output } => {
                (InstructionResult::Revert, gas_used, Some(Output::Call(output)))
            }
            ExecutionResult::Halt { reason, gas_used } => {
                (halt_to_instruction_result(reason), gas_used, None)
            }
        };
        inspector.print_logs();
        Ok((exit_reason, out, gas_used, state))
    }

    pub async fn call_with_tracing(
        &self,
        request: CallRequest,
        fee_details: FeeDetails,
        block_request: Option<BlockRequest>,
        opts: GethDefaultTracingOptions,
    ) -> Result<DefaultFrame, BlockchainError> {
        self.with_database_at(block_request, |state, block| {
            let mut inspector = Inspector::default().with_steps_tracing();
            let block_number = block.number;
            let mut evm = revm::EVM::new();
            evm.env = self.build_call_env(request, fee_details, block);
            evm.database(state);
            let result_and_state =
                match evm.inspect_ref(&mut inspector) {
                    Ok(result_and_state) => result_and_state,
                    Err(e) => return Err(e.into()),
                };
            let (exit_reason, gas_used, out, ) = match result_and_state.result {
                ExecutionResult::Success { reason, gas_used, output, .. } => {
                    (eval_to_instruction_result(reason), gas_used, Some(output), )
                },
                ExecutionResult::Revert { gas_used, output} => {
                    (InstructionResult::Revert, gas_used, Some(Output::Call(output)))
                },
                ExecutionResult::Halt { reason, gas_used } => {
                    (halt_to_instruction_result(reason), gas_used, None)
                },
            };
            let res = inspector.tracer.unwrap_or(TracingInspector::new(TracingInspectorConfig::all())).into_geth_builder().geth_traces(gas_used, match &out {
                Some(out) => out.data().clone(),
                None => Bytes::new()
            }, opts);
            trace!(target: "backend", "trace call return {:?} out: {:?} gas {} on block {}", exit_reason, out, gas_used, block_number);
            Ok(res)
        })
        .await?
    }

    pub fn build_access_list_with_state<D>(
        &self,
        state: D,
        request: CallRequest,
        fee_details: FeeDetails,
        block_env: BlockEnv,
    ) -> Result<(InstructionResult, Option<Output>, u64, AccessList), BlockchainError>
    where
        D: DatabaseRef<Error = DatabaseError>,
    {
        let from = request.from.unwrap_or_default();
        let to = if let Some(to) = request.to {
            to
        } else {
            let nonce = state.basic_ref(from)?.unwrap_or_default().nonce;
            from.create(nonce)
        };

        let mut tracer = AccessListTracer::new(
            request.access_list.clone().unwrap_or_default(),
            from,
            to,
            self.precompiles(),
        );

        let mut evm = revm::EVM::new();
        evm.env = self.build_call_env(request, fee_details, block_env);
        evm.database(state);
        let result_and_state = match evm.inspect_ref(&mut tracer) {
            Ok(result_and_state) => result_and_state,
            Err(e) => return Err(e.into()),
        };
        let (exit_reason, gas_used, out) = match result_and_state.result {
            ExecutionResult::Success { reason, gas_used, output, .. } => {
                (eval_to_instruction_result(reason), gas_used, Some(output))
            }
            ExecutionResult::Revert { gas_used, output } => {
                (InstructionResult::Revert, gas_used, Some(Output::Call(output)))
            }
            ExecutionResult::Halt { reason, gas_used } => {
                (halt_to_instruction_result(reason), gas_used, None)
            }
        };
        let access_list = tracer.access_list();
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
            return Ok(self.mined_logs_for_block(filter, block));
        }

        if let Some(fork) = self.get_fork() {
            return fork.logs(&filter).await.map_err(|_| BlockchainError::DataUnavailable);
        }

        Ok(Vec::new())
    }

    /// Returns all `Log`s mined by the node that were emitted in the `block` and match the `Filter`
    fn mined_logs_for_block(&self, filter: Filter, block: Block) -> Vec<Log> {
        let params = FilteredParams::new(Some(filter.clone()));
        let mut all_logs = Vec::new();
        let block_hash = block.header.hash();
        let mut block_log_index = 0u32;

        let transactions: Vec<_> = {
            let storage = self.blockchain.storage.read();
            block
                .transactions
                .iter()
                .filter_map(|tx| storage.transactions.get(&tx.hash()).map(|tx| tx.info.clone()))
                .collect()
        };

        for transaction in transactions {
            let logs = transaction.logs.clone();
            let transaction_hash = transaction.transaction_hash;

            for log in logs.into_iter() {
                let mut log = Log {
                    address: log.address,
                    topics: log.topics().into_iter().map(|t| *t).collect(),
                    data: log.data.data,
                    block_hash: None,
                    block_number: None,
                    transaction_hash: None,
                    transaction_index: None,
                    log_index: None,
                    removed: false,
                };
                let mut is_match: bool = true;
                if !filter.address.is_empty() && filter.has_topics() {
                    if !params.filter_address(&log) || !params.filter_topics(&log) {
                        is_match = false;
                    }
                } else if !filter.address.is_empty() {
                    if !params.filter_address(&log) {
                        is_match = false;
                    }
                } else if filter.has_topics() && !params.filter_topics(&log) {
                    is_match = false;
                }

                if is_match {
                    log.block_hash = Some(block_hash);
                    log.block_number = Some(block.header.number.to_alloy());
                    log.transaction_hash = Some(transaction_hash);
                    log.transaction_index = Some(U256::from(transaction.transaction_index));
                    log.log_index = Some(U256::from(block_log_index));
                    all_logs.push(log);
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

            if fork.predates_fork(from) {
                // this data is only available on the forked client
                let filter = filter.clone().from_block(from).to_block(to_on_fork);
                all_logs =
                    fork.logs(&filter).await.map_err(|_| BlockchainError::DataUnavailable)?;

                // update the range
                from = fork.block_number() + 1;
            }
        }

        for number in from..=to {
            if let Some(block) = self.get_block(number) {
                all_logs.extend(self.mined_logs_for_block(filter.clone(), block));
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

    pub async fn block_by_hash(&self, hash: B256) -> Result<Option<AlloyBlock>, BlockchainError> {
        trace!(target: "backend", "get block by hash {:?}", hash);
        if let tx @ Some(_) = self.mined_block_by_hash(hash) {
            return Ok(tx);
        }

        if let Some(fork) = self.get_fork() {
            return fork.block_by_hash(hash).await.map_err(|_| BlockchainError::DataUnavailable);
        }

        Ok(None)
    }

    pub async fn block_by_hash_full(
        &self,
        hash: B256,
    ) -> Result<Option<AlloyBlock>, BlockchainError> {
        trace!(target: "backend", "get block by hash {:?}", hash);
        if let tx @ Some(_) = self.get_full_block(hash) {
            return Ok(tx);
        }

        if let Some(fork) = self.get_fork() {
            return fork
                .block_by_hash_full(hash)
                .await
                .map_err(|_| BlockchainError::DataUnavailable);
        }

        Ok(None)
    }

    fn mined_block_by_hash(&self, hash: B256) -> Option<AlloyBlock> {
        let block = self.blockchain.get_block_by_hash(&hash)?;
        Some(self.convert_block(block))
    }

    pub(crate) async fn mined_transactions_by_block_number(
        &self,
        number: BlockNumber,
    ) -> Option<Vec<Transaction>> {
        if let Some(block) = self.get_block(number) {
            return self.mined_transactions_in_block(&block);
        }
        None
    }

    /// Returns all transactions given a block
    pub(crate) fn mined_transactions_in_block(&self, block: &Block) -> Option<Vec<Transaction>> {
        let mut transactions = Vec::with_capacity(block.transactions.len());
        let base_fee = block.header.base_fee_per_gas;
        let storage = self.blockchain.storage.read();
        for hash in block.transactions.iter().map(|tx| tx.hash()) {
            let info = storage.transactions.get(&hash)?.info.clone();
            let tx = block.transactions.get(info.transaction_index as usize)?.clone();

            let tx = transaction_build(
                Some(hash),
                tx,
                Some(block),
                Some(info),
                base_fee.map(|f| f.to_alloy()),
            );
            transactions.push(tx);
        }
        Some(transactions)
    }

    pub async fn block_by_number(
        &self,
        number: BlockNumber,
    ) -> Result<Option<AlloyBlock>, BlockchainError> {
        trace!(target: "backend", "get block by number {:?}", number);
        if let tx @ Some(_) = self.mined_block_by_number(number) {
            return Ok(tx);
        }

        if let Some(fork) = self.get_fork() {
            let number = self.convert_block_number(Some(number));
            if fork.predates_fork_inclusive(number) {
                return fork
                    .block_by_number(number)
                    .await
                    .map_err(|_| BlockchainError::DataUnavailable);
            }
        }

        Ok(None)
    }

    pub async fn block_by_number_full(
        &self,
        number: BlockNumber,
    ) -> Result<Option<AlloyBlock>, BlockchainError> {
        trace!(target: "backend", "get block by number {:?}", number);
        if let tx @ Some(_) = self.get_full_block(number) {
            return Ok(tx);
        }

        if let Some(fork) = self.get_fork() {
            let number = self.convert_block_number(Some(number));
            if fork.predates_fork_inclusive(number) {
                return fork
                    .block_by_number_full(number)
                    .await
                    .map_err(|_| BlockchainError::DataUnavailable);
            }
        }

        Ok(None)
    }

    pub fn get_block(&self, id: impl Into<BlockId>) -> Option<Block> {
        let hash = match id.into() {
            BlockId::Hash(hash) => hash.block_hash,
            BlockId::Number(number) => {
                let storage = self.blockchain.storage.read();
                let slots_in_an_epoch = U64::from(32u64);
                match number {
                    BlockNumber::Latest => storage.best_hash,
                    BlockNumber::Earliest => storage.genesis_hash,
                    BlockNumber::Pending => return None,
                    BlockNumber::Number(num) => *storage.hashes.get(&U64::from(num))?,
                    BlockNumber::Safe => {
                        if storage.best_number > (slots_in_an_epoch) {
                            *storage.hashes.get(&(storage.best_number - (slots_in_an_epoch)))?
                        } else {
                            storage.genesis_hash // treat the genesis block as safe "by definition"
                        }
                    }
                    BlockNumber::Finalized => {
                        if storage.best_number.to_ethers() > (slots_in_an_epoch.to_ethers() * 2) {
                            *storage.hashes.get(
                                &(storage.best_number.to_ethers() -
                                    (slots_in_an_epoch.to_ethers() * 2))
                                    .to_alloy(),
                            )?
                        } else {
                            storage.genesis_hash
                        }
                    }
                }
            }
        };
        self.get_block_by_hash(hash)
    }

    pub fn get_block_by_hash(&self, hash: B256) -> Option<Block> {
        self.blockchain.get_block_by_hash(&hash)
    }

    pub fn mined_block_by_number(&self, number: BlockNumber) -> Option<AlloyBlock> {
        let block = self.get_block(number)?;
        let mut block = self.convert_block(block);
        block.transactions.convert_to_hashes();
        Some(block)
    }

    pub fn get_full_block(&self, id: impl Into<BlockId>) -> Option<AlloyBlock> {
        let block = self.get_block(id)?;
        let transactions = self.mined_transactions_in_block(&block)?;
        let block = self.convert_block(block);
        Some(block.into_full_block(transactions))
    }

    /// Takes a block as it's stored internally and returns the eth api conform block format
    pub fn convert_block(&self, block: Block) -> AlloyBlock {
        let size = U256::from(alloy_rlp::encode(&block).len() as u32);

        let Block { header, transactions, .. } = block;

        let hash = header.hash();
        let Header {
            parent_hash,
            ommers_hash,
            beneficiary,
            state_root,
            transactions_root,
            receipts_root,
            logs_bloom,
            difficulty,
            number,
            gas_limit,
            gas_used,
            timestamp,
            extra_data,
            mix_hash,
            nonce,
            base_fee_per_gas,
            withdrawals_root: None,
            blob_gas_used: None,
            excess_blob_gas: None,
            parent_beacon_block_root: None,
        } = header;

        AlloyBlock {
            total_difficulty: Some(self.total_difficulty()),
            header: AlloyHeader {
                hash: Some(hash),
                parent_hash: parent_hash,
                uncles_hash: ommers_hash,
                miner: beneficiary,
                state_root: state_root,
                transactions_root: transactions_root,
                receipts_root: receipts_root,
                number: Some(number.to_alloy()),
                gas_used: gas_used.to_alloy(),
                gas_limit: gas_limit.to_alloy(),
                extra_data: extra_data.0.into(),
                logs_bloom,
                timestamp: U256::from(timestamp),
                difficulty: difficulty,
                mix_hash: Some(mix_hash),
                nonce: Some(B64::from(nonce)),
                base_fee_per_gas: base_fee_per_gas.map(|f| f.to_alloy()),
                withdrawals_root: None,
                blob_gas_used: None,
                excess_blob_gas: None,
                parent_beacon_block_root: None,
            },
            size: Some(size),
            transactions: alloy_rpc_types::BlockTransactions::Hashes(
                transactions.into_iter().map(|tx| tx.hash()).collect(),
            ),
            uncles: vec![],
            withdrawals: None,
            other: Default::default(),
        }
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
        let slots_in_an_epoch = 32u64;
        let requested =
            match block_id.map(Into::into).unwrap_or(BlockId::Number(BlockNumber::Latest)) {
                BlockId::Hash(hash) => self
                    .block_by_hash(hash.block_hash)
                    .await?
                    .ok_or(BlockchainError::BlockNotFound)?
                    .header
                    .number
                    .ok_or(BlockchainError::BlockNotFound)?
                    .to::<u64>(),
                BlockId::Number(num) => match num {
                    BlockNumber::Latest | BlockNumber::Pending => self.best_number(),
                    BlockNumber::Earliest => U64::ZERO.to::<u64>(),
                    BlockNumber::Number(num) => num,
                    BlockNumber::Safe => {
                        U64::from(current).saturating_sub(U64::from(slots_in_an_epoch)).to::<u64>()
                    }
                    BlockNumber::Finalized => U64::from(current)
                        .saturating_sub(U64::from(slots_in_an_epoch) * U64::from(2))
                        .to::<u64>(),
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
        let slots_in_an_epoch = 32u64;
        match block.unwrap_or(BlockNumber::Latest) {
            BlockNumber::Latest | BlockNumber::Pending => current,
            BlockNumber::Earliest => 0,
            BlockNumber::Number(num) => num,
            BlockNumber::Safe => current.saturating_sub(slots_in_an_epoch),
            BlockNumber::Finalized => current.saturating_sub(slots_in_an_epoch * 2),
        }
    }

    /// Helper function to execute a closure with the database at a specific block
    pub async fn with_database_at<F, T>(
        &self,
        block_request: Option<BlockRequest>,
        f: F,
    ) -> Result<T, BlockchainError>
    where
        F: FnOnce(Box<dyn MaybeHashDatabase + '_>, BlockEnv) -> T,
    {
        let block_number = match block_request {
            Some(BlockRequest::Pending(pool_transactions)) => {
                let result = self
                    .with_pending_block(pool_transactions, |state, block| {
                        let block = block.block;
                        let block = BlockEnv {
                            number: block.header.number.to_alloy(),
                            coinbase: block.header.beneficiary,
                            timestamp: rU256::from(block.header.timestamp),
                            difficulty: block.header.difficulty,
                            prevrandao: Some(block.header.mix_hash),
                            basefee: block.header.base_fee_per_gas.unwrap_or_default().to_alloy(),
                            gas_limit: block.header.gas_limit.to_alloy(),
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
        let block_number: U256 = U256::from(self.convert_block_number(block_number));

        if block_number < self.env.read().block.number {
            {
                let mut states = self.states.write();

                if let Some((state, block)) = self
                    .get_block(block_number.to::<u64>())
                    .and_then(|block| Some((states.get(&block.header.hash())?, block)))
                {
                    let block = BlockEnv {
                        number: block.header.number.to_alloy(),
                        coinbase: block.header.beneficiary,
                        timestamp: rU256::from(block.header.timestamp),
                        difficulty: block.header.difficulty,
                        prevrandao: Some(block.header.mix_hash),
                        basefee: block.header.base_fee_per_gas.unwrap_or_default().to_alloy(),
                        gas_limit: block.header.gas_limit.to_alloy(),
                        ..Default::default()
                    };
                    return Ok(f(Box::new(state), block));
                }
            }

            // there's an edge case in forking mode if the requested `block_number` is __exactly__
            // the forked block, which should be fetched from remote but since we allow genesis
            // accounts this may not be accurate data because an account could be provided via
            // genesis
            // So this provides calls the given provided function `f` with a genesis aware database
            if let Some(fork) = self.get_fork() {
                if block_number == U256::from(fork.block_number()) {
                    let mut block = self.env.read().block.clone();
                    let db = self.db.read().await;
                    let gen_db = self.genesis.state_db_at_genesis(Box::new(&*db));

                    block.number = block_number;
                    block.timestamp = rU256::from(fork.timestamp());
                    block.basefee = fork.base_fee().unwrap_or_default();

                    return Ok(f(Box::new(&gen_db), block));
                }
            }

            warn!(target: "backend", "Not historic state found for block={}", block_number);
            return Err(BlockchainError::BlockOutOfRange(
                self.env.read().block.number.to_ethers().as_u64(),
                block_number.to::<u64>(),
            ));
        }

        let db = self.db.read().await;
        let block = self.env.read().block.clone();
        Ok(f(Box::new(&*db), block))
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
            Ok(u256_to_h256_be(val.to_ethers()).to_alloy())
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
        self.with_database_at(block_request, |db, _| self.get_code_with_state(db, address)).await?
    }

    pub fn get_code_with_state<D>(
        &self,
        state: D,
        address: Address,
    ) -> Result<Bytes, BlockchainError>
    where
        D: DatabaseRef<Error = DatabaseError>,
    {
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

    pub fn get_balance_with_state<D>(
        &self,
        state: D,
        address: Address,
    ) -> Result<U256, BlockchainError>
    where
        D: DatabaseRef<Error = DatabaseError>,
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
        block_request: Option<BlockRequest>,
    ) -> Result<U256, BlockchainError> {
        if let Some(BlockRequest::Pending(pool_transactions)) = block_request.as_ref() {
            if let Some(value) = get_pool_transactions_nonce(pool_transactions, address) {
                return Ok(value);
            }
        }
        let final_block_request = match block_request {
            Some(BlockRequest::Pending(_)) => Some(BlockRequest::Number(self.best_number())),
            Some(BlockRequest::Number(bn)) => Some(BlockRequest::Number(bn)),
            None => None,
        };
        self.with_database_at(final_block_request, |db, _| {
            trace!(target: "backend", "get nonce for {:?}", address);
            Ok(U256::from(db.basic_ref(address)?.unwrap_or_default().nonce))
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
            return Ok(fork.trace_transaction(hash).await?)
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
        for tx in block.transactions {
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
        if let Some(traces) = self.mined_geth_trace_transaction(hash, opts.clone()) {
            return Ok(GethTrace::Default(traces));
        }

        if let Some(fork) = self.get_fork() {
            return fork
                .debug_trace_transaction(hash, opts)
                .await
                .map_err(|_| BlockchainError::DataUnavailable)
        }

        Ok(GethTrace::Default(Default::default()))
    }

    fn mined_geth_trace_transaction(
        &self,
        hash: B256,
        opts: GethDebugTracingOptions,
    ) -> Option<DefaultFrame> {
        self.blockchain.storage.read().transactions.get(&hash).map(|tx| tx.geth_trace(opts.config))
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

        if let Some(fork) = self.get_fork() {
            if fork.predates_fork(number) {
                return fork.trace_block(number).await.map_err(|_| BlockchainError::DataUnavailable)
            }
        }

        Ok(vec![])
    }

    pub async fn transaction_receipt(
        &self,
        hash: B256,
    ) -> Result<Option<TransactionReceipt>, BlockchainError> {
        if let Some(receipt) = self.mined_transaction_receipt(hash) {
            return Ok(Some(receipt.inner));
        }

        if let Some(fork) = self.get_fork() {
            let receipt = fork
                .transaction_receipt(hash)
                .await
                .map_err(|_| BlockchainError::DataUnavailable)?;
            let number = self.convert_block_number(
                receipt
                    .clone()
                    .and_then(|r| r.block_number)
                    .map(|n| BlockNumber::from(n.to::<u64>())),
            );

            if fork.predates_fork_inclusive(number) {
                return Ok(receipt);
            }
        }

        Ok(None)
    }

    /// Returns all receipts of the block
    pub fn mined_receipts(&self, hash: B256) -> Option<Vec<TypedReceipt>> {
        let block = self.mined_block_by_hash(hash)?;
        let mut receipts = Vec::new();
        let storage = self.blockchain.storage.read();
        for tx in block.transactions.hashes() {
            let receipt = storage.transactions.get(tx)?.receipt.clone();
            receipts.push(receipt);
        }
        Some(receipts)
    }

    /// Returns all transaction receipts of the block
    pub fn mined_block_receipts(&self, id: impl Into<BlockId>) -> Option<Vec<TransactionReceipt>> {
        let mut receipts = Vec::new();
        let block = self.get_block(id)?;

        for transaction in block.transactions {
            let receipt = self.mined_transaction_receipt(transaction.hash())?;
            receipts.push(receipt.inner);
        }

        Some(receipts)
    }

    /// Returns the transaction receipt for the given hash
    pub(crate) fn mined_transaction_receipt(&self, hash: B256) -> Option<MinedTransactionReceipt> {
        let MinedTransaction { info, receipt, block_hash, .. } =
            self.blockchain.get_transaction_by_hash(&hash)?;

        let ReceiptWithBloom { receipt, bloom } = receipt.into();
        let Receipt { success, cumulative_gas_used, logs } = receipt;
        let logs_bloom = bloom;

        let index = info.transaction_index as usize;

        let block = self.blockchain.get_block_by_hash(&block_hash)?;

        // TODO store cumulative gas used in receipt instead
        let receipts = self.get_receipts(block.transactions.iter().map(|tx| tx.hash()));

        let mut cumulative_gas_used = U256::ZERO;
        for receipt in receipts.iter().take(index + 1) {
            cumulative_gas_used = cumulative_gas_used.saturating_add(receipt.gas_used());
        }

        // cumulative_gas_used = cumulative_gas_used.saturating_sub(gas_used);

        let mut cumulative_receipts = receipts;
        cumulative_receipts.truncate(index + 1);

        let transaction = block.transactions[index].clone();

        let transaction_type = transaction.transaction.r#type();

        let effective_gas_price = match transaction.transaction {
            TypedTransaction::Legacy(t) => t.gas_price,
            TypedTransaction::EIP2930(t) => t.gas_price,
            TypedTransaction::EIP1559(t) => block
                .header
                .base_fee_per_gas
                .map(|b| b as u128)
                .unwrap_or(self.base_fee().to::<u128>())
                .checked_add(t.max_priority_fee_per_gas)
                .unwrap_or(u128::MAX) as u128,
            TypedTransaction::Deposit(_) => 0 as u128,
        };

        let deposit_nonce = transaction_type.and_then(|x| (x == 0x7E).then_some(info.nonce));

        let mut inner = TransactionReceipt {
            transaction_hash: Some(info.transaction_hash),
            transaction_index: U64::from(info.transaction_index),
            block_hash: Some(block_hash),
            block_number: Some(U256::from(block.header.number)),
            from: info.from,
            to: info.to,
            cumulative_gas_used,
            gas_used: Some(cumulative_gas_used),
            contract_address: info.contract_address,
            logs: {
                let mut pre_receipts_log_index = None;
                if !cumulative_receipts.is_empty() {
                    cumulative_receipts.truncate(cumulative_receipts.len() - 1);
                    pre_receipts_log_index =
                        Some(cumulative_receipts.iter().map(|_r| logs.len() as u32).sum::<u32>());
                }
                logs.iter()
                    .enumerate()
                    .map(|(i, log)| Log {
                        address: log.address,
                        topics: log.topics().clone().into_iter().map(|t| *t).collect(),
                        data: log.data.data.clone().into(),
                        block_hash: Some(block_hash),
                        block_number: Some(U256::from(block.header.number)),
                        transaction_hash: Some(info.transaction_hash),
                        transaction_index: Some(U256::from(info.transaction_index)),
                        log_index: Some(U256::from(
                            (pre_receipts_log_index.unwrap_or(0)) + i as u32,
                        )),
                        removed: false,
                    })
                    .collect()
            },
            status_code: Some(U64::from(success)),
            state_root: None,
            logs_bloom,
            transaction_type: transaction_type.map(U8::from).unwrap_or_default(),
            effective_gas_price: U128::from(effective_gas_price),
            blob_gas_price: None,
            blob_gas_used: None,
            other: Default::default(),
        };

        inner.other.insert(
            "deposit_nonce".to_string(),
            serde_json::to_value(deposit_nonce).expect("Infallible"),
        );

        Some(MinedTransactionReceipt { inner, out: info.out.map(|o| o.0.into()) })
    }

    /// Returns the blocks receipts for the given number
    pub async fn block_receipts(
        &self,
        number: BlockNumber,
    ) -> Result<Option<Vec<TransactionReceipt>>, BlockchainError> {
        if let Some(receipts) = self.mined_block_receipts(number) {
            return Ok(Some(receipts));
        }

        if let Some(fork) = self.get_fork() {
            let number = self.convert_block_number(Some(number));

            if fork.predates_fork_inclusive(number) {
                let receipts = fork
                    .block_receipts(number)
                    .await
                    .map_err(BlockchainError::AlloyForkProvider)?;

                return Ok(receipts);
            }
        }

        Ok(None)
    }

    pub async fn transaction_by_block_number_and_index(
        &self,
        number: BlockNumber,
        index: Index,
    ) -> Result<Option<Transaction>, BlockchainError> {
        if let Some(hash) = self.mined_block_by_number(number).and_then(|b| b.header.hash) {
            return Ok(self.mined_transaction_by_block_hash_and_index(hash, index));
        }

        if let Some(fork) = self.get_fork() {
            let number = self.convert_block_number(Some(number));
            if fork.predates_fork(number) {
                return fork
                    .transaction_by_block_number_and_index(number, index.into())
                    .await
                    .map_err(|_| BlockchainError::DataUnavailable);
            }
        }

        Ok(None)
    }

    pub async fn transaction_by_block_hash_and_index(
        &self,
        hash: B256,
        index: Index,
    ) -> Result<Option<Transaction>, BlockchainError> {
        if let tx @ Some(_) = self.mined_transaction_by_block_hash_and_index(hash, index) {
            return Ok(tx);
        }

        if let Some(fork) = self.get_fork() {
            return fork
                .transaction_by_block_hash_and_index(hash, index.into())
                .await
                .map_err(|_| BlockchainError::DataUnavailable);
        }

        Ok(None)
    }

    fn mined_transaction_by_block_hash_and_index(
        &self,
        block_hash: B256,
        index: Index,
    ) -> Option<Transaction> {
        let (info, block, tx) = {
            let storage = self.blockchain.storage.read();
            let block = storage.blocks.get(&block_hash).cloned()?;
            let index: usize = index.into();
            let tx = block.transactions.get(index)?.clone();
            let info = storage.transactions.get(&tx.hash())?.info.clone();
            (info, block, tx)
        };

        Some(transaction_build(
            Some(info.transaction_hash),
            tx,
            Some(&block),
            Some(info),
            block.header.base_fee_per_gas.map(|g| g.to_alloy()),
        ))
    }

    pub async fn transaction_by_hash(
        &self,
        hash: B256,
    ) -> Result<Option<Transaction>, BlockchainError> {
        trace!(target: "backend", "transaction_by_hash={:?}", hash);
        if let tx @ Some(_) = self.mined_transaction_by_hash(hash) {
            return Ok(tx);
        }

        if let Some(fork) = self.get_fork() {
            return fork.transaction_by_hash(hash).await.map_err(BlockchainError::AlloyForkProvider)
        }

        Ok(None)
    }

    fn mined_transaction_by_hash(&self, hash: B256) -> Option<Transaction> {
        let (info, block) = {
            let storage = self.blockchain.storage.read();
            let MinedTransaction { info, block_hash, .. } =
                storage.transactions.get(&hash)?.clone();
            let block = storage.blocks.get(&block_hash).cloned()?;
            (info, block)
        };
        let tx = block.transactions.get(info.transaction_index as usize)?.clone();

        Some(transaction_build(
            Some(info.transaction_hash),
            tx,
            Some(&block),
            Some(info),
            block.header.base_fee_per_gas.map(|g| g.to_alloy()),
        ))
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
        let account_key = B256::from(keccak256(address.to_ethers().as_bytes()));
        let block_number = block_request.as_ref().map(|r| r.block_number());

        self.with_database_at(block_request, |block_db, _| {
            trace!(target: "backend", "get proof for {:?} at {:?}", address, block_number);
            let (db, root) = block_db.maybe_as_hash_db().ok_or(BlockchainError::DataUnavailable)?;

            let data: &dyn HashDB<_, _> = db.deref();
            let mut recorder = Recorder::new();
            let trie = RefTrieDB::new(&data, &root.0)
                .map_err(|err| BlockchainError::TrieError(err.to_string()))?;

            let maybe_account: Option<BasicAccount> = {
                let acc_decoder = |bytes: &[u8]| {
                    rlp::decode(bytes).unwrap_or_else(|_| {
                        panic!("prove_account_at, could not query trie for account={:?}", &address)
                    })
                };
                let query = (&mut recorder, acc_decoder);
                trie.get_with(account_key.to_ethers().as_bytes(), query)
                    .map_err(|err| BlockchainError::TrieError(err.to_string()))?
            };
            let account = maybe_account.unwrap_or_default();

            let proof = recorder
                .drain()
                .into_iter()
                .map(|r| r.data)
                .map(|record| {
                    // proof is rlp encoded:
                    // <https://github.com/foundry-rs/foundry/issues/5004>
                    // <https://www.quicknode.com/docs/ethereum/eth_getProof>
                    rlp::encode(&record).to_vec().into()
                })
                .collect::<Vec<_>>();

            let account_db =
                block_db.maybe_account_db(address).ok_or(BlockchainError::DataUnavailable)?;

            let account_proof = AccountProof {
                address: address.to_ethers(),
                balance: account.balance,
                nonce: account.nonce.as_u64().into(),
                code_hash: account.code_hash,
                storage_hash: account.storage_root,
                account_proof: proof,
                storage_proof: keys
                    .into_iter()
                    .map(|storage_key| {
                        // the key that should be proofed is the keccak256 of the storage key
                        let key = B256::from(keccak256(storage_key));
                        prove_storage(&account, &account_db.0, key).map(
                            |(storage_proof, storage_value)| StorageProof {
                                key: storage_key.to_ethers(),
                                value: storage_value.to_ethers().into_uint(),
                                proof: storage_proof
                                    .into_iter()
                                    .map(|proof| {
                                        // proof is rlp encoded:
                                        // <https://github.com/foundry-rs/foundry/issues/5004>
                                        // <https://www.quicknode.com/docs/ethereum/eth_getProof>
                                        rlp::encode(&proof).to_vec().into()
                                    })
                                    .collect(),
                            },
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?,
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
}

/// Get max nonce from transaction pool by address
fn get_pool_transactions_nonce(
    pool_transactions: &[Arc<PoolTransaction>],
    address: Address,
) -> Option<U256> {
    let highest_nonce_tx = pool_transactions
        .iter()
        .filter(|tx| *tx.pending_transaction.sender() == address)
        .reduce(|accum, item| {
            let nonce = item.pending_transaction.nonce();
            if nonce.gt(&accum.pending_transaction.nonce()) {
                item
            } else {
                accum
            }
        });
    if let Some(highest_nonce_tx) = highest_nonce_tx {
        return Some(highest_nonce_tx.pending_transaction.nonce().saturating_add(U256::from(1)));
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
                if let Some(legacy) = tx.as_legacy() {
                    // <https://github.com/ethereum/EIPs/blob/master/EIPS/eip-155.md>
                    if env.cfg.spec_id >= SpecId::SPURIOUS_DRAGON &&
                        !legacy.meets_eip155(chain_id.to::<u64>())
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

        if tx.gas_limit() < MIN_TRANSACTION_GAS {
            warn!(target: "backend", "[{:?}] gas too low", tx.hash());
            return Err(InvalidTransactionError::GasTooLow);
        }

        // Check gas limit, iff block gas limit is set.
        if !env.cfg.disable_block_gas_limit && tx.gas_limit() > env.block.gas_limit {
            warn!(target: "backend", "[{:?}] gas too high", tx.hash());
            return Err(InvalidTransactionError::GasTooHigh(ErrDetail {
                detail: String::from("tx.gas_limit > env.block.gas_limit"),
            }));
        }

        // check nonce
        let is_deposit_tx =
            matches!(&pending.transaction.transaction, TypedTransaction::Deposit(_));
        let nonce: u64 =
            (tx.nonce().to::<u64>()).try_into().map_err(|_| InvalidTransactionError::NonceMaxValue)?;
        if nonce < account.nonce && !is_deposit_tx {
            warn!(target: "backend", "[{:?}] nonce too low", tx.hash());
            return Err(InvalidTransactionError::NonceTooLow);
        }

        if (env.cfg.spec_id as u8) >= (SpecId::LONDON as u8) {
            if tx.gas_price() < env.block.basefee && !is_deposit_tx {
                warn!(target: "backend", "max fee per gas={}, too low, block basefee={}",tx.gas_price(),  env.block.basefee);
                return Err(InvalidTransactionError::FeeCapTooLow);
            }

            if let (Some(max_priority_fee_per_gas), Some(max_fee_per_gas)) =
                (tx.essentials().max_priority_fee_per_gas, tx.essentials().max_fee_per_gas)
            {
                if max_priority_fee_per_gas > max_fee_per_gas {
                    warn!(target: "backend", "max priority fee per gas={}, too high, max fee per gas={}", max_priority_fee_per_gas, max_fee_per_gas);
                    return Err(InvalidTransactionError::TipAboveFeeCap);
                }
            }
        }

        let max_cost = tx.max_cost();
        let value = tx.value();
        // check sufficient funds: `gas * price + value`
        let req_funds = max_cost.checked_add(value).ok_or_else(|| {
            warn!(target: "backend", "[{:?}] cost too high",
            tx.hash());
            InvalidTransactionError::InsufficientFunds
        })?;

        if account.balance < req_funds {
            warn!(target: "backend", "[{:?}] insufficient allowance={}, required={} account={:?}", tx.hash(), account.balance, req_funds, *pending.sender());
            return Err(InvalidTransactionError::InsufficientFunds);
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
        if tx.nonce().to::<u64>() > account.nonce {
            return Err(InvalidTransactionError::NonceTooHigh);
        }
        Ok(())
    }
}

/// Creates a `Transaction` as it's expected for the `eth` RPC api from storage data
#[allow(clippy::too_many_arguments)]
pub fn transaction_build(
    tx_hash: Option<B256>,
    eth_transaction: MaybeImpersonatedTransaction,
    block: Option<&Block>,
    info: Option<TransactionInfo>,
    base_fee: Option<U256>,
) -> Transaction {
    let mut transaction: Transaction = eth_transaction.clone().into();
    if info.is_some() && transaction.transaction_type.unwrap_or(U64::ZERO).to::<u64>() == 0x7E {
        transaction.nonce = U64::from(info.as_ref().unwrap().nonce);
    }

    if eth_transaction.is_dynamic_fee() {
        if block.is_none() && info.is_none() {
            // transaction is not mined yet, gas price is considered just `max_fee_per_gas`
            transaction.gas_price = transaction.max_fee_per_gas;
        } else {
            // if transaction is already mined, gas price is considered base fee + priority fee: the
            // effective gas price.
            let base_fee = base_fee.unwrap_or(U256::ZERO);
            let max_priority_fee_per_gas =
                transaction.max_priority_fee_per_gas.map(|g| g.to::<U256>()).unwrap_or(U256::ZERO);
            transaction.gas_price = Some(
                base_fee.checked_add(max_priority_fee_per_gas).unwrap_or(U256::MAX).to::<U128>(),
            );
        }
    } else {
        transaction.max_fee_per_gas = None;
        transaction.max_priority_fee_per_gas = None;
    }

    transaction.block_hash =
        block.as_ref().map(|block| B256::from(keccak256(&alloy_rlp::encode(&block.header))));

    transaction.block_number = block.as_ref().map(|block| U256::from(block.header.number));

    transaction.transaction_index =
        info.as_ref().map(|status| U256::from(status.transaction_index));

    // need to check if the signature of the transaction is impersonated, if so then we
    // can't recover the sender, instead we use the sender from the executed transaction and set the
    // impersonated hash.
    if eth_transaction.is_impersonated() {
        transaction.from = info.as_ref().map(|info| info.from).unwrap_or_default();
        transaction.hash = eth_transaction.impersonated_hash(transaction.from);
    } else {
        transaction.from = eth_transaction.recover().expect("can recover signed tx");
    }

    // if a specific hash was provided we update the transaction's hash
    // This is important for impersonated transactions since they all use the `BYPASS_SIGNATURE`
    // which would result in different hashes
    // Note: for impersonated transactions this only concerns pending transactions because there's
    // no `info` yet.
    if let Some(tx_hash) = tx_hash {
        transaction.hash = tx_hash;
    }

    transaction.to = info.as_ref().map_or(eth_transaction.to(), |status| status.to);
    transaction
}

/// Prove a storage key's existence or nonexistence in the account's storage
/// trie.
/// `storage_key` is the hash of the desired storage key, meaning
/// this will only work correctly under a secure trie.
/// `storage_key` == keccak(key)
pub fn prove_storage(
    acc: &BasicAccount,
    data: &AsHashDB,
    storage_key: B256,
) -> Result<(Vec<Vec<u8>>, B256), BlockchainError> {
    let data: &dyn HashDB<_, _> = data.deref();
    let mut recorder = Recorder::new();
    let trie = RefTrieDB::new(&data, &acc.storage_root.0)
        .map_err(|err| BlockchainError::TrieError(err.to_string()))
        .unwrap();

    let item: U256 = {
        let decode_value = |bytes: &[u8]| rlp::decode(bytes).expect("decoding db value failed");
        let query = (&mut recorder, decode_value);
        trie.get_with(storage_key.to_ethers().as_bytes(), query)
            .map_err(|err| BlockchainError::TrieError(err.to_string()))?
            .unwrap_or_else(|| U256::ZERO.to_ethers())
            .to_alloy()
    };

    Ok((recorder.drain().into_iter().map(|r| r.data).collect(), B256::from(item)))
}
