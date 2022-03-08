pub mod trace;

use bytes::Bytes;
use revm::{return_ok, CallInputs, Database, EVMData, Gas, Inspector, Return};
use trace::{CallTrace, CallTraceArena};

#[derive(Default)]
pub struct Tracer {
    current_trace: CallTrace,
    pub traces: CallTraceArena,
}

impl Tracer {
    pub fn new() -> Self {
        Default::default()
    }
}

impl<DB> Inspector<DB> for Tracer
where
    DB: Database,
{
    fn call(
        &mut self,
        data: &mut EVMData<'_, DB>,
        call: &CallInputs,
        _: bool,
    ) -> (Return, Gas, Bytes) {
        let mut trace = CallTrace {
            depth: data.subroutine.depth() as usize,
            addr: call.contract,
            created: false,
            data: call.input.to_vec(),
            value: call.transfer.value,
            // TODO: Labels
            ..Default::default()
        };
        self.traces.push_trace(0, &mut trace);
        self.current_trace = trace;

        (Return::Continue, Gas::new(call.gas_limit), Bytes::new())
    }

    fn call_end(
        &mut self,
        _: &mut EVMData<'_, DB>,
        _: &CallInputs,
        remaining_gas: Gas,
        status: Return,
        retdata: Bytes,
        _: bool,
    ) -> (Return, Gas, Bytes) {
        self.current_trace.output = retdata.to_vec();
        // TODO
        self.current_trace.cost = 0;
        self.current_trace.success = matches!(status, return_ok!());

        (status, remaining_gas, retdata)
    }
}
