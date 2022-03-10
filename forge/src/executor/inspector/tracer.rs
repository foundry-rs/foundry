use super::logs::extract_log;
use crate::trace::{
    CallTrace, CallTraceArena, LogCallOrder, RawOrDecodedCall, RawOrDecodedLog,
    RawOrDecodedReturnData,
};
use bytes::Bytes;
use ethers::{
    types::{Address, U256},
    utils::{get_contract_address, get_create2_address},
};
use revm::{
    return_ok, CallInputs, CreateInputs, CreateScheme, Database, EVMData, Gas, Inspector,
    Interpreter, Return,
};

/// An inspector that collects call traces.
#[derive(Default, Debug)]
pub struct Tracer {
    pub trace_stack: Vec<usize>,
    pub traces: CallTraceArena,
}

impl Tracer {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn start_trace(
        &mut self,
        depth: usize,
        address: Address,
        data: Vec<u8>,
        value: U256,
        created: bool,
    ) {
        self.trace_stack.push(self.traces.push_trace(
            0,
            CallTrace {
                depth,
                address,
                created,
                data: RawOrDecodedCall::Raw(data),
                value,
                ..Default::default()
            },
        ));
    }

    pub fn fill_trace(&mut self, success: bool, cost: u64, output: Vec<u8>) {
        let trace = &mut self.traces.arena
            [self.trace_stack.pop().expect("more traces were filled than started")]
        .trace;
        trace.success = success;
        trace.gas_cost = cost;
        trace.output = RawOrDecodedReturnData::Raw(output);
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
        self.start_trace(
            data.subroutine.depth() as usize,
            call.contract,
            call.input.to_vec(),
            call.transfer.value,
            false,
        );

        (Return::Continue, Gas::new(call.gas_limit), Bytes::new())
    }

    fn step(
        &mut self,
        interpreter: &mut Interpreter,
        _: &mut EVMData<'_, DB>,
        _is_static: bool,
    ) -> Return {
        if let Some(log) = extract_log(interpreter) {
            let node = &mut self.traces.arena[*self.trace_stack.last().expect("no ongoing trace")];
            node.ordering.push(LogCallOrder::Log(node.logs.len()));
            node.logs.push(RawOrDecodedLog::Raw(log));
        }

        Return::Continue
    }

    fn call_end(
        &mut self,
        _: &mut EVMData<'_, DB>,
        _: &CallInputs,
        gas: Gas,
        status: Return,
        retdata: Bytes,
        _: bool,
    ) -> (Return, Gas, Bytes) {
        self.fill_trace(matches!(status, return_ok!()), gas.spend(), retdata.to_vec());

        (status, gas, retdata)
    }

    fn create(
        &mut self,
        data: &mut EVMData<'_, DB>,
        call: &CreateInputs,
    ) -> (Return, Option<Address>, Gas, Bytes) {
        let nonce = data.db.basic(call.caller).nonce;
        self.start_trace(
            data.subroutine.depth() as usize,
            match call.scheme {
                CreateScheme::Create => get_contract_address(call.caller, nonce),
                CreateScheme::Create2 { salt } => {
                    let mut buffer: [u8; 4 * 8] = [0; 4 * 8];
                    salt.to_big_endian(&mut buffer);
                    get_create2_address(call.caller, buffer, call.init_code.clone())
                }
            },
            call.init_code.to_vec(),
            call.value,
            true,
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
                .map_or(vec![], |code| code.to_vec()),
            None => vec![],
        };
        self.fill_trace(matches!(status, return_ok!()), gas.spend(), code);

        (status, address, gas, retdata)
    }
}
