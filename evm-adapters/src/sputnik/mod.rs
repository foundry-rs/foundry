mod evm;
pub use evm::*;

mod forked_backend;
pub use forked_backend::ForkMemoryBackend;

use ethers::types::{H160, H256, U256};

use sputnik::{
    executor::{StackExecutor, StackState},
    Config, ExitReason,
};

/// Abstraction over the StackExecutor used inside of Sputnik, so that we can replace
/// it with one that implements HEVM-style cheatcodes (or other features).
pub trait SputnikExecutor<S, C> {
    fn config(&self) -> &C;
    fn state(&self) -> &S;
    fn state_mut(&mut self) -> &mut S;
    fn gas_left(&self) -> U256;
    fn transact_call(
        &mut self,
        caller: H160,
        address: H160,
        value: U256,
        data: Vec<u8>,
        gas_limit: u64,
        access_list: Vec<(H160, Vec<H256>)>,
    ) -> (ExitReason, Vec<u8>);
}

// The implementation for the base Stack Executor just forwards to the internal methods.
impl<'a, S: StackState<'a>> SputnikExecutor<S, Config> for StackExecutor<'a, S> {
    fn config(&self) -> &Config {
        self.config()
    }

    fn state(&self) -> &S {
        self.state()
    }

    fn state_mut(&mut self) -> &mut S {
        self.state_mut()
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
        self.transact_call(caller, address, value, data, gas_limit, access_list)
    }
}
