//! Additional traits and common implementation to create a custom `SputnikExecutor`

use crate::{
    call_tracing::{CallTrace, CallTraceArena},
    sputnik::SputnikExecutor,
};

use std::io::Read;

use sputnik::{
    backend::Backend,
    executor::stack::{
        Log, PrecompileFailure, PrecompileOutput, PrecompileSet, StackExecutor, StackExitKind,
        StackState,
    },
    Capture, Config, Context, CreateScheme, ExitError, ExitReason, Handler, Opcode, Runtime, Stack,
    Transfer,
};
use std::rc::Rc;

use ethers::{
    abi::RawLog,
    signers::Signer,
    types::{Address, H160, H256, U256},
};

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
    handler: ExecHandler,
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
        self.state_mut().trace_index = pre_trace_index;
        if let Some(new_trace) = new_trace {
            let used_gas = self.stack_executor().used_gas();
            let trace = &mut self.state_mut().trace_mut().arena[new_trace.idx].trace;
            trace.output = output.unwrap_or_default();
            trace.cost = used_gas;
            trace.success = success;
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

    fn log(&mut self, _address: H160, _topics: Vec<H256>, _data: Vec<u8>) -> Result<(), ExitError> {
        todo!()
    }

    fn mark_delete(&mut self, address: H160, target: H160) -> Result<(), ExitError> {
        self.handler_mut().mark_delete(address, target)
    }

    fn create(
        &mut self,
        _caller: H160,
        _scheme: CreateScheme,
        _value: U256,
        _init_code: Vec<u8>,
        _target_gas: Option<u64>,
    ) -> Capture<(ExitReason, Option<H160>, Vec<u8>), Self::CreateInterrupt> {
        todo!()
    }

    fn call(
        &mut self,
        _code_address: H160,
        _transfer: Option<Transfer>,
        _input: Vec<u8>,
        _target_gas: Option<u64>,
        _is_static: bool,
        _context: Context,
    ) -> Capture<(ExitReason, Vec<u8>), Self::CallInterrupt> {
        todo!()
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
        _caller: H160,
        _address: H160,
        _value: U256,
        _data: Vec<u8>,
        _gas_limit: u64,
        _access_list: Vec<(H160, Vec<H256>)>,
    ) -> (ExitReason, Vec<u8>) {
        todo!()
    }

    fn transact_create(
        &mut self,
        _caller: H160,
        _value: U256,
        _data: Vec<u8>,
        _gas_limit: u64,
        _access_list: Vec<(H160, Vec<H256>)>,
    ) -> ExitReason {
        todo!()
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
