//! Backing databases.
//!
//! A backing database only serves reads that miss evm2's accepted-state overlay. Fork lifecycle,
//! snapshots and cheatcode policy live in [`ForkRegistry`](super::ForkRegistry) and
//! [`ExecutionLifecycle`](super::ExecutionLifecycle), not here.

use alloy_network::Network;
use alloy_primitives::{Address, B256, U256};
use evm2::{
    DatabaseError,
    bytecode::Bytecode,
    evm::{AccountInfo, Cache, DbResult, DynDatabase, EmptyDB, InMemoryDB, NonStaticAny},
    interpreter::Word,
};
use foundry_fork_db::{ForkBlockEnv, SharedBackend};
use revm::DatabaseRef;
use std::{any::TypeId, sync::Arc};

/// Database view the factory needs at construction time.
pub trait FoundryDatabase: DynDatabase {
    /// Whether the database is currently in forked mode.
    fn is_forked_mode(&self) -> bool {
        false
    }

    /// Returns the source chain ID of the active fork, independent of execution overrides.
    fn active_fork_source_chain_id(&self) -> Option<u64> {
        None
    }

    /// Drops remotely cached state of `address`, so the next read refetches it.
    fn evict_account(&self, _address: &Address) {}

    /// Drops a remotely cached storage slot, so the next read refetches it.
    fn evict_storage(&self, _address: &Address, _slot: &U256) {}
}

impl<D: FoundryDatabase + ?Sized> FoundryDatabase for &mut D {
    fn is_forked_mode(&self) -> bool {
        (**self).is_forked_mode()
    }

    fn active_fork_source_chain_id(&self) -> Option<u64> {
        (**self).active_fork_source_chain_id()
    }

    fn evict_account(&self, address: &Address) {
        (**self).evict_account(address);
    }

    fn evict_storage(&self, address: &Address, slot: &U256) {
        (**self).evict_storage(address, slot);
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

    fn evict_account(&self, address: &Address) {
        (**self).evict_account(address);
    }

    fn evict_storage(&self, address: &Address, slot: &U256) {
        (**self).evict_storage(address, slot);
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
        // The fork cache reports accounts the remote does not hold as existing empty accounts.
        // From Spurious Dragon on an empty account does not exist (EIP-161), so load it as absent;
        // otherwise fresh EIP-7702 authorities would be refunded as existing ones.
        Ok(info.filter(|info| !info.is_empty()).map(|info| {
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

impl<N: Network, B: ForkBlockEnv> FoundryDatabase for ForkDb<SharedBackend<N, B>> {
    fn is_forked_mode(&self) -> bool {
        true
    }

    fn active_fork_source_chain_id(&self) -> Option<u64> {
        Some(self.source_chain_id)
    }

    fn evict_account(&self, address: &Address) {
        self.db.data().accounts.write().remove(address);
    }

    fn evict_storage(&self, address: &Address, slot: &U256) {
        if let Some(storage) = self.db.data().storage.write().get_mut(address) {
            storage.remove(slot);
        }
    }
}

fn fatal(err: impl std::error::Error + Send + Sync + 'static) -> DatabaseError {
    DatabaseError::new(err, true)
}

/// A context's accepted state layered over its backing database, shared without copying.
///
/// EVMs start over this view, so creating one does not copy the accepted state; their overlay
/// only collects what they read and write. Operations that capture or replace the whole state
/// first materialize the view into the overlay (see
/// [`ExecutionLifecycle`](super::ExecutionLifecycle)).
pub(super) struct AcceptedView {
    pub(super) cache: Arc<Cache>,
    pub(super) backing: Box<dyn BackingDb>,
}

impl Clone for AcceptedView {
    fn clone(&self) -> Self {
        Self { cache: Arc::clone(&self.cache), backing: self.backing.clone_backing() }
    }
}

impl AcceptedView {
    /// Returns whether `db` is an accepted view.
    pub(super) fn is(db: &dyn DynDatabase) -> bool {
        let db: &dyn NonStaticAny = db;
        db.type_id() == TypeId::of::<Self>()
    }
}

impl DynDatabase for AcceptedView {
    fn get_account(&mut self, address: &Address) -> DbResult<Option<AccountInfo>> {
        let account = match self.cache.accounts.get(address) {
            Some(account) => account.clone(),
            None => self.backing.get_account(address)?,
        };
        // Storage written to an absent account, such as by `vm.store`, is accepted without the
        // account. evm2 reads no storage for an absent account, so report it as an empty one, as
        // the REVM cache kept it.
        if account.is_none()
            && self
                .cache
                .storage
                .get(address)
                .is_some_and(|storage| storage.slots.values().any(|value| !value.is_zero()))
        {
            return Ok(Some(AccountInfo::default()));
        }
        Ok(account)
    }

    fn get_code_by_hash(&mut self, hash: &B256) -> DbResult<Bytecode> {
        match self.cache.contracts.get(hash) {
            Some(code) => Ok(code.clone()),
            None => self.backing.get_code_by_hash(hash),
        }
    }

    fn get_storage(&mut self, address: &Address, key: &Word) -> DbResult<Word> {
        if let Some(storage) = self.cache.storage.get(address) {
            if let Some(value) = storage.slots.get(key) {
                return Ok(*value);
            }
            if storage.wiped {
                return Ok(Word::ZERO);
            }
        }
        if matches!(self.cache.accounts.get(address), Some(None)) {
            return Ok(Word::ZERO);
        }
        self.backing.get_storage(address, key)
    }

    fn get_block_hash(&mut self, number: &Word) -> DbResult<B256> {
        match self.cache.block_hashes.get(number) {
            Some(hash) => Ok(*hash),
            None => self.backing.get_block_hash(number),
        }
    }
}

impl FoundryDatabase for AcceptedView {
    fn is_forked_mode(&self) -> bool {
        self.backing.is_forked_mode()
    }

    fn active_fork_source_chain_id(&self) -> Option<u64> {
        self.backing.active_fork_source_chain_id()
    }

    fn evict_account(&self, address: &Address) {
        self.backing.evict_account(address);
    }

    fn evict_storage(&self, address: &Address, slot: &U256) {
        self.backing.evict_storage(address, slot);
    }
}
