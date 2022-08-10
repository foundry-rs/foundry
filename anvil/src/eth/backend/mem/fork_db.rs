use crate::{
    eth::backend::db::{Db, MaybeHashDatabase, SerializableState, StateDb},
    revm::AccountInfo,
    Address, U256,
};
use ethers::prelude::H256;
use forge::revm::Database;
use foundry_evm::executor::fork::database::ForkDbSnapshot;
pub use foundry_evm::executor::fork::database::ForkedDatabase;

/// Implement the helper for the fork database
impl Db for ForkedDatabase {
    fn insert_account(&mut self, address: Address, account: AccountInfo) {
        self.database_mut().insert_account(address, account)
    }

    fn set_storage_at(&mut self, address: Address, slot: U256, val: U256) {
        // this ensures the account is loaded first
        let _ = Database::basic(self, address);
        self.database_mut().set_storage_at(address, slot, val)
    }

    fn insert_block_hash(&mut self, number: U256, hash: H256) {
        self.inner().block_hashes().write().insert(number.as_u64(), hash);
    }

    fn dump_state(&self) -> Option<SerializableState> {
        None
    }

    fn load_state(&mut self, _buf: SerializableState) -> bool {
        false
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

impl MaybeHashDatabase for ForkedDatabase {}
impl MaybeHashDatabase for ForkDbSnapshot {}
