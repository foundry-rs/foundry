use crate::{
    debugger::{DebugArena, DebugNode, DebugStep, Instruction},
    executor::CHEATCODE_ADDRESS,
};
use bytes::Bytes;
use ethers::{
    types::Address,
    utils::{get_contract_address, get_create2_address},
};
use revm::{
    CallInputs, CreateInputs, CreateScheme, Database, EVMData, Gas, Inspector, Interpreter, Memory,
    OpCode, Return,
};
use std::collections::BTreeMap;

/// An inspector that collects debug nodes on every step of the interpreter.
#[derive(Default, Debug)]
pub struct Debugger {
    /// The arena of [DebugNode]s
    pub arena: DebugArena,
    /// The ID of the current [DebugNode].
    pub head: usize,
    /// A mapping of program counters to instruction counters.
    ///
    /// The program counter keeps track of where we are in the contract bytecode as a whole,
    /// including push bytes, while the instruction counter ignores push bytes.
    ///
    /// The instruction counter is used in Solidity source maps.
    pub ic_map: BTreeMap<Address, BTreeMap<usize, usize>>,
}

impl Debugger {
    pub fn new() -> Self {
        Default::default()
    }

    /// Builds the instruction counter map for the given bytecode.
    // TODO: Some of the same logic is performed in REVM, but then later discarded. We should
    // investigate if we can reuse it
    pub fn build_ic_map(&mut self, address: &Address, code: &Bytes) {
        let mut ic_map: BTreeMap<usize, usize> = BTreeMap::new();

        let mut i = 0;
        let mut cumulative_push_size = 0;
        while i < code.len() {
            let op = code[i];
            ic_map.insert(i, i - cumulative_push_size);
            match OpCode::is_push(op) {
                Some(push_size) => {
                    // Skip the push bytes
                    i += push_size as usize;
                    cumulative_push_size += push_size as usize;
                }
                None => (),
            }
            i += 1;
        }

        self.ic_map.insert(*address, ic_map);
    }

    /// Enters a new execution context.
    pub fn enter(&mut self, depth: usize, address: Address, creation: bool) {
        self.head =
            self.arena.push_node(DebugNode { depth, address, creation, ..Default::default() });
    }

    /// Exits the current execution context, replacing it with the previous one.
    pub fn exit(&mut self) {
        if let Some(parent_id) = self.arena.arena[self.head].parent {
            let DebugNode { depth, address, creation, .. } = self.arena.arena[parent_id];
            self.head =
                self.arena.push_node(DebugNode { depth, address, creation, ..Default::default() });
        }
    }

    /// Records a debug step in the current execution context.
    // TODO: Interpreter is only taken as a mutable borrow here because `Interpreter::gas` takes
    // `&mut self` by mistake.
    pub fn record_debug_step(&mut self, interpreter: &mut Interpreter) {
        let pc = interpreter.program_counter();
        let push_size = OpCode::is_push(interpreter.contract.code[pc]).map(|size| size as usize);
        let push_bytes = push_size.as_ref().map(|push_size| {
            let start = pc + 1;
            let end = start + push_size;
            interpreter.contract.code[start..end].to_vec()
        });

        self.arena.arena[self.head].steps.push(DebugStep {
            pc,
            stack: interpreter.stack().data().clone(),
            memory: interpreter.memory.clone(),
            instruction: Instruction::OpCode(interpreter.contract.code[pc]),
            push_bytes,
            ic: *self
                .ic_map
                .get(&interpreter.contract().address)
                .expect("no instruction counter map")
                .get(&pc)
                .expect("unknown ic for pc"),
            // TODO: The number reported here is off
            total_gas_used: interpreter.gas().spend(),
        });
    }
}

impl<DB> Inspector<DB> for Debugger
where
    DB: Database,
{
    fn call(
        &mut self,
        data: &mut EVMData<'_, DB>,
        call: &CallInputs,
        _: bool,
    ) -> (Return, Gas, Bytes) {
        self.enter(data.subroutine.depth() as usize, call.contract, false);
        if call.contract == *CHEATCODE_ADDRESS {
            self.arena.arena[self.head].steps.push(DebugStep {
                memory: Memory::new(),
                instruction: Instruction::Cheatcode(
                    call.input[0..4].try_into().expect("malformed cheatcode call"),
                ),
                ..Default::default()
            });
        }

        (Return::Continue, Gas::new(call.gas_limit), Bytes::new())
    }

    fn initialize_interp(
        &mut self,
        interp: &mut Interpreter,
        _: &mut EVMData<'_, DB>,
        _: bool,
    ) -> Return {
        // TODO: This is rebuilt for all contracts every time. We should only run this if the IC
        // map for a given address does not exist, *but* we need to account for the fact that the
        // code given by the interpreter may either be the contract init code, or the runtime code.
        self.build_ic_map(&interp.contract().address, &interp.contract().code);
        Return::Continue
    }

    fn step(
        &mut self,
        interpreter: &mut Interpreter,
        _: &mut EVMData<'_, DB>,
        _is_static: bool,
    ) -> Return {
        self.record_debug_step(interpreter);
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
        call: &CreateInputs,
    ) -> (Return, Option<Address>, Gas, Bytes) {
        // TODO: Does this increase gas cost?
        data.subroutine.load_account(call.caller, data.db);
        let nonce = data.subroutine.account(call.caller).info.nonce;
        let address = match call.scheme {
            CreateScheme::Create => get_contract_address(call.caller, nonce),
            CreateScheme::Create2 { salt } => {
                let mut buffer: [u8; 4 * 8] = [0; 4 * 8];
                salt.to_big_endian(&mut buffer);
                get_create2_address(call.caller, buffer, call.init_code.clone())
            }
        };
        self.enter(data.subroutine.depth() as usize, address, true);

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
