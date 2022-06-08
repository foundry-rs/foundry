use crate::{
    executor::{
        backend::{Backend, BackendDatabase, DatabaseExt},
        fork::{CreateFork, ForkId, SharedBackend},
        snapshot::Snapshots,
    },
    Address,
};
use bytes::Bytes;
use ethers::prelude::{H160, H256, U256};
use hashbrown::HashMap as Map;
use revm::{
    db::{CacheDB, DatabaseRef},
    Account, AccountInfo, Database, Env, Inspector, Log, Return, TransactOut,
};
use std::collections::HashMap;
use tracing::{trace, warn};

/// A wrapper around `Backend` that ensures only `revm::DatabaseRef` functions are called.
///
/// Any changes made during its existence that affect the caching layer of the underlying Database
/// will result in a clone of the initial Database.
///
/// Main purpose for this type is for fuzzing. A test function fuzzer will repeatedly call the
/// function via immutable raw (no state changes).
///
/// **N.B.**: we're assuming cheatcodes that alter the state (like multi fork swapping) are niche.
/// If they executed during fuzzing, it will require a clone of the initial input database. This way
/// we can support these cheatcodes in fuzzing cheaply without adding overhead for fuzz tests that
/// don't make use of them. Alternatively each test case would require its own `Backend` clone,
/// which would add significant overhead for large fuzz sets even if the Database is not big after
/// setup.
pub struct FuzzBackendWrapper<'a> {
    pub inner: &'a Backend,
    /// active database clone that holds the currently active db, like reverted snapshots, selected
    /// fork, etc.
    db_override: Option<CacheDB<BackendDatabase>>,
    /// tracks all created forks
    created_forks: HashMap<ForkId, SharedBackend>,
    /// Contains snapshots made at a certain point
    snapshots: Snapshots<CacheDB<BackendDatabase>>,
}

// === impl FuzzBackendWrapper ===

impl<'a> FuzzBackendWrapper<'a> {
    pub fn new(inner: &'a Backend) -> Self {
        Self {
            inner,
            db_override: None,
            created_forks: Default::default(),
            snapshots: Default::default(),
        }
    }

    /// Executes the configured transaction of the `env` without commiting state changes
    pub fn inspect_ref<INSP>(
        &mut self,
        mut env: Env,
        mut inspector: INSP,
    ) -> (Return, TransactOut, u64, Map<Address, Account>, Vec<Log>)
    where
        INSP: Inspector<Self>,
    {
        revm::evm_inner::<Self, true>(&mut env, self, &mut inspector).transact()
    }

    /// Returns the currently active database
    fn active_db(&self) -> &CacheDB<BackendDatabase> {
        self.db_override.as_ref().unwrap_or(&self.inner.db)
    }

    /// Sets the database override
    fn set_active(&mut self, db: CacheDB<BackendDatabase>) {
        self.db_override = Some(db)
    }
}

impl<'a> DatabaseExt for FuzzBackendWrapper<'a> {
    fn snapshot(&mut self) -> U256 {
        let id = self.snapshots.insert(self.active_db().clone());
        trace!(target: "backend", "Created new snapshot {}", id);
        id
    }

    fn revert(&mut self, id: U256) -> bool {
        if let Some(snapshot) =
            self.snapshots.remove(id).or_else(|| self.inner.snapshots.get(id).cloned())
        {
            self.set_active(snapshot);
            trace!(target: "backend", "Reverted snapshot {}", id);
            true
        } else {
            warn!(target: "backend", "No snapshot to revert for {}", id);
            false
        }
    }

    fn create_fork(&mut self, fork: CreateFork) -> eyre::Result<ForkId> {
        let (id, fork) = self.inner.forks.create_fork(fork)?;
        self.created_forks.insert(id.clone(), fork);
        Ok(id)
    }

    fn select_fork(&mut self, id: impl Into<ForkId>) -> eyre::Result<()> {
        let id = id.into();
        let fork = self
            .created_forks
            .get(&id)
            .or_else(|| self.inner.created_forks.get(&id))
            .cloned()
            .ok_or_else(|| eyre::eyre!("Fork Id {} does not exist", id))?;
        if let Some(ref mut db) = self.db_override {
            *db.db_mut() = BackendDatabase::Forked(fork, id);
        } else {
            let mut db = self.inner.db.clone();
            *db.db_mut() = BackendDatabase::Forked(fork, id);
            self.set_active(db);
        }
        Ok(())
    }
}

impl<'a> Database for FuzzBackendWrapper<'a> {
    fn basic(&mut self, address: H160) -> AccountInfo {
        if let Some(ref db) = self.db_override {
            DatabaseRef::basic(db, address)
        } else {
            DatabaseRef::basic(self.inner, address)
        }
    }
    fn code_by_hash(&mut self, code_hash: H256) -> Bytes {
        if let Some(ref db) = self.db_override {
            DatabaseRef::code_by_hash(db, code_hash)
        } else {
            DatabaseRef::code_by_hash(self.inner, code_hash)
        }
    }
    fn storage(&mut self, address: H160, index: U256) -> U256 {
        if let Some(ref db) = self.db_override {
            DatabaseRef::storage(db, address, index)
        } else {
            DatabaseRef::storage(self.inner, address, index)
        }
    }

    fn block_hash(&mut self, number: U256) -> H256 {
        if let Some(ref db) = self.db_override {
            DatabaseRef::block_hash(db, number)
        } else {
            DatabaseRef::block_hash(self.inner, number)
        }
    }
}
