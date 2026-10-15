//! Backing databases.
//!
//! A backing database only serves reads that miss evm2's accepted-state overlay. Fork lifecycle,
//! snapshots and cheatcode policy live in [`ForkRegistry`](super::ForkRegistry) and
//! [`ExecutionLifecycle`](super::ExecutionLifecycle), not here.

use alloy_primitives::{Address, B256, U256};
use evm2::{
    DatabaseError,
    bytecode::Bytecode,
    evm::{AccountInfo, DbResult, DynDatabase, EmptyDB, InMemoryDB},
};
use revm::DatabaseRef;

/// Database view the factory needs at construction time.
///
/// This is the read-only slice of [`DatabaseExt`](crate::backend::DatabaseExt) that execution
/// needs; fork and snapshot ownership live elsewhere.
pub trait FoundryDatabase: DynDatabase {
    /// Whether the database is currently in forked mode.
    fn is_forked_mode(&self) -> bool {
        false
    }

    /// Returns the source chain ID of the active fork, independent of execution overrides.
    fn active_fork_source_chain_id(&self) -> Option<u64> {
        None
    }
}

impl<D: FoundryDatabase + ?Sized> FoundryDatabase for &mut D {
    fn is_forked_mode(&self) -> bool {
        (**self).is_forked_mode()
    }

    fn active_fork_source_chain_id(&self) -> Option<u64> {
        (**self).active_fork_source_chain_id()
    }
}

impl FoundryDatabase for EmptyDB {}

impl FoundryDatabase for InMemoryDB {}

/// A shareable backing database: every fork and every nested transaction reads through its own
/// handle to the same source.
pub trait BackingDb: FoundryDatabase + Send + Sync + 'static {
    /// Returns another handle to the same source.
    fn clone_backing(&self) -> Box<dyn BackingDb>;
}

impl<D: FoundryDatabase + Clone + Send + Sync + 'static> BackingDb for D {
    fn clone_backing(&self) -> Box<dyn BackingDb> {
        Box::new(self.clone())
    }
}

impl FoundryDatabase for Box<dyn BackingDb> {
    fn is_forked_mode(&self) -> bool {
        (**self).is_forked_mode()
    }

    fn active_fork_source_chain_id(&self) -> Option<u64> {
        (**self).active_fork_source_chain_id()
    }
}

/// A remote fork's backing database: Foundry's RPC-backed fork cache, read by evm2.
///
/// The fork cache (`foundry-fork-db`) stores REVM-typed accounts; this adapter converts them at the
/// read boundary, so no REVM type reaches execution. Replacing the cache's storage types is
/// tracked separately in `foundry-core`.
#[derive(Clone, Debug)]
pub struct ForkDb<D> {
    db: D,
    source_chain_id: u64,
}

impl<D> ForkDb<D> {
    /// Wraps a fork cache whose remote state belongs to `source_chain_id`.
    pub const fn new(db: D, source_chain_id: u64) -> Self {
        Self { db, source_chain_id }
    }

    /// Returns the wrapped fork cache.
    pub const fn inner(&self) -> &D {
        &self.db
    }
}

impl<D> DynDatabase for ForkDb<D>
where
    D: DatabaseRef<Error: std::error::Error + Send + Sync + 'static> + 'static,
{
    fn get_account(&mut self, address: &Address) -> DbResult<Option<AccountInfo>> {
        let info = self.db.basic_ref(*address).map_err(fatal)?;
        Ok(info.map(|info| {
            let code = info.code.map(|code| Bytecode::new_raw(code.original_bytes()));
            AccountInfo::new(info.balance, info.nonce, info.code_hash, code)
        }))
    }

    fn get_code_by_hash(&mut self, code_hash: &B256) -> DbResult<Bytecode> {
        let code = self.db.code_by_hash_ref(*code_hash).map_err(fatal)?;
        Ok(Bytecode::new_raw(code.original_bytes()))
    }

    fn get_storage(&mut self, address: &Address, key: &U256) -> DbResult<U256> {
        self.db.storage_ref(*address, *key).map_err(fatal)
    }

    fn get_block_hash(&mut self, number: &U256) -> DbResult<B256> {
        self.db.block_hash_ref(number.saturating_to()).map_err(fatal)
    }
}

impl<D> FoundryDatabase for ForkDb<D>
where
    D: DatabaseRef<Error: std::error::Error + Send + Sync + 'static> + 'static,
{
    fn is_forked_mode(&self) -> bool {
        true
    }

    fn active_fork_source_chain_id(&self) -> Option<u64> {
        Some(self.source_chain_id)
    }
}

fn fatal(err: impl std::error::Error + Send + Sync + 'static) -> DatabaseError {
    DatabaseError::new(err, true)
}
