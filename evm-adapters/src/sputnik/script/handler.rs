//! The handler that sits in between and intercepts script calls

use crate::sputnik::cheatcodes::memory_stackstate_owned::MemoryStackStateOwned;
use sputnik::{
    backend::Backend,
    executor::stack::{PrecompileSet, StackExecutor},
    ExitReason, ExitSucceed, Runtime,
};
use std::rc::Rc;

use crate::{call_tracing::CallTrace, sputnik::common::ExecutionHandler};
use ethers::types::Address;

pub type ScriptStackState<'config, Backend> = MemoryStackStateOwned<'config, Backend>;

pub type ScriptStackExecutor<'a, 'b, B, P> =
    ScriptHandler<StackExecutor<'a, 'b, ScriptStackState<'a, B>, P>>;

pub struct ScriptHandler<H> {
    handler: H,
    state: ScriptState,
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

    fn debug_execute(
        &mut self,
        _runtime: &mut Runtime,
        _address: Address,
        _code: Rc<Vec<u8>>,
        _creation: bool,
    ) -> ExitReason {
        ExitReason::Succeed(ExitSucceed::Returned)
    }
}

/// Tracks the state of the script that's currently being executed
#[derive(Debug)]
pub struct ScriptState {
    // TODO file handles etc
}
