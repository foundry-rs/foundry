//! Owned EVM2 execution state for Ethereum.

use alloy_primitives::{Address, B256};
use evm2::{
    bytecode::Bytecode,
    evm::{AccountInfo, Cache, DbResult, DynDatabase, EmptyDB, NonStaticAny, State, StateSnapshot},
    interpreter::Word,
};
use std::{any::TypeId, mem, sync::Arc};

mod types;
pub use types::{FoundryContext, FoundryEvmTypes};

mod env;
pub use env::EthereumEnv;

mod fork;
pub use fork::{EthereumFork, RpcDatabase};

/// A saved fork's accepted state and suspended transaction state.
///
/// The backing database belongs to the fork registry and is supplied when execution resumes.
/// Saving moves the accepted cache rather than cloning it with the transaction snapshot.
#[derive(Clone, Debug)]
pub struct SavedState {
    cache: Arc<Cache>,
    transaction: StateSnapshot,
}

impl SavedState {
    /// Takes ownership of a live state's memory layers without retaining its backing database.
    pub fn capture(mut state: State<'_>) -> Self {
        let cache = Arc::new(mem::take(&mut state.overlay_db_mut().cache));
        Self { cache, transaction: state.snapshot() }
    }

    /// Resumes owned execution over the backing database for this fork incarnation.
    pub fn resume<'a>(self, backing: impl DynDatabase + 'a) -> State<'a> {
        let mut state = self.transaction.into_state(backing);
        state.overlay_db_mut().cache = Arc::unwrap_or_clone(self.cache);
        state
    }

    /// Starts speculative execution without copying or mutating the accepted cache.
    ///
    /// Reads populate a session-local cache. Registry changes and state replacement must also
    /// remain session-local; discarding only the engine transaction does not undo those changes.
    pub fn speculate<'a>(&self, backing: impl DynDatabase + 'static) -> State<'a> {
        self.transaction
            .clone()
            .into_state(AcceptedView { cache: Arc::clone(&self.cache), backing: Box::new(backing) })
    }

    /// Materializes this session's accepted read-only backing before it is saved or replaced.
    ///
    /// Snapshots exclude the backing database. Retaining only their local cache would therefore
    /// lose accepted entries not read in this session. Ordinary speculative execution keeps its
    /// immutable view; this copy occurs only at an explicit replacement/capture boundary, once.
    /// Local account absence, storage wipes and overrides take precedence over accepted backing.
    pub fn materialize_initial(state: &mut State<'_>) {
        if state.initial().type_id() != TypeId::of::<AcceptedView>() {
            return;
        }
        let (accepted, backing) = {
            // SAFETY: Type identity verified the concrete AcceptedView. Unlike a generic or
            // lifetime-parameterized erased type, it contains only owned 'static data, so no
            // borrowed lifetime can be promoted by this extraction. The reference stays local.
            let view = unsafe {
                (state.initial_mut() as &mut dyn NonStaticAny)
                    .downcast_mut_unchecked::<AcceptedView>()
            };
            (Arc::clone(&view.cache), mem::replace(&mut view.backing, Box::new(EmptyDB::default())))
        };
        let local = mem::take(&mut state.overlay_db_mut().cache);
        let mut cache = (*accepted).clone();
        cache.merge(local);
        state.overlay_db_mut().cache = cache;
        state.overlay_db_mut().db = backing;
    }

    /// Returns accepted account, code and storage entries for read-only consumers.
    pub fn accepted_cache(&self) -> &Cache {
        &self.cache
    }
}

// This concrete type owns all its data and has no lifetime/type parameters. That distinction is
// required by the guarded lifetime-erased extraction in `materialize_initial`.
struct AcceptedView {
    cache: Arc<Cache>,
    backing: Box<dyn DynDatabase>,
}

impl DynDatabase for AcceptedView {
    fn get_account(&mut self, address: &Address) -> DbResult<Option<AccountInfo>> {
        match self.cache.accounts.get(address) {
            Some(account) => Ok(account.clone()),
            None => self.backing.get_account(address),
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use evm2::evm::InMemoryDB;

    const ACCOUNT: Address = Address::with_last_byte(0x42);

    fn backing() -> InMemoryDB {
        let mut db = InMemoryDB::default();
        db.insert_account_info(&ACCOUNT, AccountInfo::default().with_balance(Word::from(10)));
        db.insert_account_storage(&ACCOUNT, &Word::ZERO, &Word::from(99));
        db
    }

    #[test]
    fn speculative_materialization_preserves_accepted_storage_and_local_writes() {
        let mut state = State::new(backing());
        state.overlay_db_mut().insert_account_storage(&ACCOUNT, &Word::ZERO, &Word::from(5));
        let saved = SavedState::capture(state);
        let mut speculative = saved.speculate(backing());
        assert_eq!(
            speculative.storage_slot_untracked(&ACCOUNT, &Word::ZERO).unwrap(),
            Word::from(5)
        );
        speculative.overlay_db_mut().insert_account_storage(&ACCOUNT, &Word::ZERO, &Word::from(7));
        SavedState::materialize_initial(&mut speculative);
        let restored = SavedState::capture(speculative);
        assert_eq!(
            restored.resume(backing()).storage_slot_untracked(&ACCOUNT, &Word::ZERO).unwrap(),
            Word::from(7)
        );
        assert_eq!(
            saved.resume(backing()).storage_slot_untracked(&ACCOUNT, &Word::ZERO).unwrap(),
            Word::from(5)
        );
    }

    #[test]
    fn accepted_absence_and_storage_wipes_shadow_backing() {
        let mut state = State::new(EmptyDB::default());
        state.overlay_db_mut().cache.accounts.insert(ACCOUNT, None);
        let saved = SavedState::capture(state);
        let mut view = saved.speculate(backing());
        assert_eq!(view.overlay_db_mut().get_account(&ACCOUNT).unwrap(), None);
        assert_eq!(view.storage_slot_untracked(&ACCOUNT, &Word::ZERO).unwrap(), Word::ZERO);

        let mut state = State::new(backing());
        state.overlay_db_mut().cache.storage.entry(ACCOUNT).or_default().wipe();
        let saved = SavedState::capture(state);
        assert_eq!(
            saved.speculate(backing()).storage_slot_untracked(&ACCOUNT, &Word::ZERO).unwrap(),
            Word::ZERO
        );
    }

    #[test]
    fn unique_capture_resume_moves_cache() {
        let mut state = State::new(backing());
        state.overlay_db_mut().insert_account_storage(&ACCOUNT, &Word::ZERO, &Word::from(5));
        let saved = SavedState::capture(state);
        assert_eq!(Arc::strong_count(&saved.cache), 1);
        let ptr = saved.cache.storage.get(&ACCOUNT).unwrap() as *const _;
        let state = saved.resume(backing());
        assert_eq!(state.overlay_db().cache.storage.get(&ACCOUNT).unwrap() as *const _, ptr);
    }

    #[test]
    fn materializing_initial_preserves_unread_accepted_state_and_local_overrides() {
        let other = Address::with_last_byte(0x43);
        let mut state = State::new(backing());
        state.overlay_db_mut().insert_account_storage(&ACCOUNT, &Word::ZERO, &Word::from(5));
        state.overlay_db_mut().insert_account_storage(&other, &Word::ZERO, &Word::from(11));
        let saved = SavedState::capture(state);
        let mut speculative = saved.speculate(backing());
        speculative.overlay_db_mut().insert_account_storage(&other, &Word::ZERO, &Word::from(7));
        SavedState::materialize_initial(&mut speculative);
        assert!(speculative.initial().downcast_ref::<InMemoryDB>().is_some());
        // A snapshot is now complete even when its backing is replaced and ACCOUNT was unread.
        let mut restored = speculative.snapshot().into_state(backing());
        assert_eq!(restored.storage_slot_untracked(&ACCOUNT, &Word::ZERO).unwrap(), Word::from(5));
        assert_eq!(restored.storage_slot_untracked(&other, &Word::ZERO).unwrap(), Word::from(7));
        assert_eq!(
            saved.speculate(backing()).storage_slot_untracked(&other, &Word::ZERO).unwrap(),
            Word::from(11)
        );
    }
}
