use crate::{
    eth::backend::db::{Db, MaybeHashDatabase, SerializableState, StateDb},
    revm::AccountInfo,
    Address, U256,
};
use ethers::prelude::H256;
use forge::revm::Database;
pub use foundry_evm::executor::fork::database::ForkedDatabase;
use foundry_evm::executor::{
    backend::{snapshot::StateSnapshot, DatabaseResult},
    fork::database::ForkDbSnapshot,
};

/// Implement the helper for the fork database
impl Db for ForkedDatabase {
    fn insert_account(&mut self, address: Address, account: AccountInfo) {
        self.database_mut().insert_account(address, account)
    }

    fn set_storage_at(&mut self, address: Address, slot: U256, val: U256) -> DatabaseResult<()> {
        // this ensures the account is loaded first
        let _ = Database::basic(self, address)?;
        self.database_mut().set_storage_at(address, slot, val)
    }

    fn insert_block_hash(&mut self, number: U256, hash: H256) {
        self.inner().block_hashes().write().insert(number, hash);
    }

    fn dump_state(&self) -> DatabaseResult<Option<SerializableState>> {
        Ok(None)
    }

    fn load_state(&mut self, _buf: SerializableState) -> DatabaseResult<bool> {
        Ok(false)
    }

    fn snapshot(&mut self) -> U256 {
        self.insert_snapshot()
    }

    fn revert(&mut self, id: U256) -> bool {
        self.revert_snapshot(id)
    }

    fn current_state(&self) -> StateDb {
        StateDb::new(self.create_snapshot())
    }
}

impl MaybeHashDatabase for ForkedDatabase {
    fn clear_into_snapshot(&mut self) -> StateSnapshot {
        let db = self.inner().db();
        let accounts = std::mem::take(&mut *db.accounts.write());
        let storage = std::mem::take(&mut *db.storage.write());
        let block_hashes = std::mem::take(&mut *db.block_hashes.write());
        StateSnapshot { accounts, storage, block_hashes }
    }

    fn init_from_snapshot(&mut self, snapshot: StateSnapshot) {
        let db = self.inner().db();
        let StateSnapshot { accounts, storage, block_hashes } = snapshot;
        *db.accounts.write() = accounts;
        *db.storage.write() = storage;
        *db.block_hashes.write() = block_hashes;
    }
}
impl MaybeHashDatabase for ForkDbSnapshot {
    fn clear_into_snapshot(&mut self) -> StateSnapshot {
        std::mem::take(&mut self.snapshot)
    }

    fn init_from_snapshot(&mut self, snapshot: StateSnapshot) {
        self.snapshot = snapshot;
    }
}
