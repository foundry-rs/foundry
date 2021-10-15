use super::{
    backend::CheatcodeBackend, memory_stackstate_owned::MemoryStackStateOwned, HevmConsoleEvents,
    HEVM,
};
use crate::{
    sputnik::{Executor, SputnikExecutor},
    Evm,
};

use sputnik::{
    backend::Backend,
    executor::{
        Log, PrecompileOutput, StackExecutor, StackExitKind, StackState, StackSubstateMetadata,
    },
    gasometer, Capture, Config, Context, CreateScheme, ExitError, ExitReason, ExitRevert,
    ExitSucceed, Handler, Runtime, Transfer,
};
use std::{process::Command, rc::Rc};

use ethers::{
    abi::{RawLog, Token},
    contract::EthLogDecode,
    types::{Address, H160, H256, U256},
};
use std::convert::Infallible;

use once_cell::sync::Lazy;

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
    enable_ffi: bool,
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
        let transaction_cost = gasometer::call_transaction_cost(&data, &access_list);
        match self.state_mut().metadata_mut().gasometer_mut().record_transaction(transaction_cost) {
            Ok(()) => (),
            Err(e) => return (e.into(), Vec::new()),
        }

        // Initialize initial addresses for EIP-2929
        if self.config().increase_state_access_gas {
            let addresses = self
                .handler
                .precompile()
                .clone()
                .into_keys()
                .into_iter()
                .chain(core::iter::once(caller))
                .chain(core::iter::once(address));
            self.state_mut().metadata_mut().access_addresses(addresses);

            self.handler.initialize_with_access_list(access_list);
        }

        self.state_mut().inc_nonce(caller);

        let context = Context { caller, address, apparent_value: value };

        match self.call_inner(
            address,
            Some(Transfer { source: caller, target: address, value }),
            data,
            Some(gas_limit),
            false,
            false,
            false,
            context,
        ) {
            Capture::Exit((s, v)) => (s, v),
            Capture::Trap(_) => unreachable!(),
        }
    }

    fn transact_create(
        &mut self,
        caller: H160,
        value: U256,
        init_code: Vec<u8>,
        gas_limit: u64,
        access_list: Vec<(H160, Vec<H256>)>,
    ) -> ExitReason {
        self.handler.transact_create(caller, value, init_code, gas_limit, access_list)
    }

    fn create_address(&self, scheme: CreateScheme) -> Address {
        self.handler.create_address(scheme)
    }

    fn clear_logs(&mut self) {
        self.state_mut().substate.logs_mut().clear()
    }

    fn logs(&self) -> Vec<String> {
        let logs = self.state().substate.logs().to_vec();
        logs.into_iter()
            .filter_map(|log| {
                // convert to the ethers type
                let log = RawLog { topics: log.topics, data: log.data };
                HevmConsoleEvents::decode_log(&log).ok()
            })
            .map(|event| {
                use HevmConsoleEvents::*;
                match event {
                    LogFilter(inner) => inner.0,
                    LogsFilter(inner) => format!("0x{}", hex::encode(inner.0)),
                    LogAddressFilter(inner) => format!("{:?}", inner.0),
                    LogBytes32Filter(inner) => format!("0x{}", hex::encode(inner.0)),
                    LogIntFilter(inner) => format!("{:?}", inner.0),
                    LogUintFilter(inner) => format!("{:?}", inner.0),
                    LogBytesFilter(inner) => format!("0x{}", hex::encode(inner.0)),
                    LogStringFilter(inner) => inner.0,
                    LogNamedAddressFilter(inner) => format!("{}: {:?}", inner.key, inner.val),
                    LogNamedBytes32Filter(inner) => {
                        format!("{}: 0x{}", inner.key, hex::encode(inner.val))
                    }
                    LogNamedDecimalIntFilter(inner) => format!(
                        "{}: {:?}",
                        inner.key,
                        ethers::utils::parse_units(inner.val, inner.decimals.as_u32()).unwrap()
                    ),
                    LogNamedDecimalUintFilter(inner) => {
                        format!(
                            "{}: {:?}",
                            inner.key,
                            ethers::utils::parse_units(inner.val, inner.decimals.as_u32()).unwrap()
                        )
                    }
                    LogNamedIntFilter(inner) => format!("{}: {:?}", inner.key, inner.val),
                    LogNamedUintFilter(inner) => format!("{}: {:?}", inner.key, inner.val),
                    LogNamedBytesFilter(inner) => {
                        format!("{}: 0x{}", inner.key, hex::encode(inner.val))
                    }
                    LogNamedStringFilter(inner) => format!("{}: {}", inner.key, inner.val),
                }
            })
            .collect()
    }
}

pub type CheatcodeStackState<'a, B> = MemoryStackStateOwned<'a, CheatcodeBackend<B>>;

pub type CheatcodeStackExecutor<'a, B> =
    CheatcodeHandler<StackExecutor<'a, CheatcodeStackState<'a, B>>>;

impl<'a, B: Backend> Executor<CheatcodeStackState<'a, B>, CheatcodeStackExecutor<'a, B>> {
    pub fn new_with_cheatcodes(
        backend: B,
        gas_limit: u64,
        config: &'a Config,
        enable_ffi: bool,
    ) -> Self {
        // make this a cheatcode-enabled backend
        let backend = CheatcodeBackend { backend, cheats: Default::default() };

        // create the memory stack state (owned, so that we can modify the backend via
        // self.state_mut on the transact_call fn)
        let metadata = StackSubstateMetadata::new(gas_limit, config);
        let state = MemoryStackStateOwned::new(metadata, backend);

        // create the executor and wrap it with the cheatcode handler
        let executor = StackExecutor::new_with_precompile(state, config, Default::default());
        let executor = CheatcodeHandler { handler: executor, enable_ffi };

        let mut evm = Executor::from_executor(executor, gas_limit);

        // Need to create a non-empty contract at the cheat code address so that the EVM backend
        // thinks that something exists there.
        evm.initialize_contracts([(*CHEATCODE_ADDRESS, vec![0u8; 1].into())]);

        evm
    }
}

fn evm_error(retdata: &str) -> Capture<(ExitReason, Vec<u8>), Infallible> {
    Capture::Exit((
        ExitReason::Revert(ExitRevert::Reverted),
        ethers::abi::encode(&[Token::String(retdata.to_owned())]),
    ))
}

impl<'a, B: Backend> CheatcodeStackExecutor<'a, B> {
    /// Decodes the provided calldata as a
    fn apply_cheatcode(&mut self, input: Vec<u8>) -> Capture<(ExitReason, Vec<u8>), Infallible> {
        let mut res = vec![];

        // Get a mutable ref to the state so we can apply the cheats
        let state = self.state_mut();

        if let Ok(timestamp) = HEVM.decode::<U256, _>("warp", &input) {
            state.backend.cheats.block_timestamp = Some(timestamp);
        }

        if let Ok(block_number) = HEVM.decode::<U256, _>("roll", &input) {
            state.backend.cheats.block_number = Some(block_number);
        }

        if let Ok((address, slot, value)) = HEVM.decode::<(Address, H256, H256), _>("store", &input)
        {
            state.set_storage(address, slot, value);
        }

        if let Ok((address, slot)) = HEVM.decode::<(Address, H256), _>("load", &input) {
            res = state.storage(address, slot).0.to_vec();
        }

        if let Ok(args) = HEVM.decode::<Vec<String>, _>("ffi", &input) {
            // if FFI is not explicitly enabled at runtime, do not let this be called
            // (we could have an FFI cheatcode executor instead but feels like
            // over engineering)
            if !self.enable_ffi {
                return evm_error(
                    "ffi disabled: run again with --ffi if you want to allow tests to call external scripts",
                )
            }

            // execute the command & get the stdout
            let output = match Command::new(&args[0]).args(&args[1..]).output() {
                Ok(res) => res.stdout,
                Err(err) => return evm_error(&err.to_string()),
            };

            // get the hex string & decode it
            let output = unsafe { std::str::from_utf8_unchecked(&output) };
            let decoded = match hex::decode(&output[2..]) {
                Ok(res) => res,
                Err(err) => return evm_error(&err.to_string()),
            };

            // encode the data as Bytes
            res = ethers::abi::encode(&[Token::Bytes(decoded.to_vec())]);
        }

        // TODO: Add more cheat codes.

        Capture::Exit((ExitReason::Succeed(ExitSucceed::Stopped), res))
    }

    // NB: This function is copy-pasted from uptream's `execute`, adjusted so that we call the
    // Runtime with our own handler
    pub fn execute(&mut self, runtime: &mut Runtime) -> ExitReason {
        match runtime.run(self) {
            Capture::Exit(s) => s,
            Capture::Trap(_) => unreachable!("Trap is Infallible"),
        }
    }

    // NB: This function is copy-pasted from uptream's call_inner
    #[allow(clippy::too_many_arguments)]
    fn call_inner(
        &mut self,
        code_address: H160,
        transfer: Option<Transfer>,
        input: Vec<u8>,
        target_gas: Option<u64>,
        is_static: bool,
        take_l64: bool,
        take_stipend: bool,
        context: Context,
    ) -> Capture<(ExitReason, Vec<u8>), Infallible> {
        macro_rules! try_or_fail {
            ( $e:expr ) => {
                match $e {
                    Ok(v) => v,
                    Err(e) => return Capture::Exit((e.into(), Vec::new())),
                }
            };
        }

        fn l64(gas: u64) -> u64 {
            gas - gas / 64
        }

        let after_gas = if take_l64 && self.config().call_l64_after_gas {
            if self.config().estimate {
                let initial_after_gas = self.state().metadata().gasometer().gas();
                let diff = initial_after_gas - l64(initial_after_gas);
                try_or_fail!(self.state_mut().metadata_mut().gasometer_mut().record_cost(diff));
                self.state().metadata().gasometer().gas()
            } else {
                l64(self.state().metadata().gasometer().gas())
            }
        } else {
            self.state().metadata().gasometer().gas()
        };

        let target_gas = target_gas.unwrap_or(after_gas);
        let mut gas_limit = std::cmp::min(target_gas, after_gas);

        try_or_fail!(self.state_mut().metadata_mut().gasometer_mut().record_cost(gas_limit));

        if let Some(transfer) = transfer.as_ref() {
            if take_stipend && transfer.value != U256::zero() {
                gas_limit = gas_limit.saturating_add(self.config().call_stipend);
            }
        }

        let code = self.code(code_address);

        self.handler.enter_substate(gas_limit, is_static);
        self.state_mut().touch(context.address);

        if let Some(depth) = self.state().metadata().depth() {
            if depth > self.config().call_stack_limit {
                let _ = self.handler.exit_substate(StackExitKind::Reverted);
                return Capture::Exit((ExitError::CallTooDeep.into(), Vec::new()))
            }
        }

        if let Some(transfer) = transfer {
            match self.state_mut().transfer(transfer) {
                Ok(()) => (),
                Err(e) => {
                    let _ = self.handler.exit_substate(StackExitKind::Reverted);
                    return Capture::Exit((ExitReason::Error(e), Vec::new()))
                }
            }
        }

        if let Some(precompile) = self.handler.precompile().get(&code_address) {
            return match (*precompile)(&input, Some(gas_limit), &context, is_static) {
                Ok(PrecompileOutput { exit_status, output, cost, logs }) => {
                    for Log { address, topics, data } in logs {
                        match self.log(address, topics, data) {
                            Ok(_) => continue,
                            Err(error) => return Capture::Exit((ExitReason::Error(error), output)),
                        }
                    }

                    let _ = self.state_mut().metadata_mut().gasometer_mut().record_cost(cost);
                    let _ = self.handler.exit_substate(StackExitKind::Succeeded);
                    Capture::Exit((ExitReason::Succeed(exit_status), output))
                }
                Err(e) => {
                    let _ = self.handler.exit_substate(StackExitKind::Failed);
                    Capture::Exit((ExitReason::Error(e), Vec::new()))
                }
            }
        }

        // each cfg is about 200 bytes, is this a lot to clone? why does this error
        // not manfiest upstream?
        let config = self.config().clone();
        let mut runtime = Runtime::new(Rc::new(code), Rc::new(input), context, &config);
        let reason = self.execute(&mut runtime);
        // // log::debug!(target: "evm", "Call execution using address {}: {:?}", code_address,
        // reason);

        match reason {
            ExitReason::Succeed(s) => {
                let _ = self.handler.exit_substate(StackExitKind::Succeeded);
                Capture::Exit((ExitReason::Succeed(s), runtime.machine().return_value()))
            }
            ExitReason::Error(e) => {
                let _ = self.handler.exit_substate(StackExitKind::Failed);
                Capture::Exit((ExitReason::Error(e), Vec::new()))
            }
            ExitReason::Revert(e) => {
                let _ = self.handler.exit_substate(StackExitKind::Reverted);
                Capture::Exit((ExitReason::Revert(e), runtime.machine().return_value()))
            }
            ExitReason::Fatal(e) => {
                self.state_mut().metadata_mut().gasometer_mut().fail();
                let _ = self.handler.exit_substate(StackExitKind::Failed);
                Capture::Exit((ExitReason::Fatal(e), Vec::new()))
            }
        }
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
    use sputnik::Config;

    use crate::{
        fuzz::FuzzedExecutor,
        sputnik::{
            helpers::{new_backend, new_vicinity},
            Executor,
        },
        test_helpers::COMPILED,
        Evm,
    };

    use super::*;

    #[test]
    fn debug_logs() {
        let config = Config::istanbul();
        let vicinity = new_vicinity();
        let backend = new_backend(&vicinity, Default::default());
        let gas_limit = 10_000_000;
        let mut evm = Executor::new_with_cheatcodes(backend, gas_limit, &config, true);

        let compiled = COMPILED.get("DebugLogs").expect("could not find contract");
        let (addr, _, _, _) =
            evm.deploy(Address::zero(), compiled.bytecode.clone(), 0.into()).unwrap();

        // after the evm call is done, we call `logs` and print it all to the user
        let (_, _, _, logs) =
            evm.call::<(), _, _>(Address::zero(), addr, "test_log()", (), 0.into()).unwrap();
        let expected = [
            "Hi",
            "0x1234",
            "0x1111111111111111111111111111111111111111",
            "0x41b1a0649752af1b28b3dc29a1556eee781e4a4c3a1f7f53f90fa834de098c4d",
            "123",
            "1234",
            "0x4567",
            "lol",
            "addr: 0x2222222222222222222222222222222222222222",
            "key: 0x41b1a0649752af1b28b3dc29a1556eee781e4a4c3a1f7f53f90fa834de098c4d",
            "key: 123000000000000000000",
            "key: 1234000000000000000000",
            "key: 123",
            "key: 1234",
            "key: 0x4567",
            "key: lol",
        ]
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
        assert_eq!(logs, expected);
    }

    #[test]
    fn cheatcodes() {
        let config = Config::istanbul();
        let vicinity = new_vicinity();
        let backend = new_backend(&vicinity, Default::default());
        let gas_limit = 10_000_000;
        let mut evm = Executor::new_with_cheatcodes(backend, gas_limit, &config, true);

        let compiled = COMPILED.get("CheatCodes").expect("could not find contract");
        let (addr, _, _, _) =
            evm.deploy(Address::zero(), compiled.bytecode.clone(), 0.into()).unwrap();

        let state = evm.state().clone();
        let mut cfg = proptest::test_runner::Config::default();
        cfg.failure_persistence = None;
        let runner = proptest::test_runner::TestRunner::new(cfg);

        // ensure the storage slot is set at 10 anyway
        let (storage_contract, _, _, _) = evm
            .call::<Address, _, _>(Address::zero(), addr, "store()(address)", (), 0.into())
            .unwrap();
        let (slot, _, _, _) = evm
            .call::<U256, _, _>(Address::zero(), storage_contract, "slot0()(uint256)", (), 0.into())
            .unwrap();
        assert_eq!(slot, 10.into());

        let evm = FuzzedExecutor::new(&mut evm, runner);

        for func in compiled.abi.functions().filter(|func| func.name.starts_with("test")) {
            let should_fail = func.name.starts_with("testFail");
            if func.inputs.is_empty() {
                let (_, reason, _, _) =
                    evm.as_mut().call_unchecked(Address::zero(), addr, func, (), 0.into()).unwrap();
                assert!(evm.as_mut().check_success(addr, &reason, should_fail));
            } else {
                // if the unwrap passes then it works
                evm.fuzz(func, addr, should_fail).unwrap();
            }

            evm.as_mut().reset(state.clone());
        }
    }

    #[test]
    fn ffi_fails_if_disabled() {
        let config = Config::istanbul();
        let vicinity = new_vicinity();
        let backend = new_backend(&vicinity, Default::default());
        let gas_limit = 10_000_000;
        let mut evm = Executor::new_with_cheatcodes(backend, gas_limit, &config, false);

        let compiled = COMPILED.get("CheatCodes").expect("could not find contract");
        let (addr, _, _, _) =
            evm.deploy(Address::zero(), compiled.bytecode.clone(), 0.into()).unwrap();

        let err =
            evm.call::<(), _, _>(Address::zero(), addr, "testFFI()", (), 0.into()).unwrap_err();
        let reason = match err {
            crate::EvmError::Execution { reason, .. } => reason,
            _ => panic!("unexpected error"),
        };
        assert_eq!(reason, "ffi disabled: run again with --ffi if you want to allow tests to call external scripts");
    }
}
