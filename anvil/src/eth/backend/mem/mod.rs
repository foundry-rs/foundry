//! In memory blockchain backend
use crate::{
    eth::{
        backend::{
            cheats,
            cheats::CheatsManager,
            db::{AsHashDB, Db, MaybeHashDatabase, SerializableState},
            executor::{ExecutedTransactions, TransactionExecutor},
            fork::ClientFork,
            genesis::GenesisConfig,
            notifications::{NewBlockNotification, NewBlockNotifications},
            time::{utc_from_secs, TimeManager},
            validate::TransactionValidator,
        },
        error::{BlockchainError, InvalidTransactionError},
        fees::{FeeDetails, FeeManager},
        macros::node_info,
        pool::transactions::PoolTransaction,
        util::get_precompiles_for,
    },
    mem::{
        inspector::Inspector,
        storage::{BlockchainStorage, InMemoryBlockStates, MinedBlockOutcome},
    },
    revm::{db::DatabaseRef, AccountInfo},
};
use anvil_core::{
    eth::{
        block::{Block, BlockInfo, Header},
        proof::{AccountProof, BasicAccount, StorageProof},
        receipt::{EIP658Receipt, TypedReceipt},
        state::StateOverride,
        transaction::{
            EthTransactionRequest, PendingTransaction, TransactionInfo, TypedTransaction,
        },
        trie::RefTrieDB,
        utils::to_access_list,
    },
    types::{Forking, Index},
};
use anvil_rpc::error::RpcError;
use ethers::{
    abi::ethereum_types::BigEndianHash,
    prelude::{BlockNumber, TxHash, H256, U256, U64},
    types::{
        transaction::eip2930::AccessList, Address, Block as EthersBlock, BlockId, Bytes, Filter,
        FilteredParams, GethDebugTracingOptions, GethTrace, Log, Trace, Transaction,
        TransactionReceipt,
    },
    utils::{get_contract_address, keccak256, rlp},
};
use forge::{
    executor::inspector::AccessListTracer,
    revm::{return_ok, return_revert, BlockEnv, ExecutionResult, Return},
};
use foundry_evm::{
    decode::decode_revert,
    executor::backend::{DatabaseError, DatabaseResult},
    revm,
    revm::{
        db::CacheDB, Account, CreateScheme, Env, SpecId, TransactOut, TransactTo, TxEnv,
        KECCAK_EMPTY,
    },
    utils::u256_to_h256_be,
};
use futures::channel::mpsc::{unbounded, UnboundedSender};
use hash_db::HashDB;
use parking_lot::{Mutex, RwLock};
use std::{collections::HashMap, ops::Deref, sync::Arc};
use storage::{Blockchain, MinedTransaction};
use tokio::sync::RwLock as AsyncRwLock;
use tracing::{trace, warn};
use trie_db::{Recorder, Trie};

pub mod cache;
pub mod fork_db;
pub mod in_memory_db;
pub mod inspector;
pub mod state;
pub mod storage;

pub type State = foundry_evm::HashMap<Address, Account>;

/// A block request, which includes the Pool Transactions if it's Pending
#[derive(Debug)]
pub enum BlockRequest {
    Pending(Vec<Arc<PoolTransaction>>),
    Number(U64),
}

impl BlockRequest {
    pub fn block_number(&self) -> BlockNumber {
        match self {
            BlockRequest::Pending(_) => BlockNumber::Pending,
            BlockRequest::Number(n) => BlockNumber::Number(*n),
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
    db: Arc<AsyncRwLock<dyn Db>>,
    /// stores all block related data in memory
    blockchain: Blockchain,
    /// Historic states of previous blocks
    states: Arc<RwLock<InMemoryBlockStates>>,
    /// env data of the chain
    env: Arc<RwLock<Env>>,
    /// this is set if this is currently forked off another client
    fork: Option<ClientFork>,
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
    active_snapshots: Arc<Mutex<HashMap<U256, (u64, H256)>>>,
    enable_steps_tracing: bool,
    /// Whether to keep history state
    prune_history: bool,
}

impl Backend {
    /// Initialises the balance of the given accounts
    pub async fn with_genesis(
        db: Arc<AsyncRwLock<dyn Db>>,
        env: Arc<RwLock<Env>>,
        genesis: GenesisConfig,
        fees: FeeManager,
        fork: Option<ClientFork>,
        enable_steps_tracing: bool,
        prune_history: bool,
    ) -> Self {
        // if this is a fork then adjust the blockchain storage
        let blockchain = if let Some(ref fork) = fork {
            trace!(target: "backend", "using forked blockchain at {}", fork.block_number());
            Blockchain::forked(fork.block_number(), fork.block_hash(), fork.total_difficulty())
        } else {
            Blockchain::new(
                &env.read(),
                fees.is_eip1559().then(|| fees.base_fee()),
                genesis.timestamp,
            )
        };

        let start_timestamp =
            if let Some(fork) = fork.as_ref() { fork.timestamp() } else { genesis.timestamp };
        let backend = Self {
            db,
            blockchain,
            states: Arc::new(RwLock::new(Default::default())),
            env,
            fork,
            time: TimeManager::new(start_timestamp),
            cheats: Default::default(),
            new_block_listeners: Default::default(),
            fees,
            genesis,
            active_snapshots: Arc::new(Mutex::new(Default::default())),
            enable_steps_tracing,
            prune_history,
        };

        // Note: this can only fail in forking mode, in which case we can't recover
        backend.apply_genesis().await.expect("Failed to create genesis");
        backend
    }

    /// Applies the configured genesis settings
    ///
    /// This will fund, create the genesis accounts
    async fn apply_genesis(&self) -> DatabaseResult<()> {
        trace!(target: "backend", "setting genesis balances");
        let mut db = self.db.write().await;

        if self.fork.is_some() {
            // in fork mode we only set the balance, this way the accountinfo is fetched from the
            // remote client, preserving code and nonce. The reason for that is private keys for dev
            // accounts are commonly known and are used on testnets
            let mut fork_genesis_infos = self.genesis.fork_genesis_account_infos.lock();
            fork_genesis_infos.clear();

            for address in self.genesis.accounts.iter().copied() {
                let mut info = db.basic(address)?.unwrap_or_default();
                info.balance = self.genesis.balance;
                db.insert_account(address, info.clone());

                // store the fetched AccountInfo, so we can cheaply reset in [Self::reset_fork()]
                fork_genesis_infos.push(info);
            }
        } else {
            for (account, info) in self.genesis.account_infos() {
                db.insert_account(account, info);
            }
        }

        // apply the genesis.json alloc
        self.genesis.apply_genesis_json_alloc(db)?;
        Ok(())
    }

    /// Sets the account to impersonate
    ///
    /// Returns `true` if the account is already impersonated
    pub async fn impersonate(&self, addr: Address) -> DatabaseResult<bool> {
        if self.cheats.is_impersonated(addr) {
            return Ok(true)
        }
        // need to bypass EIP-3607: Reject transactions from senders with deployed code by setting
        // the code hash to `KECCAK_EMPTY` temporarily and also remove the code itself and add back
        // when we stop impersonating
        let mut account = self.get_account(addr).await?;
        let mut code_hash = None;
        let mut code = None;
        if account.code_hash != KECCAK_EMPTY {
            code_hash = Some(std::mem::replace(&mut account.code_hash, KECCAK_EMPTY));
            code = account.code.take();
            self.db.write().await.insert_account(addr, account);
        }
        Ok(self.cheats.impersonate(addr, code_hash, code))
    }

    /// Removes the account that from the impersonated set
    ///
    /// If the impersonated `addr` is a contract then we also reset the code here
    pub async fn stop_impersonating(&self, addr: Address) -> DatabaseResult<()> {
        if let Some((Some(code_hash), code)) = self.cheats.stop_impersonating(&addr) {
            let mut db = self.db.write().await;
            let mut account = db.basic(addr)?.unwrap_or_default();
            account.code_hash = code_hash;
            account.code = code;
            db.insert_account(addr, account)
        }
        Ok(())
    }

    /// Returns the configured fork, if any
    pub fn get_fork(&self) -> Option<&ClientFork> {
        self.fork.as_ref()
    }

    /// Returns the database
    pub fn get_db(&self) -> &Arc<AsyncRwLock<dyn Db>> {
        &self.db
    }

    /// Returns the `AccountInfo` from the database
    pub async fn get_account(&self, address: Address) -> DatabaseResult<AccountInfo> {
        Ok(self.db.read().await.basic(address)?.unwrap_or_default())
    }

    /// Whether we're forked off some remote client
    pub fn is_fork(&self) -> bool {
        self.fork.is_some()
    }

    pub fn precompiles(&self) -> Vec<Address> {
        get_precompiles_for(self.env().read().cfg.spec_id)
    }

    /// Resets the fork to a fresh state
    pub async fn reset_fork(&self, forking: Forking) -> Result<(), BlockchainError> {
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
                let mut env = self.env.write();
                env.cfg.chain_id = fork.chain_id().into();

                env.block = BlockEnv {
                    number: fork_block_number.into(),
                    timestamp: fork_block.timestamp,
                    gas_limit: fork_block.gas_limit,
                    difficulty: fork_block.difficulty,
                    // Keep previous `coinbase` and `basefee` value
                    coinbase: env.block.coinbase,
                    basefee: env.block.basefee,
                };

                self.time.reset(env.block.timestamp.as_u64());
                self.fees.set_base_fee(env.block.basefee);

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
    pub fn best_hash(&self) -> H256 {
        self.blockchain.storage.read().best_hash
    }

    /// Returns the current best number of the chain
    pub fn best_number(&self) -> U64 {
        let num: u64 = self.env.read().block.number.try_into().unwrap_or(u64::MAX);
        num.into()
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
        self.env.read().cfg.chain_id
    }

    /// Returns balance of the given account.
    pub async fn current_balance(&self, address: Address) -> DatabaseResult<U256> {
        Ok(self.get_account(address).await?.balance)
    }

    /// Returns balance of the given account.
    pub async fn current_nonce(&self, address: Address) -> DatabaseResult<U256> {
        Ok(self.get_account(address).await?.nonce.into())
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
        self.db.write().await.set_code(address, code)
    }

    /// Sets the value for the given slot of the given address
    pub async fn set_storage_at(
        &self,
        address: Address,
        slot: U256,
        val: H256,
    ) -> DatabaseResult<()> {
        self.db.write().await.set_storage_at(address, slot, val.into_uint())
    }

    /// Returns true for post London
    pub fn is_eip1559(&self) -> bool {
        (self.env().read().cfg.spec_id as u8) >= (SpecId::LONDON as u8)
    }

    /// Returns the block gas limit
    pub fn gas_limit(&self) -> U256 {
        self.env().read().block.gas_limit
    }

    /// Sets the block gas limit
    pub fn set_gas_limit(&self, gas_limit: U256) {
        self.env().write().block.gas_limit = gas_limit;
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
        let num = self.best_number().as_u64();
        let hash = self.best_hash();
        let id = self.db.write().await.snapshot();
        trace!(target: "backend", "creating snapshot {} at {}", id, num);
        self.active_snapshots.lock().insert(id, (num, hash));
        id
    }

    /// Reverts the state to the snapshot
    pub async fn revert_snapshot(&self, id: U256) -> Result<bool, BlockchainError> {
        let block = { self.active_snapshots.lock().remove(&id) };
        if let Some((num, hash)) = block {
            let best_block_hash = {
                // revert the storage that's newer than the snapshot
                let current_height = self.best_number().as_u64();
                let mut storage = self.blockchain.storage.write();

                for n in ((num + 1)..=current_height).rev() {
                    trace!(target: "backend", "reverting block {}", n);
                    let n: U64 = n.into();
                    if let Some(hash) = storage.hashes.remove(&n) {
                        if let Some(block) = storage.blocks.remove(&hash) {
                            for tx in block.transactions {
                                let _ = storage.transactions.remove(&tx.hash());
                            }
                        }
                    }
                }

                storage.best_number = num.into();
                storage.best_hash = hash;
                hash
            };
            let block =
                self.block_by_hash(best_block_hash).await?.ok_or(BlockchainError::BlockNotFound)?;
            self.time.reset(block.timestamp.as_u64());
            self.set_block_number(num.into());
        }
        Ok(self.db.write().await.revert(id))
    }

    /// Write all chain data to serialized bytes buffer
    pub async fn dump_state(&self) -> Result<Bytes, BlockchainError> {
        self.db
            .read()
            .await
            .dump_state()?
            .map(|s| serde_json::to_vec(&s).unwrap_or_default().into())
            .ok_or_else(|| {
                RpcError::invalid_params(
                    "Dumping state not supported with the current configuration",
                )
                .into()
            })
    }

    /// Deserialize and add all chain data to the backend storage
    pub async fn load_state(&self, buf: Bytes) -> Result<bool, BlockchainError> {
        let state: SerializableState =
            serde_json::from_slice(&buf.0).map_err(|_| BlockchainError::FailedToDecodeStateDump)?;

        if !self.db.write().await.load_state(state)? {
            Err(RpcError::invalid_params(
                "Loading state not supported with the current configuration",
            )
            .into())
        } else {
            Ok(true)
        }
    }

    /// Returns the environment for the next block
    fn next_env(&self) -> Env {
        let mut env = self.env.read().clone();
        // increase block number for this block
        env.block.number = env.block.number.saturating_add(U256::one());
        env.block.basefee = self.base_fee();
        env.block.timestamp = self.time.current_call_timestamp().into();
        env
    }

    /// executes the transactions without writing to the underlying database
    pub async fn inspect_tx(
        &self,
        tx: Arc<PoolTransaction>,
    ) -> (Return, TransactOut, u64, State, Vec<revm::Log>) {
        let mut env = self.next_env();
        env.tx = tx.pending_transaction.to_revm_tx_env();
        let db = self.db.read().await;
        let mut inspector = Inspector::default();

        let mut evm = revm::EVM::new();
        evm.env = env;
        evm.database(&*db);
        let (ExecutionResult { exit_reason, out, gas_used, logs, .. }, state) =
            evm.inspect_ref(&mut inspector);
        inspector.print_logs();
        (exit_reason, out, gas_used, state, logs)
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
            gas_used: U256::zero(),
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
            // increase block number for this block
            env.block.number = env.block.number.saturating_add(U256::one());
            env.block.basefee = current_base_fee;
            env.block.timestamp = self.time.next_timestamp().into();

            let best_hash = self.blockchain.storage.read().best_hash;

            if !self.prune_history {
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
                    gas_used: U256::zero(),
                    enable_steps_tracing: self.enable_steps_tracing,
                };
                let executed_tx = executor.execute();

                // we also need to update the new blockhash in the db itself
                let block_hash = executed_tx.block.block.header.hash();
                db.insert_block_hash(executed_tx.block.block.header.number, block_hash);

                (executed_tx, block_hash)
            };

            // create the new block with the current timestamp
            let ExecutedTransactions { block, included, invalid } = executed_tx;
            let BlockInfo { block, transactions, receipts } = block;

            let header = block.header.clone();
            let block_number: U64 = env.block.number.as_u64().into();

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
            storage.total_difficulty = storage.total_difficulty.saturating_add(header.difficulty);

            storage.blocks.insert(block_hash, block);
            storage.hashes.insert(block_number, block_hash);

            node_info!("");
            // insert all transactions
            for (info, receipt) in transactions.into_iter().zip(receipts) {
                // log some tx info
                {
                    node_info!("    Transaction: {:?}", info.transaction_hash);
                    if let Some(ref contract) = info.contract_address {
                        node_info!("    Contract created: {:?}", contract);
                    }
                    node_info!("    Gas used: {}", receipt.gas_used());
                    match info.exit {
                        return_ok!() => (),
                        return_revert!() => {
                            if let Some(ref r) = info.out {
                                if let Ok(reason) = decode_revert(r.as_ref(), None, None) {
                                    node_info!("    Error: reverted with '{}'", reason);
                                } else {
                                    node_info!("    Error: reverted without a reason string");
                                }
                            } else {
                                node_info!("    Error: reverted without a reason string");
                            }
                        }
                        reason => {
                            node_info!("    Error: failed due to {:?}", reason);
                        }
                    }
                    node_info!("");
                }

                let mined_tx = MinedTransaction {
                    info,
                    receipt,
                    block_hash,
                    block_number: block_number.as_u64(),
                };
                storage.transactions.insert(mined_tx.info.transaction_hash, mined_tx);
            }

            // we intentionally set the difficulty to `0` for newer blocks
            env.block.difficulty = U256::zero();

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
            header.gas_used,
            header.gas_limit,
            header.base_fee_per_gas.unwrap_or_default(),
        );

        // notify all listeners
        self.notify_on_new_block(header, block_hash);

        // update next base fee
        self.fees.set_base_fee(next_block_base_fee.into());

        outcome
    }

    /// Executes the `EthTransactionRequest` without writing to the DB
    ///
    /// # Errors
    ///
    /// Returns an error if the `block_number` is greater than the current height
    pub async fn call(
        &self,
        request: EthTransactionRequest,
        fee_details: FeeDetails,
        block_request: Option<BlockRequest>,
        overrides: Option<StateOverride>,
    ) -> Result<(Return, TransactOut, u64, State), BlockchainError> {
        self.with_database_at(block_request, |state, block| {
            let block_number = block.number.as_u64();
            let (exit, out, gas, state) = match overrides {
                None => self.call_with_state(state, request, fee_details, block),
                Some(overrides) => {
                    let state = state::apply_state_override(overrides, state)?;
                    self.call_with_state(state, request, fee_details, block)
                },
            }?;
            trace!(target: "backend", "call return {:?} out: {:?} gas {} on block {}", exit, out, gas, block_number);
            Ok((exit, out, gas, state))
        }).await?
    }

    fn build_call_env(
        &self,
        request: EthTransactionRequest,
        fee_details: FeeDetails,
        block_env: BlockEnv,
    ) -> Env {
        let EthTransactionRequest { from, to, gas, value, data, nonce, access_list, .. } = request;

        let FeeDetails { gas_price, max_fee_per_gas, max_priority_fee_per_gas } = fee_details;

        let gas_limit = gas.unwrap_or(block_env.gas_limit);
        let mut env = self.env.read().clone();
        env.block = block_env;

        if let Some(base) = max_fee_per_gas {
            env.block.basefee = base;
        }

        let gas_price = gas_price.or(max_fee_per_gas).unwrap_or_else(|| self.gas_price());
        let caller = from.unwrap_or_default();

        env.tx = TxEnv {
            caller,
            gas_limit: gas_limit.as_u64(),
            gas_price,
            gas_priority_fee: max_priority_fee_per_gas,
            transact_to: match to {
                Some(addr) => TransactTo::Call(addr),
                None => TransactTo::Create(CreateScheme::Create),
            },
            value: value.unwrap_or_default(),
            data: data.unwrap_or_default().to_vec().into(),
            chain_id: None,
            nonce: nonce.map(|n| n.as_u64()),
            access_list: to_access_list(access_list.unwrap_or_default()),
        };
        env
    }

    pub fn call_with_state<D>(
        &self,
        state: D,
        request: EthTransactionRequest,
        fee_details: FeeDetails,
        block_env: BlockEnv,
    ) -> Result<(Return, TransactOut, u64, State), BlockchainError>
    where
        D: DatabaseRef<Error = DatabaseError>,
    {
        let mut inspector = Inspector::default();
        let mut evm = revm::EVM::new();
        evm.env = self.build_call_env(request, fee_details, block_env);
        evm.database(state);
        let (ExecutionResult { exit_reason, out, gas_used, .. }, state) =
            evm.inspect_ref(&mut inspector);
        inspector.print_logs();
        Ok((exit_reason, out, gas_used, state))
    }

    pub fn build_access_list_with_state<D>(
        &self,
        state: D,
        request: EthTransactionRequest,
        fee_details: FeeDetails,
        block_env: BlockEnv,
    ) -> Result<(Return, TransactOut, u64, AccessList), BlockchainError>
    where
        D: DatabaseRef<Error = DatabaseError>,
    {
        let from = request.from.unwrap_or_default();
        let to = if let Some(to) = request.to {
            to
        } else {
            let nonce = state.basic(from)?.unwrap_or_default().nonce;
            get_contract_address(from, nonce)
        };

        let mut tracer = AccessListTracer::new(
            AccessList(request.access_list.clone().unwrap_or_default()),
            from,
            to,
            self.precompiles(),
        );

        let mut evm = revm::EVM::new();
        evm.env = self.build_call_env(request, fee_details, block_env);
        evm.database(state);
        let (ExecutionResult { exit_reason, out, gas_used, .. }, _) = evm.inspect_ref(&mut tracer);
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
        hash: H256,
    ) -> Result<Vec<Log>, BlockchainError> {
        if let Some(block) = self.blockchain.storage.read().blocks.get(&hash).cloned() {
            return Ok(self.mined_logs_for_block(filter, block))
        }

        if let Some(fork) = self.get_fork() {
            let filter = filter;
            return Ok(fork.logs(&filter).await?)
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

            for (log_idx, log) in logs.into_iter().enumerate() {
                let mut log = Log {
                    address: log.address,
                    topics: log.topics,
                    data: log.data,
                    block_hash: None,
                    block_number: None,
                    transaction_hash: None,
                    transaction_index: None,
                    log_index: None,
                    transaction_log_index: None,
                    log_type: None,
                    removed: Some(false),
                };
                let mut is_match: bool = true;
                if filter.address.is_some() && filter.has_topics() {
                    if !params.filter_address(&log) || !params.filter_topics(&log) {
                        is_match = false;
                    }
                } else if filter.address.is_some() {
                    if !params.filter_address(&log) {
                        is_match = false;
                    }
                } else if filter.has_topics() && !params.filter_topics(&log) {
                    is_match = false;
                }

                if is_match {
                    log.block_hash = Some(block_hash);
                    log.block_number = Some(block.header.number.as_u64().into());
                    log.transaction_hash = Some(transaction_hash);
                    log.transaction_index = Some(transaction.transaction_index.into());
                    log.log_index = Some(U256::from(block_log_index));
                    log.transaction_log_index = Some(U256::from(log_idx));
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
                all_logs = fork.logs(&filter).await?;

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
            let best = self.best_number().as_u64();
            let to_block =
                self.convert_block_number(filter.block_option.get_to_block().copied()).min(best);
            let from_block =
                self.convert_block_number(filter.block_option.get_from_block().copied()).min(best);
            self.logs_for_range(&filter, from_block, to_block).await
        }
    }

    pub async fn block_by_hash(
        &self,
        hash: H256,
    ) -> Result<Option<EthersBlock<TxHash>>, BlockchainError> {
        trace!(target: "backend", "get block by hash {:?}", hash);
        if let tx @ Some(_) = self.mined_block_by_hash(hash) {
            return Ok(tx)
        }

        if let Some(fork) = self.get_fork() {
            return Ok(fork.block_by_hash(hash).await?)
        }

        Ok(None)
    }

    pub async fn block_by_hash_full(
        &self,
        hash: H256,
    ) -> Result<Option<EthersBlock<Transaction>>, BlockchainError> {
        trace!(target: "backend", "get block by hash {:?}", hash);
        if let tx @ Some(_) = self.get_full_block(hash) {
            return Ok(tx)
        }

        if let Some(fork) = self.get_fork() {
            return Ok(fork.block_by_hash_full(hash).await?)
        }

        Ok(None)
    }

    fn mined_block_by_hash(&self, hash: H256) -> Option<EthersBlock<TxHash>> {
        let block = self.blockchain.storage.read().blocks.get(&hash)?.clone();
        Some(self.convert_block(block))
    }

    /// Returns all transactions given a block
    fn mined_transactions_in_block(&self, block: &Block) -> Option<Vec<Transaction>> {
        let mut transactions = Vec::with_capacity(block.transactions.len());
        let base_fee = block.header.base_fee_per_gas;
        let storage = self.blockchain.storage.read();
        for hash in block.transactions.iter().map(|tx| tx.hash()) {
            let info = storage.transactions.get(&hash)?.info.clone();
            let tx = block.transactions.get(info.transaction_index as usize)?.clone();

            let tx = transaction_build(tx, Some(block), Some(info), true, base_fee);
            transactions.push(tx);
        }
        Some(transactions)
    }

    pub async fn block_by_number(
        &self,
        number: BlockNumber,
    ) -> Result<Option<EthersBlock<TxHash>>, BlockchainError> {
        trace!(target: "backend", "get block by number {:?}", number);
        if let tx @ Some(_) = self.mined_block_by_number(number) {
            return Ok(tx)
        }

        if let Some(fork) = self.get_fork() {
            let number = self.convert_block_number(Some(number));
            if fork.predates_fork_inclusive(number) {
                return Ok(fork.block_by_number(number).await?)
            }
        }

        Ok(None)
    }

    pub async fn block_by_number_full(
        &self,
        number: BlockNumber,
    ) -> Result<Option<EthersBlock<Transaction>>, BlockchainError> {
        trace!(target: "backend", "get block by number {:?}", number);
        if let tx @ Some(_) = self.get_full_block(number) {
            return Ok(tx)
        }

        if let Some(fork) = self.get_fork() {
            let number = self.convert_block_number(Some(number));
            if fork.predates_fork_inclusive(number) {
                return Ok(fork.block_by_number_full(number).await?)
            }
        }

        Ok(None)
    }

    pub fn get_block(&self, id: impl Into<BlockId>) -> Option<Block> {
        let hash = match id.into() {
            BlockId::Hash(hash) => hash,
            BlockId::Number(number) => {
                let storage = self.blockchain.storage.read();
                match number {
                    BlockNumber::Latest => storage.best_hash,
                    BlockNumber::Earliest => storage.genesis_hash,
                    BlockNumber::Pending => return None,
                    BlockNumber::Number(num) => *storage.hashes.get(&num)?,
                }
            }
        };
        self.get_block_by_hash(hash)
    }

    pub fn get_block_by_hash(&self, hash: H256) -> Option<Block> {
        self.blockchain.storage.read().blocks.get(&hash).cloned()
    }

    fn mined_block_by_number(&self, number: BlockNumber) -> Option<EthersBlock<TxHash>> {
        Some(self.convert_block(self.get_block(number)?))
    }

    pub fn get_full_block(&self, id: impl Into<BlockId>) -> Option<EthersBlock<Transaction>> {
        let block = self.get_block(id)?;
        let transactions = self.mined_transactions_in_block(&block)?;
        let block = self.convert_block(block);
        Some(block.into_full_block(transactions))
    }

    /// Takes a block as it's stored internally and returns the eth api conform block format
    pub fn convert_block(&self, block: Block) -> EthersBlock<TxHash> {
        let size = U256::from(rlp::encode(&block).len() as u32);

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
        } = header;

        EthersBlock {
            hash: Some(hash),
            parent_hash,
            uncles_hash: ommers_hash,
            author: Some(beneficiary),
            state_root,
            transactions_root,
            receipts_root,
            number: Some(number.as_u64().into()),
            gas_used,
            gas_limit,
            extra_data,
            logs_bloom: Some(logs_bloom),
            timestamp: timestamp.into(),
            difficulty,
            total_difficulty: Some(self.total_difficulty()),
            seal_fields: { vec![mix_hash.as_bytes().to_vec().into(), nonce.0.to_vec().into()] },
            uncles: vec![],
            transactions: transactions.into_iter().map(|tx| tx.hash()).collect(),
            size: Some(size),
            mix_hash: Some(mix_hash),
            nonce: Some(nonce),
            base_fee_per_gas,
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
        let current = self.best_number().as_u64();
        let requested =
            match block_id.map(Into::into).unwrap_or(BlockId::Number(BlockNumber::Latest)) {
                BlockId::Hash(hash) => self
                    .block_by_hash(hash)
                    .await?
                    .ok_or(BlockchainError::BlockNotFound)?
                    .number
                    .ok_or(BlockchainError::BlockNotFound)?
                    .as_u64(),
                BlockId::Number(num) => match num {
                    BlockNumber::Latest | BlockNumber::Pending => self.best_number().as_u64(),
                    BlockNumber::Earliest => 0,
                    BlockNumber::Number(num) => num.as_u64(),
                },
            };

        if requested > current {
            Err(BlockchainError::BlockOutOfRange(current, requested))
        } else {
            Ok(requested)
        }
    }

    pub fn convert_block_number(&self, block: Option<BlockNumber>) -> u64 {
        match block.unwrap_or(BlockNumber::Latest) {
            BlockNumber::Latest | BlockNumber::Pending => self.best_number().as_u64(),
            BlockNumber::Earliest => 0,
            BlockNumber::Number(num) => num.as_u64(),
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
                            number: block.header.number,
                            coinbase: block.header.beneficiary,
                            timestamp: block.header.timestamp.into(),
                            difficulty: block.header.difficulty,
                            basefee: block.header.base_fee_per_gas.unwrap_or_default(),
                            gas_limit: block.header.gas_limit,
                        };
                        f(state, block)
                    })
                    .await;
                return Ok(result)
            }
            Some(BlockRequest::Number(bn)) => Some(BlockNumber::Number(bn)),
            None => None,
        };
        let block_number: U256 = self.convert_block_number(block_number).into();

        if block_number < self.env.read().block.number {
            let mut states = self.states.write();
            return if let Some((state, block)) = self
                .get_block(block_number.as_u64())
                .and_then(|block| Some((states.get(&block.header.hash())?, block)))
            {
                let block = BlockEnv {
                    number: block.header.number,
                    coinbase: block.header.beneficiary,
                    timestamp: block.header.timestamp.into(),
                    difficulty: block.header.difficulty,
                    basefee: block.header.base_fee_per_gas.unwrap_or_default(),
                    gas_limit: block.header.gas_limit,
                };
                Ok(f(Box::new(state), block))
            } else {
                warn!(target: "backend", "Not historic state found for block={}", block_number);
                Err(BlockchainError::BlockOutOfRange(
                    self.env.read().block.number.as_u64(),
                    block_number.as_u64(),
                ))
            }
        }
        let db = self.db.read().await;
        let block = self.env().read().block.clone();
        Ok(f(Box::new(&*db), block))
    }

    pub async fn storage_at(
        &self,
        address: Address,
        index: U256,
        block_request: Option<BlockRequest>,
    ) -> Result<H256, BlockchainError> {
        self.with_database_at(block_request, |db, _| {
            trace!(target: "backend", "get storage for {:?} at {:?}", address, index);
            let val = db.storage(address, index)?;
            Ok(u256_to_h256_be(val))
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
        let account = state.basic(address)?.unwrap_or_default();
        if account.code_hash == KECCAK_EMPTY {
            // if the code hash is `KECCAK_EMPTY`, we check no further
            return Ok(Default::default())
        }
        let code = if let Some(code) = account.code {
            code
        } else {
            state.code_by_hash(account.code_hash)?
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
        Ok(state.basic(address)?.unwrap_or_default().balance)
    }

    /// Returns the nonce of the address
    ///
    /// If the requested number predates the fork then this will fetch it from the endpoint
    pub async fn get_nonce(
        &self,
        address: Address,
        block_request: Option<BlockRequest>,
    ) -> Result<U256, BlockchainError> {
        self.with_database_at(block_request, |db, _| {
            trace!(target: "backend", "get nonce for {:?}", address);
            Ok(db.basic(address)?.unwrap_or_default().nonce.into())
        })
        .await?
    }

    /// Returns the traces for the given transaction
    pub async fn trace_transaction(&self, hash: H256) -> Result<Vec<Trace>, BlockchainError> {
        if let Some(traces) = self.mined_parity_trace_transaction(hash) {
            return Ok(traces)
        }

        if let Some(fork) = self.get_fork() {
            return Ok(fork.trace_transaction(hash).await?)
        }

        Ok(vec![])
    }

    /// Returns the traces for the given transaction
    fn mined_parity_trace_transaction(&self, hash: H256) -> Option<Vec<Trace>> {
        self.blockchain.storage.read().transactions.get(&hash).map(|tx| tx.parity_traces())
    }

    /// Returns the traces for the given transaction
    fn mined_parity_trace_block(&self, block: u64) -> Option<Vec<Trace>> {
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
        hash: H256,
        opts: GethDebugTracingOptions,
    ) -> Result<GethTrace, BlockchainError> {
        if let Some(traces) = self.mined_geth_trace_transaction(hash, opts.clone()) {
            return Ok(traces)
        }

        if let Some(fork) = self.get_fork() {
            return Ok(fork.debug_trace_transaction(hash, opts).await?)
        }

        Ok(GethTrace::default())
    }

    fn mined_geth_trace_transaction(
        &self,
        hash: H256,
        opts: GethDebugTracingOptions,
    ) -> Option<GethTrace> {
        self.blockchain.storage.read().transactions.get(&hash).map(|tx| tx.geth_trace(opts))
    }

    /// Returns the traces for the given block
    pub async fn trace_block(&self, block: BlockNumber) -> Result<Vec<Trace>, BlockchainError> {
        let number = self.convert_block_number(Some(block));
        if let Some(traces) = self.mined_parity_trace_block(number) {
            return Ok(traces)
        }

        if let Some(fork) = self.get_fork() {
            if fork.predates_fork(number) {
                return Ok(fork.trace_block(number).await?)
            }
        }

        Ok(vec![])
    }

    pub async fn transaction_receipt(
        &self,
        hash: H256,
    ) -> Result<Option<TransactionReceipt>, BlockchainError> {
        if let tx @ Some(_) = self.mined_transaction_receipt(hash) {
            return Ok(tx)
        }

        if let Some(fork) = self.get_fork() {
            return Ok(fork.transaction_receipt(hash).await?)
        }

        Ok(None)
    }

    /// Returns all receipts of the block
    pub fn mined_receipts(&self, hash: H256) -> Option<Vec<TypedReceipt>> {
        let block = self.mined_block_by_hash(hash)?;
        let mut receipts = Vec::new();
        let storage = self.blockchain.storage.read();
        for tx in block.transactions {
            let receipt = storage.transactions.get(&tx)?.receipt.clone();
            receipts.push(receipt);
        }
        Some(receipts)
    }

    /// Returns the transaction receipt for the given hash
    fn mined_transaction_receipt(&self, hash: H256) -> Option<TransactionReceipt> {
        let MinedTransaction { info, receipt, block_hash, .. } =
            self.blockchain.storage.read().transactions.get(&hash)?.clone();

        let EIP658Receipt { status_code, gas_used, logs_bloom, logs } = receipt.into();

        let index = info.transaction_index as usize;

        let block = self.blockchain.storage.read().blocks.get(&block_hash).cloned()?;

        // TODO store cumulative gas used in receipt instead
        let receipts = self.get_receipts(block.transactions.iter().map(|tx| tx.hash()));

        let mut cumulative_gas_used = U256::zero();
        for receipt in receipts.iter().take(index + 1) {
            cumulative_gas_used = cumulative_gas_used.saturating_add(receipt.gas_used());
        }

        // cumulative_gas_used = cumulative_gas_used.saturating_sub(gas_used);

        let mut cumulative_receipts = receipts;
        cumulative_receipts.truncate(index + 1);

        let transaction = block.transactions[index].clone();

        let effective_gas_price = match transaction {
            TypedTransaction::Legacy(t) => t.gas_price,
            TypedTransaction::EIP2930(t) => t.gas_price,
            TypedTransaction::EIP1559(t) => self
                .base_fee()
                .checked_add(t.max_priority_fee_per_gas)
                .unwrap_or_else(U256::max_value),
        };

        Some(TransactionReceipt {
            transaction_hash: info.transaction_hash,
            transaction_index: info.transaction_index.into(),
            block_hash: Some(block_hash),
            block_number: Some(block.header.number.as_u64().into()),
            from: info.from,
            to: info.to,
            cumulative_gas_used,
            gas_used: Some(gas_used),
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
                        topics: log.topics.clone(),
                        data: log.data.clone(),
                        block_hash: Some(block_hash),
                        block_number: Some(block.header.number.as_u64().into()),
                        transaction_hash: Some(info.transaction_hash),
                        transaction_index: Some(info.transaction_index.into()),
                        log_index: Some(U256::from(
                            (pre_receipts_log_index.unwrap_or(0)) + i as u32,
                        )),
                        transaction_log_index: Some(U256::from(i)),
                        log_type: None,
                        removed: Some(false),
                    })
                    .collect()
            },
            status: Some(status_code.into()),
            root: None,
            logs_bloom,
            transaction_type: None,
            effective_gas_price: Some(effective_gas_price),
        })
    }

    pub async fn transaction_by_block_number_and_index(
        &self,
        number: BlockNumber,
        index: Index,
    ) -> Result<Option<Transaction>, BlockchainError> {
        if let Some(hash) = self.mined_block_by_number(number).and_then(|b| b.hash) {
            return Ok(self.mined_transaction_by_block_hash_and_index(hash, index))
        }

        if let Some(fork) = self.get_fork() {
            let number = self.convert_block_number(Some(number));
            if fork.predates_fork(number) {
                return Ok(fork.transaction_by_block_number_and_index(number, index.into()).await?)
            }
        }

        Ok(None)
    }

    pub async fn transaction_by_block_hash_and_index(
        &self,
        hash: H256,
        index: Index,
    ) -> Result<Option<Transaction>, BlockchainError> {
        if let tx @ Some(_) = self.mined_transaction_by_block_hash_and_index(hash, index) {
            return Ok(tx)
        }

        if let Some(fork) = self.get_fork() {
            return Ok(fork.transaction_by_block_hash_and_index(hash, index.into()).await?)
        }

        Ok(None)
    }

    fn mined_transaction_by_block_hash_and_index(
        &self,
        block_hash: H256,
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

        Some(transaction_build(tx, Some(&block), Some(info), true, block.header.base_fee_per_gas))
    }

    pub async fn transaction_by_hash(
        &self,
        hash: H256,
    ) -> Result<Option<Transaction>, BlockchainError> {
        trace!(target: "backend", "transaction_by_hash={:?}", hash);
        if let tx @ Some(_) = self.mined_transaction_by_hash(hash) {
            return Ok(tx)
        }

        if let Some(fork) = self.get_fork() {
            return Ok(fork.transaction_by_hash(hash).await?)
        }

        Ok(None)
    }

    fn mined_transaction_by_hash(&self, hash: H256) -> Option<Transaction> {
        let (info, block) = {
            let storage = self.blockchain.storage.read_recursive();
            let MinedTransaction { info, block_hash, .. } =
                storage.transactions.get(&hash)?.clone();
            let block = storage.blocks.get(&block_hash).cloned()?;
            (info, block)
        };
        let tx = block.transactions.get(info.transaction_index as usize)?.clone();

        Some(transaction_build(tx, Some(&block), Some(info), true, block.header.base_fee_per_gas))
    }

    /// Prove an account's existence or nonexistence in the state trie.
    ///
    /// Returns a merkle proof of the account's trie node, `account_key` == keccak(address)
    pub async fn prove_account_at(
        &self,
        address: Address,
        values: Vec<H256>,
        block_request: Option<BlockRequest>,
    ) -> Result<AccountProof, BlockchainError> {
        let account_key = H256::from(keccak256(address.as_bytes()));
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
                trie.get_with(account_key.as_bytes(), query)
                    .map_err(|err| BlockchainError::TrieError(err.to_string()))?
            };
            let account = maybe_account.unwrap_or_default();

            let proof =
                recorder.drain().into_iter().map(|r| r.data).map(Into::into).collect::<Vec<_>>();

            let account_db =
                block_db.maybe_account_db(address).ok_or(BlockchainError::DataUnavailable)?;

            let account_proof = AccountProof {
                address,
                balance: account.balance,
                nonce: account.nonce.as_u64().into(),
                code_hash: account.code_hash,
                storage_hash: account.storage_root,
                account_proof: proof,
                storage_proof: values
                    .into_iter()
                    .map(|storage_key| {
                        let key = H256::from(keccak256(storage_key));
                        prove_storage(&account, &account_db.0, key).map(
                            |(storage_proof, storage_value)| StorageProof {
                                key,
                                value: storage_value.into_uint(),
                                proof: storage_proof.into_iter().map(Into::into).collect(),
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
    fn notify_on_new_block(&self, header: Header, hash: H256) {
        // cleanup closed notification streams first, if the channel is closed we can remove the
        // sender half for the set
        self.new_block_listeners.lock().retain(|tx| !tx.is_closed());

        let notification = NewBlockNotification { hash, header: Arc::new(header) };

        self.new_block_listeners
            .lock()
            .retain(|tx| tx.unbounded_send(notification.clone()).is_ok());
    }
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
            if chain_id != tx_chain_id.into() {
                if let Some(legacy) = tx.as_legacy() {
                    // <https://github.com/ethereum/EIPs/blob/master/EIPS/eip-155.md>
                    if env.cfg.spec_id >= SpecId::SPURIOUS_DRAGON &&
                        !legacy.meets_eip155(chain_id.as_u64())
                    {
                        warn!(target: "backend", ?chain_id, ?tx_chain_id, "incompatible EIP155-based V");
                        return Err(InvalidTransactionError::IncompatibleEIP155)
                    }
                } else {
                    warn!(target: "backend", ?chain_id, ?tx_chain_id, "invalid chain id");
                    return Err(InvalidTransactionError::InvalidChainId)
                }
            }
        }

        if tx.gas_limit() > env.block.gas_limit {
            warn!(target: "backend", "[{:?}] gas too high", tx.hash());
            return Err(InvalidTransactionError::GasTooHigh)
        }

        // check nonce
        let nonce: u64 = (*tx.nonce()).try_into().map_err(|_| InvalidTransactionError::NonceMax)?;
        if nonce < account.nonce {
            warn!(target: "backend", "[{:?}] nonce too low", tx.hash());
            return Err(InvalidTransactionError::NonceTooLow)
        }

        if (env.cfg.spec_id as u8) >= (SpecId::LONDON as u8) && tx.gas_price() < env.block.basefee {
            warn!(target: "backend", "max fee per gas={}, too low, block basefee={}",tx.gas_price(),  env.block.basefee);
            return Err(InvalidTransactionError::FeeTooLow)
        }

        let max_cost = tx.max_cost();
        let value = tx.value();
        // check sufficient funds: `gas * price + value`
        let req_funds = max_cost.checked_add(value).ok_or_else(|| {
            warn!(target: "backend", "[{:?}] cost too high",
            tx.hash());
            InvalidTransactionError::Payment
        })?;

        if account.balance < req_funds {
            warn!(target: "backend", "[{:?}] insufficient allowance={}, required={} account={:?}", tx.hash(), account.balance, req_funds, *pending.sender());
            return Err(InvalidTransactionError::Payment)
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
        if tx.nonce().as_u64() > account.nonce {
            return Err(InvalidTransactionError::NonceTooHigh)
        }
        Ok(())
    }
}

/// Creates a `Transaction` as it's expected for the `eth` RPC api from storage data
pub fn transaction_build(
    eth_transaction: TypedTransaction,
    block: Option<&Block>,
    info: Option<TransactionInfo>,
    is_eip1559: bool,
    base_fee: Option<U256>,
) -> Transaction {
    let mut transaction: Transaction = eth_transaction.clone().into();

    if let TypedTransaction::EIP1559(_) = eth_transaction {
        if block.is_none() && info.is_none() {
            // transaction is not mined yet, gas price is considered just `max_fee_per_gas`
            transaction.gas_price = transaction.max_fee_per_gas;
        } else {
            // if transaction is already mined, gas price is considered base fee + priority fee: the
            // effective gas price.
            let base_fee = base_fee.unwrap_or(U256::zero());
            let max_priority_fee_per_gas =
                transaction.max_priority_fee_per_gas.unwrap_or(U256::zero());
            transaction.gas_price = Some(
                base_fee.checked_add(max_priority_fee_per_gas).unwrap_or_else(U256::max_value),
            );
        }
    } else if !is_eip1559 {
        transaction.max_fee_per_gas = None;
        transaction.max_priority_fee_per_gas = None;
        transaction.transaction_type = None;
    }

    transaction.block_hash =
        block.as_ref().map(|block| H256::from(keccak256(&rlp::encode(&block.header))));

    transaction.block_number = block.as_ref().map(|block| block.header.number.as_u64().into());

    transaction.transaction_index = info.as_ref().map(|status| status.transaction_index.into());

    // need to check if the signature of the transaction is the `BYPASS_SIGNATURE`, if so then we
    // can't recover the sender, instead we use the sender from the executed transaction
    if cheats::is_bypassed(&eth_transaction) {
        transaction.from = info.as_ref().map(|info| info.from).unwrap_or_default()
    } else {
        transaction.from = eth_transaction.recover().expect("can recover signed tx");
    }

    transaction.to = info.as_ref().map_or(eth_transaction.to().cloned(), |status| status.to);

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
    storage_key: H256,
) -> Result<(Vec<Vec<u8>>, H256), BlockchainError> {
    let data: &dyn HashDB<_, _> = data.deref();
    let mut recorder = Recorder::new();
    let trie = RefTrieDB::new(&data, &acc.storage_root.0)
        .map_err(|err| BlockchainError::TrieError(err.to_string()))
        .unwrap();

    let item: U256 = {
        let decode_value = |bytes: &[u8]| rlp::decode(bytes).expect("decoding db value failed");
        let query = (&mut recorder, decode_value);
        trie.get_with(storage_key.as_bytes(), query)
            .map_err(|err| BlockchainError::TrieError(err.to_string()))?
            .unwrap_or_else(U256::zero)
    };

    Ok((recorder.drain().into_iter().map(|r| r.data).collect(), BigEndianHash::from_uint(&item)))
}
