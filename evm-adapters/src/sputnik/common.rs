//! Additional traits and common implementation to create a custom `SputnikExecutor`

use crate::{
    call_tracing::{CallTrace, CallTraceArena},
    sputnik::SputnikExecutor,
};

use sputnik::{
    backend::Backend,
    executor::stack::{
        Log, PrecompileFailure, PrecompileOutput, PrecompileSet, StackExecutor, StackExitKind,
        StackState,
    },
    gasometer, Capture, Config, Context, CreateScheme, ExitError, ExitReason, ExitRevert, Handler,
    Opcode, Runtime, Stack, Transfer,
};
use std::rc::Rc;

use ethers::{
    abi::RawLog,
    types::{Address, H160, H256, U256},
};

use crate::call_tracing::LogCallOrder;
use ethers_core::abi::Token;
use std::{convert::Infallible, marker::PhantomData};

use crate::sputnik::cheatcodes::debugger::DebugArena;

use crate::sputnik::{
    cheatcodes::memory_stackstate_owned::MemoryStackStateOwned, utils::convert_log,
};

/// an(other) abstraction over a sputnik `Handler` implementation
///
/// This provides default implementations for `Handler` functions that can be replaced by
/// implementers. In other words, unless overwritten by the implementer all functions are delegated
/// to `<sputnik::StackExecutor as sputnik::Handler>` The main purpose of this trait is to ease the
/// implementation of custom `SputnikExecutor`s as this comes with a lot of boilerplate.
///
/// On top of delegates for the `sputnik::Handler`, this trait provides additional hooks that are
/// invoked by the `SputnikExecutor`.
///
/// # Example
///
/// Implement your own `ExecutionHandler`
///
/// ```rust
/// use std::rc::Rc;
/// use evm_adapters::sputnik::cheatcodes::memory_stackstate_owned::MemoryStackStateOwned;
/// use sputnik::{
///     backend::Backend,
///     executor::stack::{PrecompileSet, StackExecutor},
///     ExitReason, ExitSucceed, Runtime,
/// };
/// use evm_adapters::sputnik::common::ExecutionHandler;
/// use ethers::types::Address;
///
/// // declare your custom handler, the type that's essentially the wrapper around the standard
/// // sputnik handler, but with additional context and state
/// pub struct MyHandler<H> {
///     /// placeholder for the sputnik `StackExecutor`
///     handler: H,
///     /// additional, custom state, for example diagnostics, cheatcode context, etc.
///     custom_state: (),
/// }
///
/// // declare your state
/// pub type MyStackState<'config, Backend> = MemoryStackStateOwned<'config, Backend>;
///
/// // declare your `StackExecutor`, the type that actually drives the runtime
/// pub type MyStackExecutor<'a, 'b, B, P> =
///     MyHandler<StackExecutor<'a, 'b, MyStackState<'a, B>, P>>;
///
/// // finally, we implement `ExecutionHandler`
/// // the default implementation delegates all `sputnik::Handler` calls to `MyHandler.handler`
/// // additional functions like `ExecutionHandler::do_call` can be replaced,
/// // essentially intercepting the call
/// // the control flow is `SputnikExecutor -> ExecutionHandler -> sputnik::Handler`
/// impl<'a, 'b, Back, Precom: 'b> ExecutionHandler<'a, 'b, Back, Precom, MyStackState<'a, Back>>
/// for MyStackExecutor<'a, 'b, Back, Precom>
///     where
///         Back: Backend,
///         Precom: PrecompileSet,
/// {
///     fn stack_executor(&self) -> &StackExecutor<'a, 'b, MyStackState<'a, Back>, Precom> {
///         &self.handler
///     }
///
///     fn stack_executor_mut(
///         &mut self,
///     ) -> &mut StackExecutor<'a, 'b, MyStackState<'a, Back>, Precom> {
///         &mut self.handler
///     }
///
///     fn is_tracing_enabled(&self) -> bool {
///         false
///     }
///
///     fn debug_execute(
///         &mut self,
///         _runtime: &mut Runtime,
///         _address: Address,
///         _code: Rc<Vec<u8>>,
///         _creation: bool,
///     ) -> ExitReason {
///         ExitReason::Succeed(ExitSucceed::Returned)
///     }
/// }
/// ```
pub trait ExecutionHandler<'a, 'b, Back, Precom: 'b, State>
where
    Back: Backend,
    Precom: PrecompileSet,
    State: StackState<'a>,
{
    /// returns the wrapper sputnik `StackExecutor`
    fn stack_executor(&self) -> &StackExecutor<'a, 'b, State, Precom>;

    /// returns the wrapper sputnik `StackExecutor`
    fn stack_executor_mut(&mut self) -> &mut StackExecutor<'a, 'b, State, Precom>;

    // Everything else is left the same
    fn balance(&self, address: H160) -> U256 {
        self.stack_executor().balance(address)
    }

    fn code_size(&self, address: H160) -> U256 {
        self.stack_executor().code_size(address)
    }

    fn code_hash(&self, address: H160) -> H256 {
        self.stack_executor().code_hash(address)
    }

    fn code(&self, address: H160) -> Vec<u8> {
        self.stack_executor().code(address)
    }

    fn storage(&self, address: H160, index: H256) -> H256 {
        self.stack_executor().storage(address, index)
    }

    fn original_storage(&self, address: H160, index: H256) -> H256 {
        self.stack_executor().original_storage(address, index)
    }

    fn gas_left(&self) -> U256 {
        // Need to disambiguate type, because the same method exists in the `SputnikExecutor`
        // trait and the `Handler` trait.
        Handler::gas_left(self.stack_executor())
    }

    fn gas_price(&self) -> U256 {
        self.stack_executor().gas_price()
    }

    fn origin(&self) -> H160 {
        self.stack_executor().origin()
    }

    fn block_hash(&self, number: U256) -> H256 {
        self.stack_executor().block_hash(number)
    }

    fn block_number(&self) -> U256 {
        self.stack_executor().block_number()
    }

    fn block_coinbase(&self) -> H160 {
        self.stack_executor().block_coinbase()
    }

    fn block_timestamp(&self) -> U256 {
        self.stack_executor().block_timestamp()
    }

    fn block_difficulty(&self) -> U256 {
        self.stack_executor().block_difficulty()
    }

    fn block_gas_limit(&self) -> U256 {
        self.stack_executor().block_gas_limit()
    }

    fn block_base_fee_per_gas(&self) -> U256 {
        self.stack_executor().block_base_fee_per_gas()
    }

    fn chain_id(&self) -> U256 {
        self.stack_executor().chain_id()
    }

    fn exists(&self, address: H160) -> bool {
        self.stack_executor().exists(address)
    }

    fn deleted(&self, address: H160) -> bool {
        self.stack_executor().deleted(address)
    }

    fn is_cold(&self, address: H160, index: Option<H256>) -> bool {
        self.stack_executor().is_cold(address, index)
    }

    fn set_storage(&mut self, address: H160, index: H256, value: H256) -> Result<(), ExitError> {
        self.stack_executor_mut().set_storage(address, index, value)
    }

    fn mark_delete(&mut self, address: H160, target: H160) -> Result<(), ExitError> {
        self.stack_executor_mut().mark_delete(address, target)
    }

    fn pre_validate(
        &mut self,
        context: &Context,
        opcode: sputnik::Opcode,
        stack: &sputnik::Stack,
    ) -> Result<(), ExitError> {
        self.stack_executor_mut().pre_validate(context, opcode, stack)
    }

    /// Invoked when logs are cleared
    fn on_clear_logs(&mut self) {}

    /// Returns an additional a vector of string parsed logs that occurred during the previous VM
    /// execution
    fn additional_logs(&self) -> Vec<String> {
        Default::default()
    }

    fn is_tracing_enabled(&self) -> bool;

    /// Executes the call/create while also tracking the state of the machine (including opcodes)
    fn debug_execute(
        &mut self,
        runtime: &mut Runtime,
        address: Address,
        code: Rc<Vec<u8>>,
        creation: bool,
    ) -> ExitReason;

    /// This is provided so the implementers can also access fill_trace
    fn fill_trace(
        &mut self,
        new_trace: &Option<CallTrace>,
        success: bool,
        output: Option<Vec<u8>>,
        pre_trace_index: usize,
    );

    /// The delegate for `sputnik::Handler::create`
    fn do_create(
        &mut self,
        caller: H160,
        scheme: CreateScheme,
        value: U256,
        init_code: Vec<u8>,
        target_gas: Option<u64>,
    ) -> Capture<(ExitReason, Option<H160>, Vec<u8>), Infallible> {
        self.stack_executor_mut().create(caller, scheme, value, init_code, target_gas)
    }

    /// The delegate for `sputnik::Handler::call`
    fn do_call(
        &mut self,
        code_address: H160,
        transfer: Option<Transfer>,
        input: Vec<u8>,
        target_gas: Option<u64>,
        is_static: bool,
        context: Context,
    ) -> Capture<(ExitReason, Vec<u8>), Infallible> {
        self.stack_executor_mut().call(
            code_address,
            transfer,
            input,
            target_gas,
            is_static,
            context,
        )
    }
}

/// This wrapper type is necessary as we can't implement foreign traits for traits (Handler for
/// ExecutionHandler)
pub struct ExecutionHandlerWrapper<'a, 'b, Back, Precom: 'b, State, ExecHandler>
where
    Back: Backend,
    Precom: PrecompileSet,
    State: StackState<'a>,
    ExecHandler: ExecutionHandler<'a, 'b, Back, Precom, State>,
{
    /// The wrapped `ExecutionHandler`
    handler: ExecHandler,
    // this is necessary because of all the unconstrained trait generics...
    _marker: PhantomData<(&'a Back, &'b State, Precom)>,
}

impl<'a, 'b, Back, Precom: 'b, State, ExecHandler>
    ExecutionHandlerWrapper<'a, 'b, Back, Precom, State, ExecHandler>
where
    Back: Backend,
    Precom: PrecompileSet,
    State: StackState<'a>,
    ExecHandler: ExecutionHandler<'a, 'b, Back, Precom, State>,
{
    pub fn new(handler: ExecHandler) -> Self {
        Self { handler, _marker: Default::default() }
    }

    pub fn handler(&self) -> &ExecHandler {
        &self.handler
    }

    pub fn handler_mut(&mut self) -> &mut ExecHandler {
        &mut self.handler
    }

    fn stack_executor(&self) -> &StackExecutor<'a, 'b, State, Precom> {
        self.handler().stack_executor()
    }

    fn stack_executor_mut(&mut self) -> &mut StackExecutor<'a, 'b, State, Precom> {
        self.handler_mut().stack_executor_mut()
    }
}

impl<'a, 'b, Back, Precom: 'b, ExecHandler>
    ExecutionHandlerWrapper<'a, 'b, Back, Precom, MemoryStackStateOwned<'a, Back>, ExecHandler>
where
    ExecHandler: ExecutionHandler<'a, 'b, Back, Precom, MemoryStackStateOwned<'a, Back>>,
    Back: Backend,
    Precom: PrecompileSet,
{
    // NB: This function is copy-pasted from upstream's `execute`, adjusted so that we call the
    // Runtime with our own handler
    pub fn execute(&mut self, runtime: &mut Runtime) -> ExitReason {
        match runtime.run(self) {
            Capture::Exit(s) => s,
            Capture::Trap(_) => unreachable!("Trap is Infallible"),
        }
    }

    fn start_trace(
        &mut self,
        address: H160,
        input: Vec<u8>,
        transfer: U256,
        creation: bool,
    ) -> Option<CallTrace> {
        if self.handler().is_tracing_enabled() {
            let mut trace: CallTrace = CallTrace {
                // depth only starts tracking at first child substate and is 0. so add 1 when depth
                // is some.
                depth: if let Some(depth) = self.state().metadata().depth() {
                    depth + 1
                } else {
                    0
                },
                addr: address,
                created: creation,
                data: input,
                value: transfer,
                label: self.state().labels.get(&address).cloned(),
                ..Default::default()
            };

            self.state_mut().trace_mut().push_trace(0, &mut trace);
            self.state_mut().trace_index = trace.idx;
            Some(trace)
        } else {
            None
        }
    }

    fn fill_trace(
        &mut self,
        new_trace: &Option<CallTrace>,
        success: bool,
        output: Option<Vec<u8>>,
        pre_trace_index: usize,
    ) {
        self.handler.fill_trace(new_trace, success, output, pre_trace_index)
    }

    // NB: This function is copy-pasted from upstream's create_inner
    fn create_inner(
        &mut self,
        caller: H160,
        scheme: CreateScheme,
        value: U256,
        init_code: Vec<u8>,
        target_gas: Option<u64>,
        take_l64: bool,
    ) -> Capture<(ExitReason, Option<H160>, Vec<u8>), Infallible> {
        let pre_index = self.state().trace_index;

        let address = self.create_address(scheme);
        let trace = self.start_trace(address, init_code.clone(), value, true);

        macro_rules! try_or_fail {
            ( $e:expr ) => {
                match $e {
                    Ok(v) => v,
                    Err(e) => {
                        self.fill_trace(&trace, false, None, pre_index);
                        return Capture::Exit((e.into(), None, Vec::new()))
                    }
                }
            };
        }

        fn check_first_byte(config: &Config, code: &[u8]) -> Result<(), ExitError> {
            if config.disallow_executable_format {
                if let Some(0xef) = code.get(0) {
                    return Err(ExitError::InvalidCode)
                }
            }
            Ok(())
        }

        fn l64(gas: u64) -> u64 {
            gas - gas / 64
        }

        self.state_mut().metadata_mut().access_address(caller);
        self.state_mut().metadata_mut().access_address(address);

        if let Some(depth) = self.state().metadata().depth() {
            if depth > self.config().call_stack_limit {
                self.fill_trace(&trace, false, None, pre_index);
                return Capture::Exit((ExitError::CallTooDeep.into(), None, Vec::new()))
            }
        }

        if self.balance(caller) < value {
            self.fill_trace(&trace, false, None, pre_index);
            return Capture::Exit((ExitError::OutOfFund.into(), None, Vec::new()))
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

        let gas_limit = core::cmp::min(after_gas, target_gas);
        try_or_fail!(self.state_mut().metadata_mut().gasometer_mut().record_cost(gas_limit));

        self.state_mut().inc_nonce(caller);

        self.stack_executor_mut().enter_substate(gas_limit, false);

        {
            if self.code_size(address) != U256::zero() {
                self.fill_trace(&trace, false, None, pre_index);
                let _ = self.stack_executor_mut().exit_substate(StackExitKind::Failed);
                return Capture::Exit((ExitError::CreateCollision.into(), None, Vec::new()))
            }

            if self.stack_executor_mut().nonce(address) > U256::zero() {
                self.fill_trace(&trace, false, None, pre_index);
                let _ = self.stack_executor_mut().exit_substate(StackExitKind::Failed);
                return Capture::Exit((ExitError::CreateCollision.into(), None, Vec::new()))
            }

            self.state_mut().reset_storage(address);
        }

        let context = Context { address, caller, apparent_value: value };
        let transfer = Transfer { source: caller, target: address, value };
        match self.state_mut().transfer(transfer) {
            Ok(()) => (),
            Err(e) => {
                self.fill_trace(&trace, false, None, pre_index);
                let _ = self.stack_executor_mut().exit_substate(StackExitKind::Reverted);
                return Capture::Exit((ExitReason::Error(e), None, Vec::new()))
            }
        }

        if self.config().create_increase_nonce {
            self.state_mut().inc_nonce(address);
        }

        let config = self.config().clone();
        let mut runtime;
        let reason = if self.state().debug_enabled {
            let code = Rc::new(init_code);
            runtime = Runtime::new(code.clone(), Rc::new(Vec::new()), context, &config);
            self.handler_mut().debug_execute(&mut runtime, address, code, true)
        } else {
            runtime = Runtime::new(Rc::new(init_code), Rc::new(Vec::new()), context, &config);
            self.execute(&mut runtime)
        };
        // log::debug!(target: "evm", "Create execution using address {}: {:?}", address, reason);

        match reason {
            ExitReason::Succeed(s) => {
                let out = runtime.machine().return_value();

                // As of EIP-3541 code starting with 0xef cannot be deployed
                if let Err(e) = check_first_byte(self.config(), &out) {
                    self.state_mut().metadata_mut().gasometer_mut().fail();
                    self.fill_trace(&trace, false, None, pre_index);
                    let _ = self.stack_executor_mut().exit_substate(StackExitKind::Failed);
                    return Capture::Exit((e.into(), None, Vec::new()))
                }

                if let Some(limit) = self.config().create_contract_limit {
                    if out.len() > limit {
                        self.state_mut().metadata_mut().gasometer_mut().fail();
                        self.fill_trace(&trace, false, None, pre_index);
                        let _ = self.stack_executor_mut().exit_substate(StackExitKind::Failed);
                        return Capture::Exit((
                            ExitError::CreateContractLimit.into(),
                            None,
                            Vec::new(),
                        ))
                    }
                }

                match self.state_mut().metadata_mut().gasometer_mut().record_deposit(out.len()) {
                    Ok(()) => {
                        self.fill_trace(&trace, true, Some(out.clone()), pre_index);
                        let e = self.stack_executor_mut().exit_substate(StackExitKind::Succeeded);
                        self.state_mut().set_code(address, out);
                        // this may overwrite the trace and thats okay
                        try_or_fail!(e);
                        Capture::Exit((ExitReason::Succeed(s), Some(address), Vec::new()))
                    }
                    Err(e) => {
                        self.fill_trace(&trace, false, None, pre_index);
                        let _ = self.stack_executor_mut().exit_substate(StackExitKind::Failed);
                        Capture::Exit((ExitReason::Error(e), None, Vec::new()))
                    }
                }
            }
            ExitReason::Error(e) => {
                self.state_mut().metadata_mut().gasometer_mut().fail();
                self.fill_trace(&trace, false, None, pre_index);
                let _ = self.stack_executor_mut().exit_substate(StackExitKind::Failed);
                Capture::Exit((ExitReason::Error(e), None, Vec::new()))
            }
            ExitReason::Revert(e) => {
                self.fill_trace(&trace, false, Some(runtime.machine().return_value()), pre_index);
                let _ = self.stack_executor_mut().exit_substate(StackExitKind::Reverted);
                Capture::Exit((ExitReason::Revert(e), None, runtime.machine().return_value()))
            }
            ExitReason::Fatal(e) => {
                self.state_mut().metadata_mut().gasometer_mut().fail();
                self.fill_trace(&trace, false, None, pre_index);
                let _ = self.stack_executor_mut().exit_substate(StackExitKind::Failed);
                Capture::Exit((ExitReason::Fatal(e), None, Vec::new()))
            }
        }
    }

    // NB: This function is copy-pasted from upstream's call_inner
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
        let pre_index = self.state().trace_index;
        let trace = self.start_trace(
            code_address,
            input.clone(),
            transfer.as_ref().map(|x| x.value).unwrap_or_default(),
            false,
        );

        macro_rules! try_or_fail {
            ( $e:expr ) => {
                match $e {
                    Ok(v) => v,
                    Err(e) => {
                        self.fill_trace(&trace, false, None, pre_index);
                        return Capture::Exit((e.into(), Vec::new()))
                    }
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
        self.stack_executor_mut().enter_substate(gas_limit, is_static);
        self.state_mut().touch(context.address);

        if let Some(depth) = self.state().metadata().depth() {
            if depth > self.config().call_stack_limit {
                self.fill_trace(&trace, false, None, pre_index);
                let _ = self.stack_executor_mut().exit_substate(StackExitKind::Reverted);
                return Capture::Exit((ExitError::CallTooDeep.into(), Vec::new()))
            }
        }

        if let Some(transfer) = transfer {
            match self.state_mut().transfer(transfer) {
                Ok(()) => (),
                Err(e) => {
                    self.fill_trace(&trace, false, None, pre_index);
                    let _ = self.stack_executor_mut().exit_substate(StackExitKind::Reverted);
                    return Capture::Exit((ExitReason::Error(e), Vec::new()))
                }
            }
        }

        if let Some(result) = self.stack_executor().precompiles().execute(
            code_address,
            &input,
            Some(gas_limit),
            &context,
            is_static,
        ) {
            return match result {
                Ok(PrecompileOutput { exit_status, output, cost, logs }) => {
                    for Log { address, topics, data } in logs {
                        match self.log(address, topics, data) {
                            Ok(_) => continue,
                            Err(error) => {
                                self.fill_trace(&trace, false, Some(output.clone()), pre_index);
                                return Capture::Exit((ExitReason::Error(error), output))
                            }
                        }
                    }

                    let _ = self.state_mut().metadata_mut().gasometer_mut().record_cost(cost);
                    self.fill_trace(&trace, true, Some(output.clone()), pre_index);
                    let _ = self.stack_executor_mut().exit_substate(StackExitKind::Succeeded);
                    Capture::Exit((ExitReason::Succeed(exit_status), output))
                }
                Err(e) => {
                    let e = match e {
                        PrecompileFailure::Error { exit_status } => ExitReason::Error(exit_status),
                        PrecompileFailure::Revert { exit_status, .. } => {
                            ExitReason::Revert(exit_status)
                        }
                        PrecompileFailure::Fatal { exit_status } => ExitReason::Fatal(exit_status),
                    };
                    self.fill_trace(&trace, false, None, pre_index);
                    let _ = self.stack_executor_mut().exit_substate(StackExitKind::Failed);
                    Capture::Exit((e, Vec::new()))
                }
            }
        }

        // each cfg is about 200 bytes, is this a lot to clone? why does this error
        // not manifest upstream?
        let config = self.config().clone();
        let mut runtime;
        let reason = if self.state().debug_enabled {
            let code = Rc::new(code);
            runtime = Runtime::new(code.clone(), Rc::new(input), context, &config);
            self.handler_mut().debug_execute(&mut runtime, code_address, code, false)
        } else {
            runtime = Runtime::new(Rc::new(code), Rc::new(input), context, &config);
            self.execute(&mut runtime)
        };

        // // log::debug!(target: "evm", "Call execution using address {}: {:?}", code_address,
        // reason);

        match reason {
            ExitReason::Succeed(s) => {
                self.fill_trace(&trace, true, Some(runtime.machine().return_value()), pre_index);
                let _ = self.stack_executor_mut().exit_substate(StackExitKind::Succeeded);
                Capture::Exit((ExitReason::Succeed(s), runtime.machine().return_value()))
            }
            ExitReason::Error(e) => {
                self.fill_trace(&trace, false, Some(runtime.machine().return_value()), pre_index);
                let _ = self.stack_executor_mut().exit_substate(StackExitKind::Failed);
                Capture::Exit((ExitReason::Error(e), Vec::new()))
            }
            ExitReason::Revert(e) => {
                self.fill_trace(&trace, false, Some(runtime.machine().return_value()), pre_index);
                let _ = self.stack_executor_mut().exit_substate(StackExitKind::Reverted);
                Capture::Exit((ExitReason::Revert(e), runtime.machine().return_value()))
            }
            ExitReason::Fatal(e) => {
                self.fill_trace(&trace, false, Some(runtime.machine().return_value()), pre_index);
                self.state_mut().metadata_mut().gasometer_mut().fail();
                let _ = self.stack_executor_mut().exit_substate(StackExitKind::Failed);
                Capture::Exit((ExitReason::Fatal(e), Vec::new()))
            }
        }
    }
}

impl<'a, 'b, Back, Precom: 'b, ExecHandler> Handler
    for ExecutionHandlerWrapper<'a, 'b, Back, Precom, MemoryStackStateOwned<'a, Back>, ExecHandler>
where
    ExecHandler: ExecutionHandler<'a, 'b, Back, Precom, MemoryStackStateOwned<'a, Back>>,
    Back: Backend,
    Precom: PrecompileSet,
    // State: StackState<'a>,
{
    type CreateInterrupt = Infallible;
    type CreateFeedback = Infallible;
    type CallInterrupt = Infallible;
    type CallFeedback = Infallible;

    fn balance(&self, address: H160) -> U256 {
        self.handler().balance(address)
    }

    fn code_size(&self, address: H160) -> U256 {
        self.handler().code_size(address)
    }

    fn code_hash(&self, address: H160) -> H256 {
        self.handler().code_hash(address)
    }

    fn code(&self, address: H160) -> Vec<u8> {
        self.handler().code(address)
    }

    fn storage(&self, address: H160, index: H256) -> H256 {
        self.handler().storage(address, index)
    }

    fn original_storage(&self, address: H160, index: H256) -> H256 {
        self.handler().original_storage(address, index)
    }

    fn gas_left(&self) -> U256 {
        self.handler().gas_left()
    }

    fn gas_price(&self) -> U256 {
        self.handler().gas_price()
    }

    fn origin(&self) -> H160 {
        self.handler().origin()
    }

    fn block_hash(&self, number: U256) -> H256 {
        self.handler().block_hash(number)
    }

    fn block_number(&self) -> U256 {
        self.handler().block_number()
    }

    fn block_coinbase(&self) -> H160 {
        self.handler().block_coinbase()
    }

    fn block_timestamp(&self) -> U256 {
        self.handler().block_timestamp()
    }

    fn block_difficulty(&self) -> U256 {
        self.handler().block_difficulty()
    }

    fn block_gas_limit(&self) -> U256 {
        self.handler().block_gas_limit()
    }

    fn block_base_fee_per_gas(&self) -> U256 {
        self.handler().block_base_fee_per_gas()
    }

    fn chain_id(&self) -> U256 {
        self.handler().chain_id()
    }

    fn exists(&self, address: H160) -> bool {
        self.handler().exists(address)
    }

    fn deleted(&self, address: H160) -> bool {
        self.handler().deleted(address)
    }

    fn is_cold(&self, address: H160, index: Option<H256>) -> bool {
        self.handler().is_cold(address, index)
    }

    fn set_storage(&mut self, address: H160, index: H256, value: H256) -> Result<(), ExitError> {
        self.handler_mut().set_storage(address, index, value)
    }

    fn log(&mut self, address: H160, topics: Vec<H256>, data: Vec<u8>) -> Result<(), ExitError> {
        if self.state().trace_enabled {
            let index = self.state().trace_index;
            let node = &mut self.state_mut().traces.last_mut().expect("no traces").arena[index];
            node.ordering.push(LogCallOrder::Log(node.logs.len()));
            node.logs.push(RawLog { topics: topics.clone(), data: data.clone() });
        }

        if let Some(decoded) =
            convert_log(Log { address, topics: topics.clone(), data: data.clone() })
        {
            self.state_mut().all_logs.push(decoded);
        }

        if !self.state().expected_emits.is_empty() {
            // get expected emits
            let expected_emits = &mut self.state_mut().expected_emits;

            // do we have empty expected emits to fill?
            if let Some(next_expect_to_fill) =
                expected_emits.iter_mut().find(|expect| expect.log.is_none())
            {
                next_expect_to_fill.log =
                    Some(RawLog { topics: topics.clone(), data: data.clone() });
            } else {
                // no unfilled, grab next unfound
                // try to fill the first unfound
                if let Some(next_expect) = expected_emits.iter_mut().find(|expect| !expect.found) {
                    // unpack the log
                    if let Some(RawLog { topics: expected_topics, data: expected_data }) =
                        &next_expect.log
                    {
                        if expected_topics[0] == topics[0] {
                            // same event topic 0, topic length should be the same
                            let topics_match = topics
                                .iter()
                                .skip(1)
                                .enumerate()
                                .filter(|(i, _topic)| {
                                    // do we want to check?
                                    next_expect.checks[*i]
                                })
                                .all(|(i, topic)| topic == &expected_topics[i + 1]);

                            // check data
                            next_expect.found = if next_expect.checks[3] {
                                expected_data == &data && topics_match
                            } else {
                                topics_match
                            };
                        }
                    }
                }
            }
        }

        self.stack_executor_mut().log(address, topics, data)
    }

    fn mark_delete(&mut self, address: H160, target: H160) -> Result<(), ExitError> {
        self.handler_mut().mark_delete(address, target)
    }

    fn create(
        &mut self,
        caller: H160,
        scheme: CreateScheme,
        value: U256,
        init_code: Vec<u8>,
        target_gas: Option<u64>,
    ) -> Capture<(ExitReason, Option<H160>, Vec<u8>), Self::CreateInterrupt> {
        self.handler_mut().do_create(caller, scheme, value, init_code, target_gas)
    }

    fn call(
        &mut self,
        code_address: H160,
        transfer: Option<Transfer>,
        input: Vec<u8>,
        target_gas: Option<u64>,
        is_static: bool,
        context: Context,
    ) -> Capture<(ExitReason, Vec<u8>), Self::CallInterrupt> {
        self.handler_mut().do_call(code_address, transfer, input, target_gas, is_static, context)
    }

    fn pre_validate(
        &mut self,
        context: &Context,
        opcode: Opcode,
        stack: &Stack,
    ) -> Result<(), ExitError> {
        self.handler_mut().pre_validate(context, opcode, stack)
    }
}

// Forwards everything internally except for the transact_call which is overwritten.
impl<'a, 'b, Back, Precom: 'b, ExecHandler> SputnikExecutor<MemoryStackStateOwned<'a, Back>>
    for ExecutionHandlerWrapper<'a, 'b, Back, Precom, MemoryStackStateOwned<'a, Back>, ExecHandler>
where
    ExecHandler: ExecutionHandler<'a, 'b, Back, Precom, MemoryStackStateOwned<'a, Back>>,
    Back: Backend,
    Precom: PrecompileSet,
    // State: StackState<'a>,
{
    fn config(&self) -> &Config {
        self.handler().stack_executor().config()
    }

    fn state(&self) -> &MemoryStackStateOwned<'a, Back> {
        self.handler().stack_executor().state()
    }

    fn state_mut(&mut self) -> &mut MemoryStackStateOwned<'a, Back> {
        self.handler_mut().stack_executor_mut().state_mut()
    }

    fn expected_revert(&self) -> Option<&[u8]> {
        self.handler().stack_executor().state().expected_revert.as_deref()
    }

    fn set_tracing_enabled(&mut self, enabled: bool) -> bool {
        let curr = self.state_mut().trace_enabled;
        self.state_mut().trace_enabled = enabled;
        curr
    }

    fn tracing_enabled(&self) -> bool {
        self.state().trace_enabled
    }

    fn debug_calls(&self) -> Vec<DebugArena> {
        self.state().debug_steps.clone()
    }

    fn all_logs(&self) -> Vec<String> {
        self.handler().stack_executor().state().all_logs.clone()
    }

    fn gas_left(&self) -> U256 {
        // NB: We do this to avoid `function cannot return without recursing`
        U256::from(self.state().metadata().gasometer().gas())
    }

    fn gas_used(&self) -> U256 {
        // NB: We do this to avoid `function cannot return without recursing`
        U256::from(self.state().metadata().gasometer().total_used_gas())
    }

    fn gas_refund(&self) -> U256 {
        U256::from(self.state().metadata().gasometer().refunded_gas())
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
        // reset all_logs because its a new call
        self.state_mut().all_logs = vec![];

        let transaction_cost = gasometer::call_transaction_cost(&data, &access_list);
        match self.state_mut().metadata_mut().gasometer_mut().record_transaction(transaction_cost) {
            Ok(()) => (),
            Err(e) => return (e.into(), Vec::new()),
        }

        // Initialize initial addresses for EIP-2929
        if self.config().increase_state_access_gas {
            let addresses = core::iter::once(caller).chain(core::iter::once(address));
            self.state_mut().metadata_mut().access_addresses(addresses);

            self.stack_executor_mut().initialize_with_access_list(access_list);
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
            Capture::Exit((s, v)) => {
                self.state_mut().increment_call_index();

                // check if all expected calls were made
                if let Some((address, expecteds)) =
                    self.state().expected_calls.iter().find(|(_, expecteds)| !expecteds.is_empty())
                {
                    return (
                        ExitReason::Revert(ExitRevert::Reverted),
                        ethers::abi::encode(&[Token::String(format!(
                            "Expected a call to 0x{} with data {}, but got none",
                            address,
                            ethers::types::Bytes::from(expecteds[0].clone())
                        ))]),
                    )
                }

                if !self.state().expected_emits.is_empty() {
                    return (
                        ExitReason::Revert(ExitRevert::Reverted),
                        ethers::abi::encode(&[Token::String(
                            "Expected an emit, but no logs were emitted afterward".to_string(),
                        )]),
                    )
                }
                (s, v)
            }
            Capture::Trap(_) => {
                self.state_mut().increment_call_index();
                unreachable!()
            }
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
        // reset all_logs because its a new call
        self.state_mut().all_logs = Vec::new();

        let transaction_cost = gasometer::create_transaction_cost(&init_code, &access_list);
        match self.state_mut().metadata_mut().gasometer_mut().record_transaction(transaction_cost) {
            Ok(()) => (),
            Err(e) => return e.into(),
        };
        self.stack_executor_mut().initialize_with_access_list(access_list);

        match self.create_inner(
            caller,
            CreateScheme::Legacy { caller },
            value,
            init_code,
            Some(gas_limit),
            false,
        ) {
            Capture::Exit((s, _, _)) => {
                self.state_mut().increment_call_index();
                s
            }
            Capture::Trap(_) => {
                self.state_mut().increment_call_index();
                unreachable!()
            }
        }
    }

    fn create_address(&self, caller: CreateScheme) -> Address {
        self.handler().stack_executor().create_address(caller)
    }

    fn raw_logs(&self) -> Vec<RawLog> {
        let logs = self.state().substate.logs().to_vec();
        logs.into_iter().map(|log| RawLog { topics: log.topics, data: log.data }).collect()
    }

    fn traces(&self) -> Vec<CallTraceArena> {
        self.state().traces.clone()
    }

    fn reset_traces(&mut self) {
        self.state_mut().reset_traces();
    }

    fn logs(&self) -> Vec<String> {
        let logs = self.state().substate.logs().to_vec();
        logs.into_iter().filter_map(convert_log).chain(self.handler().additional_logs()).collect()
    }

    fn clear_logs(&mut self) {
        self.state_mut().substate.logs_mut().clear()
    }
}
