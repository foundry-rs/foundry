use std::ops::Deref;

use super::{Executor, SputnikExecutor};
use sputnik::{
    backend::{Backend, Basic},
    executor::{MemoryStackState, Precompile, StackExecutor, StackState, StackSubstateMetadata},
    Config, ExitReason, Handler,
};
use std::marker::PhantomData;

use std::{
    cell::{RefCell, RefMut},
    rc::Rc,
};

use ethers::types::{H160, H256, U256};

#[derive(Clone, Debug, Default)]
struct CheatcodeState {
    block_number: Option<U256>,
    block_timestamp: Option<u64>,
}

struct CheatcodeBackend<'a, B> {
    backend: RefMut<'a, B>,
    // TODO: remove.
    #[allow(unused)]
    state: CheatcodeState,
}

impl<'a, B: Backend> Backend for CheatcodeBackend<'a, B> {
    // TODO: Override the return values based on the values of `self.state`
    fn gas_price(&self) -> U256 {
        self.backend.gas_price()
    }

    fn origin(&self) -> H160 {
        self.backend.origin()
    }

    fn block_hash(&self, number: U256) -> H256 {
        self.backend.block_hash(number)
    }

    fn block_number(&self) -> U256 {
        self.backend.block_number()
    }

    fn block_coinbase(&self) -> H160 {
        self.backend.block_coinbase()
    }

    fn block_timestamp(&self) -> U256 {
        self.backend.block_timestamp()
    }

    fn block_difficulty(&self) -> U256 {
        self.backend.block_difficulty()
    }

    fn block_gas_limit(&self) -> U256 {
        self.backend.block_gas_limit()
    }

    fn chain_id(&self) -> U256 {
        self.backend.chain_id()
    }

    fn exists(&self, address: H160) -> bool {
        self.backend.exists(address)
    }

    fn basic(&self, address: H160) -> Basic {
        self.backend.basic(address)
    }

    fn code(&self, address: H160) -> Vec<u8> {
        self.backend.code(address)
    }

    fn storage(&self, address: H160, index: H256) -> H256 {
        self.backend.storage(address, index)
    }

    fn original_storage(&self, address: H160, index: H256) -> Option<H256> {
        self.backend.original_storage(address, index)
    }
}

impl<'a, B: Backend> CheatcodeBackend<'a, B> {
    fn new(backend: RefMut<'a, B>) -> Self {
        Self { backend, state: Default::default() }
    }
}

struct CheatcodeStackExecutor<'config, S, B> {
    executor: StackExecutor<'config, S>,
    backend: CheatcodeBackend<'config, B>,
}

impl<'c, S, B> CheatcodeStackExecutor<'c, S, B>
where
    S: StackState<'c>,
    B: Backend,
{
    pub fn new_with_precompile(
        backend: RefMut<'c, B>,
        state: S,
        config: &'c Config,
        precompile: Precompile,
    ) -> Self {
        Self {
            executor: StackExecutor::new_with_precompile(state, config, precompile),
            backend: CheatcodeBackend::new(backend),
        }
    }
}

// The implementation for the base Stack Executor just forwards to the internal methods.
impl<'a, S: StackState<'a>, B: Backend> SputnikExecutor<S, Config>
    for CheatcodeStackExecutor<'a, S, B>
{
    fn config(&self) -> &Config {
        self.executor.config()
    }

    fn state(&self) -> &S {
        self.executor.state()
    }

    fn state_mut(&mut self) -> &mut S {
        self.executor.state_mut()
    }

    fn gas_left(&self) -> U256 {
        // NB: We do this to avoid `function cannot return without recursing`
        U256::from(self.state().metadata().gasometer().gas())
    }

    fn transact_call(
        &mut self,
        caller: H160,
        address: H160,
        value: U256,
        data: Vec<u8>,
        gas_limit: u64,
        access_list: Vec<(H160, Vec<H256>)>,
    ) -> (ExitReason, Vec<u8>) {
        // TODO: Implement cheat code interception logic.
        self.executor.transact_call(caller, address, value, data, gas_limit, access_list)
    }
}

impl<'config, S, B> Deref for CheatcodeStackExecutor<'config, S, B> {
    type Target = StackExecutor<'config, S>;

    fn deref(&self) -> &Self::Target {
        &self.executor
    }
}

// Concrete implementation over the in-memory backend
impl<'a, B: Backend>
    Executor<
        MemoryStackState<'a, 'a, B>,
        Config,
        CheatcodeStackExecutor<'a, MemoryStackState<'a, 'a, B>, B>,
    >
{
    /// Given a gas limit, vm version, initial chain configuration and initial state
    // TOOD: See if we can make lifetimes better here
    pub fn new_with_cheatcode(
        gas_limit: u64,
        config: &'a Config,
        immutable_backend: &'a B,
        backend: RefMut<'a, B>,
    ) -> Self {
        // setup gasometer
        let metadata = StackSubstateMetadata::new(gas_limit, config);
        let state = MemoryStackState::new(metadata, immutable_backend);

        let executor =
            CheatcodeStackExecutor::new_with_precompile(backend, state, config, Default::default());

        Self { executor, gas_limit, marker: PhantomData }
    }
}
#[cfg(test)]
mod tests {
    use crate::sputnik::helpers::{new_backend, new_vicinity};
    use sputnik::Config;

    use super::*;

    #[test]
    fn intercepts_cheat_code() {
        let cfg = Config::istanbul();
        let vicinity = new_vicinity();
        let backend = new_backend(&vicinity, Default::default());
        // make it clone-able with interior mutability
        let backend = Rc::new(RefCell::new(backend));

        let b = backend.clone();
        let used_backend = b.borrow_mut();

        // `BorrowMutError` -> already borrowed, obviously wont' work, need to Clone
        let backend_immut = &*backend.as_ref().borrow();

        let evm = Executor::new_with_cheatcode(10_000_000, &cfg, backend_immut, used_backend);

        // run hevm test which sets the context
    }
}
