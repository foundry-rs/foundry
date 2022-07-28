use crate::{
    coverage::HitMaps, executor::inspector::utils::get_create_address, utils::build_ic_map,
};
use bytes::Bytes;
use ethers::types::Address;
use revm::{CallInputs, CreateInputs, Database, EVMData, Gas, Inspector, Interpreter, Return};
use std::collections::BTreeMap;

#[derive(Default, Debug)]
pub struct CoverageCollector {
    /// Maps that track instruction hit data.
    pub maps: HitMaps,

    /// The execution addresses, with the topmost one being the current address.
    context: Vec<Address>,

    /// A mapping of program counters to instruction counters.
    ///
    /// The program counter keeps track of where we are in the contract bytecode as a whole,
    /// including push bytes, while the instruction counter ignores push bytes.
    ///
    /// The instruction counter is used in Solidity source maps.
    pub ic_map: BTreeMap<Address, BTreeMap<usize, usize>>,
}

impl CoverageCollector {
    pub fn enter(&mut self, address: Address) {
        self.context.push(address);
    }

    pub fn exit(&mut self) {
        self.context.pop();
    }
}

impl<DB> Inspector<DB> for CoverageCollector
where
    DB: Database,
{
    fn call(
        &mut self,
        _: &mut EVMData<'_, DB>,
        call: &mut CallInputs,
        _: bool,
    ) -> (Return, Gas, Bytes) {
        self.enter(call.context.code_address);

        (Return::Continue, Gas::new(call.gas_limit), Bytes::new())
    }

    // TODO: Duplicate code of the debugger inspector
    fn initialize_interp(
        &mut self,
        interp: &mut Interpreter,
        data: &mut EVMData<'_, DB>,
        _: bool,
    ) -> Return {
        // TODO: This is rebuilt for all contracts every time. We should only run this if the IC
        // map for a given address does not exist, *but* we need to account for the fact that the
        // code given by the interpreter may either be the contract init code, or the runtime code.
        if let Some(context) = self.context.last() {
            self.ic_map
                .insert(*context, build_ic_map(data.env.cfg.spec_id, &interp.contract().code));
        }
        Return::Continue
    }

    // TODO: Don't collect coverage for test contract if possible
    fn step(
        &mut self,
        interpreter: &mut Interpreter,
        _: &mut EVMData<'_, DB>,
        _is_static: bool,
    ) -> Return {
        let pc = interpreter.program_counter();
        if let Some(context) = self.context.last() {
            if let Some(ic) = self.ic_map.get(context).and_then(|ic_map| ic_map.get(&pc)) {
                let map = self.maps.entry(*context).or_default();

                map.hit(*ic);
            }
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
        self.exit();

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
        self.enter(get_create_address(call, nonce));

        (Return::Continue, None, Gas::new(call.gas_limit), Bytes::new())
    }

    fn create_end(
        &mut self,
        _: &mut EVMData<'_, DB>,
        _: &CreateInputs,
        status: Return,
        address: Option<Address>,
        gas: Gas,
        retdata: Bytes,
    ) -> (Return, Option<Address>, Gas, Bytes) {
        self.exit();

        (status, address, gas, retdata)
    }
}
