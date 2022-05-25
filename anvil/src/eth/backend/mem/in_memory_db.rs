//! The in memory DB

use crate::{
    eth::backend::db::{Db, StateDb},
    mem::{snapshot::Snapshots, state::state_merkle_trie_root},
    revm::{db::DatabaseRef, Account, AccountInfo, Database, DatabaseCommit},
    Address, U256,
};
use bytes::Bytes;
use ethers::prelude::{H160, H256};
use foundry_evm::{revm::InMemoryDB, HashMap as Map};
use tracing::{trace, warn};

/// In memory Database for anvil
///
/// This acts like a wrapper type for [InMemoryDB] but is capable of applying snapshots
#[derive(Debug)]
pub struct MemDb {
    inner: InMemoryDB,
    snapshots: Snapshots<InMemoryDB>,
}

impl Default for MemDb {
    fn default() -> Self {
        Self { inner: InMemoryDB::default(), snapshots: Default::default() }
    }
}

impl DatabaseRef for MemDb {
    fn basic(&self, address: H160) -> AccountInfo {
        DatabaseRef::basic(&self.inner, address)
    }

    fn code_by_hash(&self, code_hash: H256) -> Bytes {
        DatabaseRef::code_by_hash(&self.inner, code_hash)
    }

    fn storage(&self, address: H160, index: U256) -> U256 {
        DatabaseRef::storage(&self.inner, address, index)
    }

    fn block_hash(&self, number: U256) -> H256 {
        DatabaseRef::block_hash(&self.inner, number)
    }
}

impl Database for MemDb {
    fn basic(&mut self, address: H160) -> AccountInfo {
        Database::basic(&mut self.inner, address)
    }

    fn code_by_hash(&mut self, code_hash: H256) -> Bytes {
        Database::code_by_hash(&mut self.inner, code_hash)
    }

    fn storage(&mut self, address: H160, index: U256) -> U256 {
        Database::storage(&mut self.inner, address, index)
    }

    fn block_hash(&mut self, number: U256) -> H256 {
        Database::block_hash(&mut self.inner, number)
    }
}

impl DatabaseCommit for MemDb {
    fn commit(&mut self, changes: Map<H160, Account>) {
        DatabaseCommit::commit(&mut self.inner, changes)
    }
}

impl Db for MemDb {
    fn insert_account(&mut self, address: Address, account: AccountInfo) {
        self.inner.insert_cache(address, account)
    }

    fn set_storage_at(&mut self, address: Address, slot: U256, val: U256) {
        self.inner.insert_cache_storage(address, slot, val)
    }

    /// Creates a new snapshot
    fn snapshot(&mut self) -> U256 {
        let id = self.snapshots.insert(self.inner.clone());
        trace!(target: "backend::memdb", "Created new snapshot {}", id);
        id
    }

    fn revert(&mut self, id: U256) -> bool {
        if let Some(snapshot) = self.snapshots.remove(id) {
            self.inner = snapshot;
            trace!(target: "backend::memdb", "Reverted snapshot {}", id);
            true
        } else {
            warn!(target: "backend::memdb", "No snapshot to revert for {}", id);
            false
        }
    }

    fn maybe_state_root(&self) -> Option<H256> {
        Some(state_merkle_trie_root(self.inner.cache(), self.inner.storage()))
    }

    fn current_state(&self) -> StateDb {
        StateDb::new(self.inner.clone())
    }
}
