//! The handler that sits in between and intercepts script calls

pub mod fs;
use fs::ForgeFsCalls;

use crate::sputnik::cheatcodes::memory_stackstate_owned::MemoryStackStateOwned;
use ethers::abi::AbiDecode;
use sputnik::{
    backend::Backend,
    executor::stack::{PrecompileSet, StackExecutor},
    Capture, Context, ExitReason, ExitSucceed, Handler, Runtime, Transfer,
};
use std::{convert::Infallible, rc::Rc};

use crate::{
    call_tracing::CallTrace,
    sputnik::{
        common::{ExecutionHandler, ExecutionHandlerWrapper, RuntimeExecutionHandler},
        script::{handler::fs::FsManager, FORGE_SCRIPT_ADDRESS},
        utils::evm_error,
    },
};
use ethers::types::Address;
use ethers_core::types::H160;

pub type ScriptStackState<'config, Backend> = MemoryStackStateOwned<'config, Backend>;

pub type ScriptStackExecutor<'a, 'b, B, P> =
    ScriptHandler<StackExecutor<'a, 'b, ScriptStackState<'a, B>, P>>;

impl<'a, 'b, Back: Backend, Pre: PrecompileSet + 'b> ScriptStackExecutor<'a, 'b, Back, Pre> {
    /// This allows has to turn this mutable ref into a type that implements `sputnik::Handler` to
    /// execute runtime operations
    fn as_handler<'handler>(
        &'handler mut self,
    ) -> RuntimeExecutionHandler<'handler, 'a, 'b, Back, Pre, ScriptStackState<'a, Back>, Self, Back>
    {
        RuntimeExecutionHandler::new(self)
    }
}

/// The wrapper type that takes the `ScriptStackExecutor` and implements all `SputnikExecutor`
/// functions
pub type ScriptExecutionHandler<'a, 'b, Back, Pre> = ExecutionHandlerWrapper<
    'a,
    'b,
    Back,
    Pre,
    ScriptStackState<'a, Back>,
    ScriptStackExecutor<'a, 'b, Back, Pre>,
>;

#[derive(Debug)]
pub struct ScriptHandler<H> {
    handler: H,
    state: ScriptState,
}

impl<H> ScriptHandler<H> {
    pub fn new(handler: H) -> Self {
        Self { handler, state: Default::default() }
    }
}

impl<'a, 'b, Back, Precom: 'b> ExecutionHandler<'a, 'b, Back, Precom, ScriptStackState<'a, Back>>
    for ScriptStackExecutor<'a, 'b, Back, Precom>
where
    Back: Backend,
    Precom: PrecompileSet,
{
    fn stack_executor(&self) -> &StackExecutor<'a, 'b, ScriptStackState<'a, Back>, Precom> {
        &self.handler
    }

    fn stack_executor_mut(
        &mut self,
    ) -> &mut StackExecutor<'a, 'b, ScriptStackState<'a, Back>, Precom> {
        &mut self.handler
    }

    fn is_tracing_enabled(&self) -> bool {
        false
    }

    fn debug_execute(
        &mut self,
        _runtime: &mut Runtime,
        _address: Address,
        _code: Rc<Vec<u8>>,
        _creation: bool,
    ) -> ExitReason {
        ExitReason::Succeed(ExitSucceed::Returned)
    }

    fn fill_trace(
        &mut self,
        new_trace: &Option<CallTrace>,
        success: bool,
        output: Option<Vec<u8>>,
        pre_trace_index: usize,
    ) {
        self.stack_executor_mut().state_mut().trace_index = pre_trace_index;
        if let Some(new_trace) = new_trace {
            let used_gas = self.stack_executor().used_gas();
            let trace =
                &mut self.stack_executor_mut().state_mut().trace_mut().arena[new_trace.idx].trace;
            trace.output = output.unwrap_or_default();
            trace.cost = used_gas;
            trace.success = success;
        }
    }

    fn do_call(
        &mut self,
        code_address: H160,
        transfer: Option<Transfer>,
        input: Vec<u8>,
        target_gas: Option<u64>,
        is_static: bool,
        context: Context,
    ) -> Capture<(ExitReason, Vec<u8>), Infallible> {
        if code_address == *FORGE_SCRIPT_ADDRESS {
            if let Ok(call) = ForgeFsCalls::decode(&input) {
                return self.on_fs_call(call, context.caller)
            }
            evm_error("failed to decode forge script call")
        } else {
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
}

/// Tracks the state of the script that's currently being executed
#[derive(Debug, Default)]
pub struct ScriptState {
    /// manages the `fs` related state
    fs: FsManager,
}
