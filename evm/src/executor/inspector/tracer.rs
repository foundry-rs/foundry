use crate::{
    executor::inspector::utils::{gas_used, get_create_address},
    trace::{
        CallTrace, CallTraceArena, LogCallOrder, RawOrDecodedCall, RawOrDecodedLog,
        RawOrDecodedReturnData,
    },
    CallKind,
};
use bytes::Bytes;
use ethers::{
    abi::RawLog,
    types::{Address, H256, U256},
};
use revm::{
    return_ok, CallInputs, CallScheme, CreateInputs, Database, EVMData, Gas, Inspector, Return,
};

/// An inspector that collects call traces.
#[derive(Default, Debug, Clone)]
pub struct Tracer {
    pub trace_stack: Vec<usize>,
    pub traces: CallTraceArena,
}

impl Tracer {
    pub fn start_trace(
        &mut self,
        depth: usize,
        address: Address,
        data: Vec<u8>,
        value: U256,
        kind: CallKind,
        caller: Address,
    ) {
        self.trace_stack.push(self.traces.push_trace(
            0,
            CallTrace {
                depth,
                address,
                kind,
                data: RawOrDecodedCall::Raw(data),
                value,
                status: Return::Continue,
                caller,
                ..Default::default()
            },
        ));
    }

    pub fn fill_trace(
        &mut self,
        status: Return,
        cost: u64,
        output: Vec<u8>,
        address: Option<Address>,
    ) {
        let success = matches!(status, return_ok!());
        let trace = &mut self.traces.arena
            [self.trace_stack.pop().expect("more traces were filled than started")]
        .trace;
        trace.status = status;
        trace.success = success;
        trace.gas_cost = cost;
        trace.output = RawOrDecodedReturnData::Raw(output);

        if let Some(address) = address {
            trace.address = address;
        }
    }
}

impl<DB> Inspector<DB> for Tracer
where
    DB: Database,
{
    fn log(&mut self, _: &mut EVMData<'_, DB>, _: &Address, topics: &[H256], data: &Bytes) {
        let node = &mut self.traces.arena[*self.trace_stack.last().expect("no ongoing trace")];
        node.ordering.push(LogCallOrder::Log(node.logs.len()));
        node.logs
            .push(RawOrDecodedLog::Raw(RawLog { topics: topics.to_vec(), data: data.to_vec() }));
    }

    fn call(
        &mut self,
        data: &mut EVMData<'_, DB>,
        call: &mut CallInputs,
        _: bool,
    ) -> (Return, Gas, Bytes) {
        let (from, to) = match call.context.scheme {
            CallScheme::DelegateCall | CallScheme::CallCode => {
                (call.context.address, call.context.code_address)
            }
            _ => (call.context.caller, call.context.address),
        };

        self.start_trace(
            data.subroutine.depth() as usize,
            to,
            call.input.to_vec(),
            call.transfer.value,
            call.context.scheme.into(),
            from,
        );

        (Return::Continue, Gas::new(call.gas_limit), Bytes::new())
    }

    fn call_end(
        &mut self,
        data: &mut EVMData<'_, DB>,
        _call: &CallInputs,
        gas: Gas,
        status: Return,
        retdata: Bytes,
        _: bool,
    ) -> (Return, Gas, Bytes) {
        self.fill_trace(
            status,
            gas_used(data.env.cfg.spec_id, gas.spend(), gas.refunded() as u64),
            retdata.to_vec(),
            None,
        );

        (status, gas, retdata)
    }

    fn create(
        &mut self,
        data: &mut EVMData<'_, DB>,
        call: &mut CreateInputs,
    ) -> (Return, Option<Address>, Gas, Bytes) {
        // TODO: Does this increase gas cost?
        data.subroutine.load_account(call.caller, data.db);
        let nonce = data.subroutine.account(call.caller).info.nonce;
        self.start_trace(
            data.subroutine.depth() as usize,
            get_create_address(call, nonce),
            call.init_code.to_vec(),
            call.value,
            call.scheme.into(),
            call.caller,
        );

        (Return::Continue, None, Gas::new(call.gas_limit), Bytes::new())
    }

    fn create_end(
        &mut self,
        data: &mut EVMData<'_, DB>,
        _: &CreateInputs,
        status: Return,
        address: Option<Address>,
        gas: Gas,
        retdata: Bytes,
    ) -> (Return, Option<Address>, Gas, Bytes) {
        let code = match address {
            Some(address) => data
                .subroutine
                .account(address)
                .info
                .code
                .as_ref()
                .map_or(vec![], |code| code.bytes()[..code.len()].to_vec()),
            None => vec![],
        };
        self.fill_trace(
            status,
            gas_used(data.env.cfg.spec_id, gas.spend(), gas.refunded() as u64),
            code,
            address,
        );

        (status, address, gas, retdata)
    }
}
