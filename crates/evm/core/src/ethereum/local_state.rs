//! Copy-on-write local state for Ethereum execution.

use alloy_primitives::{Address, B256, U256};
use evm2::evm::{AccountInfo, CacheDB, Database, Db, EmptyDB, PendingState};
use std::sync::Arc;

/// Persistent local state shared by cloned Forge executors until a transaction commits.
#[derive(Clone, Debug)]
pub struct LocalState<D: Database + Clone = EmptyDB>(Arc<CacheDB<Db<D>>>);

impl<D: Database + Clone> LocalState<D> {
    /// Creates a copy-on-write state overlay over an evm2 database.
    pub fn new(database: D) -> Self {
        Self(Arc::new(CacheDB::new(Db::new(database))))
    }

    /// Returns the accepted state.
    pub fn database(&self) -> &CacheDB<Db<D>> {
        &self.0
    }

    /// Returns mutable accepted state, cloning it if another executor still shares it.
    pub fn database_mut(&mut self) -> &mut CacheDB<Db<D>> {
        Arc::make_mut(&mut self.0)
    }

    /// Accepts a detached transaction's state changes.
    pub fn commit(&mut self, pending: &PendingState) {
        self.database_mut().commit_pending(pending);
    }

    /// Sets an account's balance while retaining its other fields from the backing database.
    pub fn set_balance(&mut self, address: Address, balance: U256) -> Result<(), evm2::AnyError> {
        let db = self.database_mut();
        let mut info = Database::get_account(db, &address)?.unwrap_or_default();
        info.balance = balance;
        db.insert_account_info(&address, info);
        Ok(())
    }

    /// Sets an account's nonce while retaining its other fields from the backing database.
    pub fn set_nonce(&mut self, address: Address, nonce: u64) -> Result<(), evm2::AnyError> {
        let db = self.database_mut();
        let mut info = Database::get_account(db, &address)?.unwrap_or_default();
        info.nonce = nonce;
        db.insert_account_info(&address, info);
        Ok(())
    }
}

impl Default for LocalState {
    fn default() -> Self {
        Self::new(EmptyDB::default())
    }
}

impl<D: Database + Clone + 'static> Database for LocalState<D> {
    type Error = evm2::AnyError;

    fn get_account(&mut self, address: &Address) -> Result<Option<AccountInfo>, Self::Error> {
        Database::get_account(self.database_mut(), address)
    }

    fn get_code_by_hash(
        &mut self,
        code_hash: &B256,
    ) -> Result<evm2::bytecode::Bytecode, Self::Error> {
        Database::get_code_by_hash(self.database_mut(), code_hash)
    }

    fn get_storage(&mut self, address: &Address, key: &U256) -> Result<U256, Self::Error> {
        Database::get_storage(self.database_mut(), address, key)
    }

    fn get_block_hash(&mut self, number: &U256) -> Result<B256, Self::Error> {
        Database::get_block_hash(self.database_mut(), number)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evm2::{bytecode::Bytecode, evm::InMemoryDB};

    #[test]
    fn backing_reads_and_commits_remain_isolated_across_clones() {
        let address = Address::with_last_byte(1);
        let key = U256::from(2);
        let mut backing = InMemoryDB::default();
        backing.insert_account_info(&address, AccountInfo::default());
        backing.insert_account_storage(&address, &key, &U256::from(3));
        let mut state = LocalState::new(backing);
        let mut snapshot = state.clone();

        assert_eq!(Database::get_storage(&mut &mut state, &address, &key).unwrap(), U256::from(3));
        let mut pending = PendingState::default();
        pending.insert_storage(address, key, U256::from(3), U256::from(4));
        state.commit(&pending);

        assert_eq!(Database::get_storage(&mut &mut state, &address, &key).unwrap(), U256::from(4));
        assert_eq!(
            Database::get_storage(&mut &mut snapshot, &address, &key).unwrap(),
            U256::from(3)
        );
    }

    #[test]
    fn overrides_preserve_backing_account_fields() {
        let address = Address::with_last_byte(1);
        let code = Bytecode::new_legacy([0x60, 0x00].into());
        let mut backing = InMemoryDB::default();
        backing.insert_account_info(
            &address,
            AccountInfo {
                balance: U256::from(3),
                nonce: 7,
                code: Some(code),
                ..Default::default()
            },
        );
        let mut state = LocalState::new(backing);

        state.set_balance(address, U256::from(5)).unwrap();
        state.set_nonce(address, 8).unwrap();

        let info = state.database().account_info(&address).unwrap();
        assert_eq!((info.balance, info.nonce), (U256::from(5), 8));
        let code_hash = info.code_hash;
        assert_eq!(
            Database::get_code_by_hash(&mut &mut state, &code_hash)
                .unwrap()
                .original_bytes()
                .as_ref(),
            &[0x60, 0x00]
        );
    }
}
