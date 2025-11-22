use crate::eth::backend::{
    db::{
        Db, MaybeForkedDatabase, MaybeFullDatabase, SerializableAccountRecord, SerializableBlock,
        SerializableHistoricalStates, SerializableState, SerializableTransaction, StateDb,
    },
    mem::offline_fork_db::OfflineForkedDatabase,
};
use alloy_primitives::{Address, B256, U256, map::HashMap};
use alloy_rpc_types::BlockId;
use foundry_evm::{
    backend::{
        BlockchainDb, DatabaseError, DatabaseResult, RevertStateSnapshotAction, StateSnapshot,
    },
    fork::database::ForkDbStateSnapshot,
};
use revm::{
    context::BlockEnv,
    database::{Database, DatabaseRef, DbAccount},
    state::AccountInfo,
};

pub use foundry_evm::fork::database::ForkedDatabase;

/// An enum that can hold either a regular ForkedDatabase or an OfflineForkedDatabase
#[derive(Clone, Debug)]
pub enum MaybeOfflineForkedDatabase {
    Online(ForkedDatabase),
    Offline(OfflineForkedDatabase),
}

impl MaybeOfflineForkedDatabase {
    pub fn online(db: ForkedDatabase) -> Self {
        Self::Online(db)
    }

    pub fn offline(db: OfflineForkedDatabase) -> Self {
        Self::Offline(db)
    }
}

// Delegate all trait implementations to the inner type
impl Database for MaybeOfflineForkedDatabase {
    type Error = DatabaseError;

    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        match self {
            Self::Online(db) => db.basic(address),
            Self::Offline(db) => db.basic(address),
        }
    }

    fn code_by_hash(&mut self, code_hash: B256) -> Result<revm::bytecode::Bytecode, Self::Error> {
        match self {
            Self::Online(db) => db.code_by_hash(code_hash),
            Self::Offline(db) => db.code_by_hash(code_hash),
        }
    }

    fn storage(&mut self, address: Address, index: U256) -> Result<U256, Self::Error> {
        match self {
            Self::Online(db) => db.storage(address, index),
            Self::Offline(db) => db.storage(address, index),
        }
    }

    fn block_hash(&mut self, number: u64) -> Result<B256, Self::Error> {
        match self {
            Self::Online(db) => db.block_hash(number),
            Self::Offline(db) => db.block_hash(number),
        }
    }
}

impl DatabaseRef for MaybeOfflineForkedDatabase {
    type Error = DatabaseError;

    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        match self {
            Self::Online(db) => db.basic_ref(address),
            Self::Offline(db) => db.basic_ref(address),
        }
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> Result<revm::bytecode::Bytecode, Self::Error> {
        match self {
            Self::Online(db) => db.code_by_hash_ref(code_hash),
            Self::Offline(db) => db.code_by_hash_ref(code_hash),
        }
    }

    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        match self {
            Self::Online(db) => db.storage_ref(address, index),
            Self::Offline(db) => db.storage_ref(address, index),
        }
    }

    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        match self {
            Self::Online(db) => db.block_hash_ref(number),
            Self::Offline(db) => db.block_hash_ref(number),
        }
    }
}

impl revm::DatabaseCommit for MaybeOfflineForkedDatabase {
    fn commit(&mut self, changes: HashMap<Address, revm::state::Account>) {
        match self {
            Self::Online(db) => db.commit(changes),
            Self::Offline(db) => db.commit(changes),
        }
    }
}

impl MaybeFullDatabase for MaybeOfflineForkedDatabase {
    fn maybe_as_full_db(&self) -> Option<&HashMap<Address, DbAccount>> {
        match self {
            Self::Online(db) => db.maybe_as_full_db(),
            Self::Offline(db) => db.maybe_as_full_db(),
        }
    }

    fn clear_into_state_snapshot(&mut self) -> StateSnapshot {
        match self {
            Self::Online(db) => db.clear_into_state_snapshot(),
            Self::Offline(db) => db.clear_into_state_snapshot(),
        }
    }

    fn read_as_state_snapshot(&self) -> StateSnapshot {
        match self {
            Self::Online(db) => db.read_as_state_snapshot(),
            Self::Offline(db) => db.read_as_state_snapshot(),
        }
    }

    fn clear(&mut self) {
        match self {
            Self::Online(db) => db.clear(),
            Self::Offline(db) => db.clear(),
        }
    }

    fn init_from_state_snapshot(&mut self, state_snapshot: StateSnapshot) {
        match self {
            Self::Online(db) => db.init_from_state_snapshot(state_snapshot),
            Self::Offline(db) => db.init_from_state_snapshot(state_snapshot),
        }
    }
}

impl MaybeForkedDatabase for MaybeOfflineForkedDatabase {
    fn maybe_reset(&mut self, url: Option<String>, block_number: BlockId) -> Result<(), String> {
        match self {
            Self::Online(db) => db.maybe_reset(url, block_number),
            Self::Offline(db) => db.maybe_reset(url, block_number),
        }
    }

    fn maybe_flush_cache(&self) -> Result<(), String> {
        match self {
            Self::Online(db) => db.maybe_flush_cache(),
            Self::Offline(db) => db.maybe_flush_cache(),
        }
    }

    fn maybe_inner(&self) -> Result<&BlockchainDb, String> {
        match self {
            Self::Online(db) => db.maybe_inner(),
            Self::Offline(db) => db.maybe_inner(),
        }
    }
}

impl Db for MaybeOfflineForkedDatabase {
    fn insert_account(&mut self, address: Address, account: AccountInfo) {
        match self {
            Self::Online(db) => db.insert_account(address, account),
            Self::Offline(db) => db.insert_account(address, account),
        }
    }

    fn set_storage_at(&mut self, address: Address, slot: B256, val: B256) -> DatabaseResult<()> {
        match self {
            Self::Online(db) => db.set_storage_at(address, slot, val),
            Self::Offline(db) => db.set_storage_at(address, slot, val),
        }
    }

    fn insert_block_hash(&mut self, number: U256, hash: B256) {
        match self {
            Self::Online(db) => db.insert_block_hash(number, hash),
            Self::Offline(db) => db.insert_block_hash(number, hash),
        }
    }

    fn dump_state(
        &self,
        at: BlockEnv,
        best_number: u64,
        blocks: Vec<SerializableBlock>,
        transactions: Vec<SerializableTransaction>,
        historical_states: Option<SerializableHistoricalStates>,
    ) -> DatabaseResult<Option<SerializableState>> {
        match self {
            Self::Online(db) => {
                db.dump_state(at, best_number, blocks, transactions, historical_states)
            }
            Self::Offline(db) => {
                db.dump_state(at, best_number, blocks, transactions, historical_states)
            }
        }
    }

    fn snapshot_state(&mut self) -> U256 {
        match self {
            Self::Online(db) => db.insert_state_snapshot(),
            Self::Offline(db) => db.inner_mut().insert_state_snapshot(),
        }
    }

    fn revert_state(&mut self, id: U256, action: RevertStateSnapshotAction) -> bool {
        match self {
            Self::Online(db) => db.revert_state(id, action),
            Self::Offline(db) => db.revert_state(id, action),
        }
    }

    fn current_state(&self) -> StateDb {
        match self {
            Self::Online(db) => db.current_state(),
            Self::Offline(db) => db.current_state(),
        }
    }
}

impl Db for ForkedDatabase {
    fn insert_account(&mut self, address: Address, account: AccountInfo) {
        self.database_mut().insert_account(address, account)
    }

    fn set_storage_at(&mut self, address: Address, slot: B256, val: B256) -> DatabaseResult<()> {
        // this ensures the account is loaded first
        let _ = Database::basic(self, address)?;
        self.database_mut().set_storage_at(address, slot, val)
    }

    fn insert_block_hash(&mut self, number: U256, hash: B256) {
        self.inner().block_hashes().write().insert(number, hash);
    }

    fn dump_state(
        &self,
        at: BlockEnv,
        best_number: u64,
        blocks: Vec<SerializableBlock>,
        transactions: Vec<SerializableTransaction>,
        historical_states: Option<SerializableHistoricalStates>,
    ) -> DatabaseResult<Option<SerializableState>> {
        let mut db = self.database().clone();
        let accounts = self
            .database()
            .cache
            .accounts
            .clone()
            .into_iter()
            .map(|(k, v)| -> DatabaseResult<_> {
                let code = if let Some(code) = v.info.code {
                    code
                } else {
                    db.code_by_hash(v.info.code_hash)?
                };
                Ok((
                    k,
                    SerializableAccountRecord {
                        nonce: v.info.nonce,
                        balance: v.info.balance,
                        code: code.original_bytes(),
                        storage: v.storage.into_iter().map(|(k, v)| (k.into(), v.into())).collect(),
                    },
                ))
            })
            .collect::<Result<_, _>>()?;
        Ok(Some(SerializableState {
            block: Some(at),
            accounts,
            best_block_number: Some(best_number),
            blocks,
            transactions,
            historical_states,
        }))
    }

    fn snapshot_state(&mut self) -> U256 {
        self.insert_state_snapshot()
    }

    fn revert_state(&mut self, id: U256, action: RevertStateSnapshotAction) -> bool {
        self.revert_state_snapshot(id, action)
    }

    fn current_state(&self) -> StateDb {
        StateDb::new(self.create_state_snapshot())
    }
}

impl MaybeFullDatabase for ForkedDatabase {
    fn maybe_as_full_db(&self) -> Option<&HashMap<Address, DbAccount>> {
        Some(&self.database().cache.accounts)
    }

    fn clear_into_state_snapshot(&mut self) -> StateSnapshot {
        let db = self.inner().db();
        let accounts = std::mem::take(&mut *db.accounts.write());
        let storage = std::mem::take(&mut *db.storage.write());
        let block_hashes = std::mem::take(&mut *db.block_hashes.write());
        StateSnapshot { accounts, storage, block_hashes }
    }

    fn read_as_state_snapshot(&self) -> StateSnapshot {
        let db = self.inner().db();
        let accounts = db.accounts.read().clone();
        let storage = db.storage.read().clone();
        let block_hashes = db.block_hashes.read().clone();
        StateSnapshot { accounts, storage, block_hashes }
    }

    fn clear(&mut self) {
        self.flush_cache();
        self.clear_into_state_snapshot();
    }

    fn init_from_state_snapshot(&mut self, state_snapshot: StateSnapshot) {
        let db = self.inner().db();
        let StateSnapshot { accounts, storage, block_hashes } = state_snapshot;
        *db.accounts.write() = accounts;
        *db.storage.write() = storage;
        *db.block_hashes.write() = block_hashes;
    }
}

impl MaybeFullDatabase for ForkDbStateSnapshot {
    fn maybe_as_full_db(&self) -> Option<&HashMap<Address, DbAccount>> {
        Some(&self.local.cache.accounts)
    }

    fn clear_into_state_snapshot(&mut self) -> StateSnapshot {
        std::mem::take(&mut self.state_snapshot)
    }

    fn read_as_state_snapshot(&self) -> StateSnapshot {
        self.state_snapshot.clone()
    }

    fn clear(&mut self) {
        std::mem::take(&mut self.state_snapshot);
        self.local.clear()
    }

    fn init_from_state_snapshot(&mut self, state_snapshot: StateSnapshot) {
        self.state_snapshot = state_snapshot;
    }
}

impl MaybeForkedDatabase for ForkedDatabase {
    fn maybe_reset(&mut self, url: Option<String>, block_number: BlockId) -> Result<(), String> {
        self.reset(url, block_number)
    }

    fn maybe_flush_cache(&self) -> Result<(), String> {
        self.flush_cache();
        Ok(())
    }

    fn maybe_inner(&self) -> Result<&BlockchainDb, String> {
        Ok(self.inner())
    }
}
