use crate::{
    eth::backend::db::{Db, StateDb},
    revm::AccountInfo,
    Address, U256,
};
pub use foundry_evm::executor::fork::database::ForkedDatabase;

/// Implement the helper for the fork database
impl Db for ForkedDatabase {
    fn insert_account(&mut self, address: Address, account: AccountInfo) {
        self.database_mut().insert_account(address, account)
    }

    fn set_storage_at(&mut self, address: Address, slot: U256, val: U256) {
        self.database_mut().set_storage_at(address, slot, val)
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
