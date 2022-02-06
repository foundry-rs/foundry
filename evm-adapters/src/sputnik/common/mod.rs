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
mod macros;
use macros::{call_inner, create_inner, start_trace};

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
// impl<'a, 'b, Back, Pre: 'b> ExecutionHandler<'a, 'b, Back, Pre, MyStackState<'a, Back>>
// for MyStackExecutor<'a, 'b, Back, Pre>
//     where
//         Back: Backend,
//         Pre: PrecompileSet,
// {
//     fn stack_executor(&self) -> &StackExecutor<'a, 'b, MyStackState<'a, Back>, Pre> {
//         &self.handler
//     }
//
//     fn stack_executor_mut(
//         &mut self,
//     ) -> &mut StackExecutor<'a, 'b, MyStackState<'a, Back>, Pre> {
//         &mut self.handler
//     }
//
//     fn is_tracing_enabled(&self) -> bool {
//         false
//     }
//
//     fn debug_execute(
//         &mut self,
//         _runtime: &mut Runtime,
//         _address: Address,
//         _code: Rc<Vec<u8>>,
//         _creation: bool,
//     ) -> ExitReason {
//         ExitReason::Succeed(ExitSucceed::Returned)
//     }
// }
// ```
pub trait ExecutionHandler<'a, 'b, Back, Pre: 'b, State>
where
    Back: Backend,
    Pre: PrecompileSet,
    State: StackState<'a>,
{
    // /// This allows has to turn this mutable ref into a type that implements `sputnik::Handler`
    // to /// execute runtime operations
    // fn as_handler<'handler>(
    //     &'handler mut self,
    // ) -> RuntimeExecutionHandler<'handler, 'a, 'b, Back, Pre, State, Self>
    // where
    //     Self: Sized,
    // {
    //     RuntimeExecutionHandler::new(self)
    // }

    /// returns the wrapper sputnik `StackExecutor`
    fn stack_executor(&self) -> &StackExecutor<'a, 'b, State, Pre>;

    /// returns the wrapper sputnik `StackExecutor`
    fn stack_executor_mut(&mut self) -> &mut StackExecutor<'a, 'b, State, Pre>;

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

/// Yet another helper type that provides all necessary runtime operations for which both the
/// `ExecutionHandlerWrapper` and the actual implementer need access to
pub struct RuntimeExecutionHandler<
    'handler,
    'a,
    'b,
    Back,
    Pre: 'b,
    State,
    ExecHandler,
    HandlerBackend,
> where
    Back: Backend,
    Pre: PrecompileSet,
    State: StackState<'a>,
    ExecHandler: ExecutionHandler<'a, 'b, Back, Pre, State>,
    HandlerBackend: Backend,
{
    /// The wrapped `ExecutionHandler`
    handler: &'handler mut ExecHandler,
    // this is necessary because of all the unconstrained trait generics...
    _marker: PhantomData<(&'a (), State, &'b (), Back, Pre, HandlerBackend)>,
}

impl<'handler, 'a, 'b, Back, Pre: 'b, State, ExecHandler, HandlerBackend>
    RuntimeExecutionHandler<'handler, 'a, 'b, Back, Pre, State, ExecHandler, HandlerBackend>
where
    ExecHandler: ExecutionHandler<'a, 'b, Back, Pre, State>,
    Back: Backend,
    Pre: PrecompileSet,
    State: StackState<'a>,
    HandlerBackend: Backend,
{
    pub fn new(handler: &'handler mut ExecHandler) -> Self {
        Self { handler, _marker: Default::default() }
    }

    pub fn handler(&self) -> &ExecHandler {
        self.handler
    }

    pub fn handler_mut(&mut self) -> &mut ExecHandler {
        &mut self.handler
    }

    fn stack_executor(&self) -> &StackExecutor<'a, 'b, State, Pre> {
        self.handler().stack_executor()
    }

    fn stack_executor_mut(&mut self) -> &mut StackExecutor<'a, 'b, State, Pre> {
        self.handler_mut().stack_executor_mut()
    }

    fn state(&self) -> &State {
        self.handler().stack_executor().state()
    }

    fn state_mut(&mut self) -> &mut State {
        self.handler_mut().stack_executor_mut().state_mut()
    }
}

impl<'handler, 'a, 'b, Back, Pre: 'b, ExecHandler, HandlerBackend>
    RuntimeExecutionHandler<
        'handler,
        'a,
        'b,
        Back,
        Pre,
        MemoryStackStateOwned<'a, HandlerBackend>,
        ExecHandler,
        HandlerBackend,
    >
where
    ExecHandler: ExecutionHandler<'a, 'b, Back, Pre, MemoryStackStateOwned<'a, HandlerBackend>>,
    Back: Backend,
    Pre: PrecompileSet,
    HandlerBackend: Backend,
{
    fn config(&self) -> &Config {
        self.handler().stack_executor().config()
    }

    // NB: This function is copy-pasted from upstream's `execute`, adjusted so that we call the
    // Runtime with our own handler
    pub fn execute(&mut self, runtime: &mut Runtime) -> ExitReason {
        match runtime.run(self) {
            Capture::Exit(s) => s,
            Capture::Trap(_) => unreachable!("Trap is Infallible"),
        }
    }

    pub fn start_trace(
        &mut self,
        address: H160,
        input: Vec<u8>,
        transfer: U256,
        creation: bool,
    ) -> Option<CallTrace> {
        start_trace!(self, address, input, transfer, creation)
    }

    pub fn fill_trace(
        &mut self,
        new_trace: &Option<CallTrace>,
        success: bool,
        output: Option<Vec<u8>>,
        pre_trace_index: usize,
    ) {
        self.handler.fill_trace(new_trace, success, output, pre_trace_index)
    }

    pub fn create_address(&self, caller: CreateScheme) -> Address {
        self.handler().stack_executor().create_address(caller)
    }

    // NB: This function is copy-pasted from upstream's create_inner
    pub fn create_inner(
        &mut self,
        caller: H160,
        scheme: CreateScheme,
        value: U256,
        init_code: Vec<u8>,
        target_gas: Option<u64>,
        take_l64: bool,
    ) -> Capture<(ExitReason, Option<H160>, Vec<u8>), Infallible> {
        create_inner!(self, caller, scheme, value, init_code, target_gas, take_l64)
    }

    // NB: This function is copy-pasted from upstream's call_inner
    #[allow(clippy::too_many_arguments)]
    pub fn call_inner(
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
        call_inner!(
            self,
            code_address,
            transfer,
            input,
            target_gas,
            is_static,
            take_l64,
            take_stipend,
            context
        )
    }
}

impl<'handler, 'a, 'b, Back, Pre: 'b, ExecHandler, HandlerBackend> Handler
    for RuntimeExecutionHandler<
        'handler,
        'a,
        'b,
        Back,
        Pre,
        MemoryStackStateOwned<'a, HandlerBackend>,
        ExecHandler,
        HandlerBackend,
    >
where
    ExecHandler: ExecutionHandler<'a, 'b, Back, Pre, MemoryStackStateOwned<'a, HandlerBackend>>,
    Back: Backend,
    Pre: PrecompileSet,
    HandlerBackend: Backend,
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
        macros::log!(self, address, topics, data)
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

/// This wrapper type is necessary as we can't implement foreign traits for traits (Handler for
/// ExecutionHandler)
pub struct ExecutionHandlerWrapper<'a, 'b, Back, Pre: 'b, State, ExecHandler>
where
    Back: Backend,
    Pre: PrecompileSet,
    State: StackState<'a>,
    ExecHandler: ExecutionHandler<'a, 'b, Back, Pre, State>,
{
    /// The wrapped `ExecutionHandler`
    handler: ExecHandler,
    // this is necessary because of all the unconstrained trait generics...
    _marker: PhantomData<(&'a (), State, &'b (), Back, Pre)>,
}

impl<'a, 'b, Back, Pre: 'b, State, ExecHandler>
    ExecutionHandlerWrapper<'a, 'b, Back, Pre, State, ExecHandler>
where
    Back: Backend,
    Pre: PrecompileSet,
    State: StackState<'a>,
    ExecHandler: ExecutionHandler<'a, 'b, Back, Pre, State>,
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

    fn stack_executor(&self) -> &StackExecutor<'a, 'b, State, Pre> {
        self.handler().stack_executor()
    }

    fn stack_executor_mut(&mut self) -> &mut StackExecutor<'a, 'b, State, Pre> {
        self.handler_mut().stack_executor_mut()
    }
}

impl<'a, 'b, Back, Pre: 'b, ExecHandler, HandlerBack>
    ExecutionHandlerWrapper<'a, 'b, Back, Pre, MemoryStackStateOwned<'a, HandlerBack>, ExecHandler>
where
    ExecHandler: ExecutionHandler<'a, 'b, Back, Pre, MemoryStackStateOwned<'a, HandlerBack>>,
    Back: Backend,
    Pre: PrecompileSet,
    HandlerBack: Backend,
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
        start_trace!(self, address, input, transfer, creation)
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
        create_inner!(self, caller, scheme, value, init_code, target_gas, take_l64)
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
        call_inner!(
            self,
            code_address,
            transfer,
            input,
            target_gas,
            is_static,
            take_l64,
            take_stipend,
            context
        )
    }
}

impl<'a, 'b, Back, Pre: 'b, ExecHandler, HandlerBack> Handler
    for ExecutionHandlerWrapper<
        'a,
        'b,
        Back,
        Pre,
        MemoryStackStateOwned<'a, HandlerBack>,
        ExecHandler,
    >
where
    ExecHandler: ExecutionHandler<'a, 'b, Back, Pre, MemoryStackStateOwned<'a, HandlerBack>>,
    Back: Backend,
    Pre: PrecompileSet,
    HandlerBack: Backend,
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
        macros::log!(self, address, topics, data)
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
impl<'a, 'b, Back, Pre: 'b, ExecHandler, HandlerBack>
    SputnikExecutor<MemoryStackStateOwned<'a, HandlerBack>>
    for ExecutionHandlerWrapper<
        'a,
        'b,
        Back,
        Pre,
        MemoryStackStateOwned<'a, HandlerBack>,
        ExecHandler,
    >
where
    ExecHandler: ExecutionHandler<'a, 'b, Back, Pre, MemoryStackStateOwned<'a, HandlerBack>>,
    Back: Backend,
    Pre: PrecompileSet,
    HandlerBack: Backend,
{
    fn config(&self) -> &Config {
        self.handler().stack_executor().config()
    }

    fn state(&self) -> &MemoryStackStateOwned<'a, HandlerBack> {
        self.handler().stack_executor().state()
    }

    fn state_mut(&mut self) -> &mut MemoryStackStateOwned<'a, HandlerBack> {
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
