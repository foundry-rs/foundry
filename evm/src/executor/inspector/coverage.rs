use crate::{coverage::HitMaps, executor::inspector::utils::get_create_address};
use bytes::Bytes;
use ethers::types::Address;
use revm::{
    opcode, spec_opcode_gas, CallInputs, CreateInputs, Database, EVMData, Gas, Inspector,
    Interpreter, Return, SpecId,
};
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
    /// Builds the instruction counter map for the given bytecode.
    // TODO: Some of the same logic is performed in REVM, but then later discarded. We should
    // investigate if we can reuse it
    // TODO: Duplicate code of the debugger inspector
    pub fn build_ic_map(&mut self, spec: SpecId, code: &Bytes) {
        if let Some(context) = self.context.last() {
            let opcode_infos = spec_opcode_gas(spec);
            let mut ic_map: BTreeMap<usize, usize> = BTreeMap::new();

            let mut i = 0;
            let mut cumulative_push_size = 0;
            while i < code.len() {
                let op = code[i];
                ic_map.insert(i, i - cumulative_push_size);
                if opcode_infos[op as usize].is_push {
                    // Skip the push bytes.
                    //
                    // For more context on the math, see: https://github.com/bluealloy/revm/blob/007b8807b5ad7705d3cacce4d92b89d880a83301/crates/revm/src/interpreter/contract.rs#L114-L115
                    i += (op - opcode::PUSH1 + 1) as usize;
                    cumulative_push_size += (op - opcode::PUSH1 + 1) as usize;
                }
                i += 1;
            }

            self.ic_map.insert(*context, ic_map);
        }
    }

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
        self.build_ic_map(data.env.cfg.spec_id, &interp.contract().code);
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
