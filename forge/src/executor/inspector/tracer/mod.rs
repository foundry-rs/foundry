pub mod trace;

use bytes::Bytes;
use ethers::{
    types::Address,
    utils::{get_contract_address, get_create2_address},
};
use revm::{
    return_ok, CallInputs, CreateInputs, CreateScheme, Database, EVMData, Gas, Inspector, Return,
};
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

    fn create(
        &mut self,
        data: &mut EVMData<'_, DB>,
        call: &CreateInputs,
    ) -> (Return, Option<Address>, Gas, Bytes) {
        let nonce = data.db.basic(call.caller).nonce;
        let mut trace = CallTrace {
            depth: data.subroutine.depth() as usize,
            addr: match call.scheme {
                CreateScheme::Create => get_contract_address(call.caller, nonce),
                CreateScheme::Create2 { salt } => {
                    let mut buffer: [u8; 4 * 8] = [0; 4 * 8];
                    salt.to_big_endian(&mut buffer);
                    get_create2_address(call.caller, buffer, call.init_code.clone())
                }
            },
            created: true,
            data: call.init_code.to_vec(),
            value: call.value,
            // TODO: Labels
            ..Default::default()
        };
        self.traces.push_trace(0, &mut trace);
        self.current_trace = trace;

        (Return::Continue, None, Gas::new(call.gas_limit), Bytes::new())
    }

    fn create_end(
        &mut self,
        _: &mut EVMData<'_, DB>,
        _: &CreateInputs,
        status: Return,
        address: Option<Address>,
        remaining_gas: Gas,
        retdata: Bytes,
    ) -> (Return, Option<Address>, Gas, Bytes) {
        self.current_trace.output = retdata.to_vec();
        // TODO
        self.current_trace.cost = 0;
        self.current_trace.success = matches!(status, return_ok!());

        (status, address, remaining_gas, retdata)
    }
}
