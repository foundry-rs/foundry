use ethers::abi::ethereum_types::{H160, H256, U256};
use parking_lot::RwLock;
use sputnik::{
    backend::{Apply, Backend, Basic, Log},
    executor::stack::{MemoryStackSubstate, StackState, StackSubstateMetadata},
    ExitError, Transfer,
};
use std::{fmt::Debug, ops::Deref, sync::Arc};

/// A state that can be shared across threads
///
/// This can can be used as global state.
pub type SharedState<S> = Arc<RwLock<S>>;

/// Create a new shareable state.
pub fn new_shared_state<'config, S: StackState<'config>>(state: S) -> SharedState<S> {
    Arc::new(RwLock::new(state))
}

/// A state that branches off from the shared state and operates according to the following rules:
///
/// Reading:
///   - forked local state takes precedent over shared state: if the storage value is not present in
///     the local state, it queries the shared state.
///
/// Writing:
///   - all memory altering operations will be applied to the local state
#[derive(Clone)]
pub struct ForkedState<'config, S> {
    shared_state: SharedState<S>,
    substate: MemoryStackSubstate<'config>,
}

impl<'config, S> Debug for ForkedState<'config, S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ForkedState").field("substate", &self.substate).finish_non_exhaustive()
    }
}

impl<'config, S> ForkedState<'config, S>
where
    S: StackState<'config>,
{
    /// Create a new forked state with the `SharedState` as root state
    /// This will initialize a new, empty substate that will hold all modifications to the
    /// shared_state, so that the shared state remains untouched.
    pub fn new(shared_state: SharedState<S>, metadata: StackSubstateMetadata<'config>) -> Self {
        Self { shared_state, substate: MemoryStackSubstate::new(metadata) }
    }

    #[must_use]
    pub fn deconstruct(
        self,
    ) -> (
        impl IntoIterator<Item = Apply<impl IntoIterator<Item = (H256, H256)>>>,
        impl IntoIterator<Item = Log>,
    ) {
        self.substate.deconstruct(self.shared_state.read().deref())
    }

    pub fn withdraw(&mut self, address: H160, value: U256) -> Result<(), ExitError> {
        self.substate.withdraw(address, value, self.shared_state.read().deref())
    }

    pub fn deposit(&mut self, address: H160, value: U256) {
        self.substate.deposit(address, value, self.shared_state.read().deref())
    }
}

impl<'config, S> Backend for ForkedState<'config, S>
where
    S: StackState<'config>,
{
    fn gas_price(&self) -> U256 {
        self.shared_state.read().gas_price()
    }
    fn origin(&self) -> H160 {
        self.shared_state.read().origin()
    }
    fn block_hash(&self, number: U256) -> H256 {
        self.shared_state.read().block_hash(number)
    }
    fn block_number(&self) -> U256 {
        self.shared_state.read().block_number()
    }
    fn block_coinbase(&self) -> H160 {
        self.shared_state.read().block_coinbase()
    }
    fn block_timestamp(&self) -> U256 {
        self.shared_state.read().block_timestamp()
    }
    fn block_difficulty(&self) -> U256 {
        self.shared_state.read().block_difficulty()
    }
    fn block_gas_limit(&self) -> U256 {
        self.shared_state.read().block_gas_limit()
    }
    fn block_base_fee_per_gas(&self) -> U256 {
        self.shared_state.read().block_base_fee_per_gas()
    }
    fn chain_id(&self) -> U256 {
        self.shared_state.read().chain_id()
    }

    fn exists(&self, address: H160) -> bool {
        self.substate.known_account(address).is_some() || self.shared_state.read().exists(address)
    }

    fn basic(&self, address: H160) -> Basic {
        self.substate
            .known_basic(address)
            .unwrap_or_else(|| self.shared_state.read().basic(address))
    }

    fn code(&self, address: H160) -> Vec<u8> {
        self.substate.known_code(address).unwrap_or_else(|| self.shared_state.read().code(address))
    }

    fn storage(&self, address: H160, key: H256) -> H256 {
        self.substate
            .known_storage(address, key)
            .unwrap_or_else(|| self.shared_state.read().storage(address, key))
    }

    fn original_storage(&self, address: H160, key: H256) -> Option<H256> {
        if let Some(value) = self.substate.known_original_storage(address, key) {
            return Some(value)
        }
        self.shared_state.read().original_storage(address, key)
    }
}

impl<'config, S> StackState<'config> for ForkedState<'config, S>
where
    S: StackState<'config>,
{
    fn metadata(&self) -> &StackSubstateMetadata<'config> {
        self.substate.metadata()
    }

    fn metadata_mut(&mut self) -> &mut StackSubstateMetadata<'config> {
        self.substate.metadata_mut()
    }

    fn enter(&mut self, gas_limit: u64, is_static: bool) {
        self.substate.enter(gas_limit, is_static)
    }

    fn exit_commit(&mut self) -> Result<(), ExitError> {
        self.substate.exit_commit()
    }

    fn exit_revert(&mut self) -> Result<(), ExitError> {
        self.substate.exit_revert()
    }

    fn exit_discard(&mut self) -> Result<(), ExitError> {
        self.substate.exit_discard()
    }

    fn is_empty(&self, address: H160) -> bool {
        if let Some(known_empty) = self.substate.known_empty(address) {
            return known_empty
        }

        let basic = self.shared_state.read().basic(address);
        basic.balance == U256::zero() &&
            basic.nonce == U256::zero() &&
            self.shared_state.read().code(address).is_empty()
    }

    fn deleted(&self, address: H160) -> bool {
        self.substate.deleted(address)
    }

    fn is_cold(&self, address: H160) -> bool {
        self.substate.is_cold(address)
    }

    fn is_storage_cold(&self, address: H160, key: H256) -> bool {
        self.substate.is_storage_cold(address, key)
    }

    fn inc_nonce(&mut self, address: H160) {
        self.substate.inc_nonce(address, self.shared_state.read().deref());
    }

    fn set_storage(&mut self, address: H160, key: H256, value: H256) {
        self.substate.set_storage(address, key, value)
    }

    fn reset_storage(&mut self, address: H160) {
        self.substate.reset_storage(address, self.shared_state.read().deref());
    }

    fn log(&mut self, address: H160, topics: Vec<H256>, data: Vec<u8>) {
        self.substate.log(address, topics, data);
    }

    fn set_deleted(&mut self, address: H160) {
        self.substate.set_deleted(address)
    }

    fn set_code(&mut self, address: H160, code: Vec<u8>) {
        self.substate.set_code(address, code, self.shared_state.read().deref());
    }

    fn transfer(&mut self, transfer: Transfer) -> Result<(), ExitError> {
        self.substate.transfer(transfer, self.shared_state.read().deref())
    }

    fn reset_balance(&mut self, address: H160) {
        self.substate.reset_balance(address, self.shared_state.read().deref());
    }

    fn touch(&mut self, address: H160) {
        self.substate.touch(address, self.shared_state.read().deref());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sputnik::{
        forked_backend::MemCache, helpers::new_vicinity, new_shared_cache, SharedBackend,
        SharedCache,
    };
    use ethers::{
        abi::Address,
        prelude::{Http, Provider},
    };
    use once_cell::sync::Lazy;
    use sputnik::{
        backend::{MemoryBackend, MemoryVicinity},
        executor::stack::MemoryStackState,
        Config,
    };
    use std::convert::TryFrom;

    // We need a bunch of global static values in order to satisfy sputnik's lifetime requirements
    // for `'static` so that we can Send them
    static G_CONFIG: Lazy<Config> = Lazy::new(Config::istanbul);
    static G_VICINITY: Lazy<MemoryVicinity> = Lazy::new(new_vicinity);

    // This is the root `Backend` that stores all root state
    static G_BACKEND: Lazy<MemoryBackend> =
        Lazy::new(|| MemoryBackend::new(&*G_VICINITY, Default::default()));

    // Type that hold the global root storage type and a shareable Backend
    struct GlobalBackend {
        cache: SharedCache<MemCache>,
        backend: SharedBackend,
    }

    // this is pretty horrible but the sputnik types require 'static lifetime in order to be
    // shareable
    static G_FORKED_BACKEND: Lazy<GlobalBackend> = Lazy::new(|| {
        let cache = new_shared_cache(MemCache::default());
        let provider = Provider::<Http>::try_from(
            "https://mainnet.infura.io/v3/c60b0bb42f8a4c6481ecd229eddaca27",
        )
        .unwrap();
        let vicinity = G_VICINITY.clone();
        let backend = SharedBackend::new(Arc::new(provider), cache.clone(), vicinity, None);
        GlobalBackend { cache, backend }
    });

    // this looks horrendous and is due to how sputnik borrows
    fn setup_states() -> (
        SharedState<MemoryStackState<'static, 'static, SharedBackend>>,
        ForkedState<'static, MemoryStackState<'static, 'static, SharedBackend>>,
    ) {
        let gas_limit = 12_000_000;
        let metadata = StackSubstateMetadata::new(gas_limit, &*G_CONFIG);
        let state = MemoryStackState::new(metadata.clone(), &G_FORKED_BACKEND.backend);

        let shared_state = new_shared_state(state);
        let forked_state = ForkedState::new(shared_state.clone(), metadata.clone());
        (shared_state, forked_state)
    }

    #[test]
    fn forked_shared_state_works() {
        let (shared_state, mut forked_state) = setup_states();

        // some rng contract from etherscan
        let address: Address = "63091244180ae240c87d1f528f5f269134cb07b3".parse().unwrap();

        assert!(shared_state.read().exists(address));
        assert!(forked_state.exists(address));

        let amount = shared_state.read().basic(address).balance;
        assert_eq!(forked_state.basic(address).balance, amount);

        forked_state.deposit(address, amount);
        assert_eq!(forked_state.basic(address).balance, amount * 2);
        // shared state remains the same
        assert_eq!(shared_state.read().basic(address).balance, amount);
        assert_eq!(G_FORKED_BACKEND.cache.read().get(&address).unwrap().balance, amount);
    }

    #[test]
    fn can_spawn_state_to_thread() {
        // some rng contract from etherscan
        let address: Address = "63091244180ae240c87d1f528f5f269134cb07b3".parse().unwrap();

        let (shared_state, mut forked_state) = setup_states();

        let amount = shared_state.read().basic(address).balance;
        let t = std::thread::spawn(move || {
            forked_state.deposit(address, amount);
            assert_eq!(forked_state.basic(address).balance, amount * 2);
        });
        t.join().unwrap();

        // amount remains unchanged
        assert_eq!(shared_state.read().basic(address).balance, amount);
    }

    #[test]
    fn shared_state_works() {
        let gas_limit = 12_000_000;
        let metadata = StackSubstateMetadata::new(gas_limit, &*G_CONFIG);
        let state = MemoryStackState::new(metadata.clone(), &*G_BACKEND);

        let shared_state = new_shared_state(state);
        let mut forked_state = ForkedState::new(shared_state.clone(), metadata.clone());

        // deposit some funds in a new address
        let address = Address::random();
        assert!(!shared_state.read().exists(address));
        assert!(!forked_state.exists(address));

        let amount = 1337u64.into();
        shared_state.write().deposit(address, amount);

        assert!(shared_state.read().exists(address));
        assert!(forked_state.exists(address));
        assert_eq!(shared_state.read().basic(address).balance, amount);
        assert_eq!(forked_state.basic(address).balance, amount);

        // double deposit in fork
        forked_state.deposit(address, amount);
        assert_eq!(forked_state.basic(address).balance, amount * 2);
        // shared state remains the same
        assert_eq!(shared_state.read().basic(address).balance, amount);

        let mut another_forked_state = ForkedState::new(shared_state.clone(), metadata);
        let t = std::thread::spawn(move || {
            assert_eq!(another_forked_state.basic(address).balance, amount);
            another_forked_state.deposit(address, amount * 10);
        });
        t.join().unwrap();

        assert_eq!(forked_state.basic(address).balance, amount * 2);
        assert_eq!(shared_state.read().basic(address).balance, amount);
    }
}
