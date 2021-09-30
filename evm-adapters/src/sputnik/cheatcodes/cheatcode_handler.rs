use sputnik::{
    backend::Backend,
    executor::{StackExecutor, StackState},
    Capture, Config, Context, CreateScheme, ExitError, ExitReason, ExitSucceed, Handler, Transfer,
};

use ethers::types::{Address, H160, H256, U256};
use std::{convert::Infallible, ops::Deref};

use once_cell::sync::Lazy;

use crate::sputnik::SputnikExecutor;

use super::{backend::CheatcodeBackend, memory_stackstate_owned::MemoryStackStateOwned};

// This is now getting us the right hash? Also tried [..20]
// Lazy::new(|| Address::from_slice(&keccak256("hevm cheat code")[12..]));
pub static CHEATCODE_ADDRESS: Lazy<Address> = Lazy::new(|| {
    Address::from_slice(&hex::decode("7109709ECfa91a80626fF3989D68f67F5b1DD12D").unwrap())
});

#[derive(Clone, Debug)]
// TODO: Should this be called `HookedHandler`? Maybe we could implement other hooks
// here, e.g. hardhat console.log-style, or dapptools logs, some ad-hoc method for tracing
// etc.
pub struct CheatcodeHandler<H> {
    handler: H,
}

impl<H> Deref for CheatcodeHandler<H> {
    type Target = H;
    fn deref(&self) -> &Self::Target {
        &self.handler
    }
}

// Forwards everything internally except for the transact_call which is overriden.
// TODO: Maybe we can pull this functionality up to the `Evm` trait to avoid having so many traits?
impl<'a, B: Backend> SputnikExecutor<CheatcodeStackState<'a, B>> for CheatcodeStackExecutor<'a, B> {
    fn config(&self) -> &Config {
        self.handler.config()
    }

    fn state(&self) -> &CheatcodeStackState<'a, B> {
        self.handler.state()
    }

    fn state_mut(&mut self) -> &mut CheatcodeStackState<'a, B> {
        self.handler.state_mut()
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
		event!(TransactCall {
			caller,
			address,
			value,
			data: &data,
			gas_limit,
		});

		let transaction_cost = gasometer::call_transaction_cost(&data, &access_list);
		let gasometer = &mut self.state.metadata_mut().gasometer;
		match gasometer.record_transaction(transaction_cost) {
			Ok(()) => (),
			Err(e) => return emit_exit!(e.into(), Vec::new()),
		}

		// Initialize initial addresses for EIP-2929
		if self.config.increase_state_access_gas {
			let addresses = self
				.precompile
				.clone()
				.into_keys()
				.into_iter()
				.chain(core::iter::once(caller))
				.chain(core::iter::once(address));
			self.state.metadata_mut().access_addresses(addresses);

			self.initialize_with_access_list(access_list);
		}

		self.state.inc_nonce(caller);

		let context = Context {
			caller,
			address,
			apparent_value: value,
		};

		match self.call_inner(
			address,
			Some(Transfer {
				source: caller,
				target: address,
				value,
			}),
			data,
			Some(gas_limit),
			false,
			false,
			false,
			context,
		) {
			Capture::Exit((s, v)) => emit_exit!(s, v),
			Capture::Trap(_) => unreachable!(),
		}
    }
}

pub type CheatcodeStackState<'a, B> = MemoryStackStateOwned<'a, CheatcodeBackend<B>>;

pub type CheatcodeStackExecutor<'a, B> =
    CheatcodeHandler<StackExecutor<'a, CheatcodeStackState<'a, B>>>;

impl<'a, B: Backend> CheatcodeStackExecutor<'a, B> {
    /// Decodes the provided calldata as a
    fn apply_cheatcode(&mut self, _input: Vec<u8>) -> Capture<(ExitReason, Vec<u8>), Infallible> {
        // Get a mutable ref to the state so we can apply the cheats
        let state = self.state_mut();

        // TODO: Decode ABI -> if function is not matched, return a Revert with "unknown cheatcode
        // [name]" as the retdata
        state.backend.cheats.block_timestamp = Some(100.into());

        Capture::Exit((ExitReason::Succeed(ExitSucceed::Stopped), vec![1; 32]))
    }
}

// Delegates everything internally, except the `call_inner` call, which is hooked
// so that we can modify
impl<'a, B: Backend> Handler for CheatcodeStackExecutor<'a, B> {
    type CreateInterrupt = Infallible;
    type CreateFeedback = Infallible;
    type CallInterrupt = Infallible;
    type CallFeedback = Infallible;

    fn call(
        &mut self,
        code_address: H160,
        transfer: Option<Transfer>,
        input: Vec<u8>,
        target_gas: Option<u64>,
        is_static: bool,
        context: Context,
    ) -> Capture<(ExitReason, Vec<u8>), Self::CallInterrupt> {
        // We intercept calls to the `CHEATCODE_ADDRESS` to apply the cheatcode directly
        // to the state.
        // NB: This is very similar to how Optimism's custom intercept logic to "predeploys" work
        // (e.g. with the StateManager)
        if code_address == *CHEATCODE_ADDRESS {
            self.apply_cheatcode(input)
        } else {
            self.handler.call(code_address, transfer, input, target_gas, is_static, context)
        }
    }

    // Everything else is left the same
    fn balance(&self, address: H160) -> U256 {
        self.handler.balance(address)
    }

    fn code_size(&self, address: H160) -> U256 {
        self.handler.code_size(address)
    }

    fn code_hash(&self, address: H160) -> H256 {
        self.handler.code_hash(address)
    }

    fn code(&self, address: H160) -> Vec<u8> {
        self.handler.code(address)
    }

    fn storage(&self, address: H160, index: H256) -> H256 {
        self.handler.storage(address, index)
    }

    fn original_storage(&self, address: H160, index: H256) -> H256 {
        self.handler.original_storage(address, index)
    }

    fn gas_left(&self) -> U256 {
        // Need to disambiguate type, because the same method exists in the `SputnikExecutor`
        // trait and the `Handler` trait.
        Handler::gas_left(&self.handler)
    }

    fn gas_price(&self) -> U256 {
        self.handler.gas_price()
    }

    fn origin(&self) -> H160 {
        self.handler.origin()
    }

    fn block_hash(&self, number: U256) -> H256 {
        self.handler.block_hash(number)
    }

    fn block_number(&self) -> U256 {
        self.handler.block_number()
    }

    fn block_coinbase(&self) -> H160 {
        self.handler.block_coinbase()
    }

    fn block_timestamp(&self) -> U256 {
        self.handler.block_timestamp()
    }

    fn block_difficulty(&self) -> U256 {
        self.handler.block_difficulty()
    }

    fn block_gas_limit(&self) -> U256 {
        self.handler.block_gas_limit()
    }

    fn chain_id(&self) -> U256 {
        self.handler.chain_id()
    }

    fn exists(&self, address: H160) -> bool {
        self.handler.exists(address)
    }

    fn deleted(&self, address: H160) -> bool {
        self.handler.deleted(address)
    }

    fn is_cold(&self, address: H160, index: Option<H256>) -> bool {
        self.handler.is_cold(address, index)
    }

    fn set_storage(&mut self, address: H160, index: H256, value: H256) -> Result<(), ExitError> {
        self.handler.set_storage(address, index, value)
    }

    fn log(&mut self, address: H160, topics: Vec<H256>, data: Vec<u8>) -> Result<(), ExitError> {
        self.handler.log(address, topics, data)
    }

    fn mark_delete(&mut self, address: H160, target: H160) -> Result<(), ExitError> {
        self.handler.mark_delete(address, target)
    }

    fn create(
        &mut self,
        caller: H160,
        scheme: CreateScheme,
        value: U256,
        init_code: Vec<u8>,
        target_gas: Option<u64>,
    ) -> Capture<(ExitReason, Option<H160>, Vec<u8>), Self::CreateInterrupt> {
        self.handler.create(caller, scheme, value, init_code, target_gas)
    }

    fn pre_validate(
        &mut self,
        context: &Context,
        opcode: sputnik::Opcode,
        stack: &sputnik::Stack,
    ) -> Result<(), ExitError> {
        self.handler.pre_validate(context, opcode, stack)
    }
}

#[cfg(test)]
mod tests {
    use sputnik::{executor::StackSubstateMetadata, Config};

    use crate::{
        sputnik::{
            cheatcodes::{memory_stackstate_owned::MemoryStackStateOwned, Cheatcodes},
            helpers::{new_backend, new_vicinity},
            Executor,
        },
        test_helpers::COMPILED,
        Evm,
    };

    use super::*;

    #[test]
    fn cheatcodes() {
        let config = Config::istanbul();

        // start w/ no cheatcodes
        let mut cheats = Cheatcodes::default();
        cheats.block_timestamp = Some(100.into());

        // create backend to instantiate the stack executor with
        let vicinity = new_vicinity();
        let backend = new_backend(&vicinity, Default::default());

        // make this a cheatcode-enabled backend
        let backend = CheatcodeBackend { backend, cheats };

        // create the memory stack state (owned, so that we can modify the backend via
        // self.state_mut on the transact_call fn)
        let gas_limit = 10_000_000;
        let metadata = StackSubstateMetadata::new(gas_limit, &config);
        let state = MemoryStackStateOwned::new(metadata, backend);
        let executor = StackExecutor::new_with_precompile(state, &config, Default::default());

        let executor = CheatcodeHandler { handler: executor };

        let mut evm = Executor::from_executor(executor, gas_limit);

        let compiled = COMPILED.get("GreeterTest").expect("could not find contract");
        let addr = "0x1000000000000000000000000000000000000000".parse().unwrap();
        evm.initialize_contracts(vec![(addr, compiled.runtime_bytecode.clone())]);
        evm.initialize_contracts([(*CHEATCODE_ADDRESS, vec![1u8; 1000].into())]);

        evm.call::<(), _>(
            Address::zero(),
            addr,
            &dapp_utils::get_func("function setUp()").unwrap(),
            (),
            0.into(),
        )
        .unwrap();

        let (_, reason, _) = evm
            .call::<(), _>(
                Address::zero(),
                addr,
                &dapp_utils::get_func("function testHevmTime()").unwrap(),
                (),
                0.into(),
            )
            .unwrap();
        assert_eq!(reason, ExitReason::Succeed(ExitSucceed::Stopped));
    }
}
