use crate::{
    debug::{DebugArena, DebugNode, DebugStep, Instruction},
    error::SolError,
    executor::{
        backend::DatabaseExt,
        inspector::utils::{gas_used, get_create_address},
        CHEATCODE_ADDRESS,
    },
    CallKind,
};
use bytes::Bytes;
use ethers::types::Address;
use revm::{
    opcode, spec_opcode_gas, CallInputs, CreateInputs, EVMData, Gas, GasInspector, Inspector,
    Interpreter, Memory, Return,
};
use std::{cell::RefCell, rc::Rc};

/// An inspector that collects debug nodes on every step of the interpreter.
#[derive(Debug)]
pub struct Debugger {
    /// The arena of [DebugNode]s
    pub arena: DebugArena,
    /// The ID of the current [DebugNode].
    pub head: usize,
    /// The current execution address.
    pub context: Address,

    gas_inspector: Rc<RefCell<GasInspector>>,
}

impl Debugger {
    pub fn new(gas_inspector: Rc<RefCell<GasInspector>>) -> Self {
        Self {
            arena: Default::default(),
            head: Default::default(),
            context: Default::default(),
            gas_inspector,
        }
    }

    /// Enters a new execution context.
    pub fn enter(&mut self, depth: usize, address: Address, kind: CallKind) {
        self.context = address;
        self.head = self.arena.push_node(DebugNode { depth, address, kind, ..Default::default() });
    }

    /// Exits the current execution context, replacing it with the previous one.
    pub fn exit(&mut self) {
        if let Some(parent_id) = self.arena.arena[self.head].parent {
            let DebugNode { depth, address, kind, .. } = self.arena.arena[parent_id];
            self.context = address;
            self.head =
                self.arena.push_node(DebugNode { depth, address, kind, ..Default::default() });
        }
    }
}

impl<DB> Inspector<DB> for Debugger
where
    DB: DatabaseExt,
{
    fn step(
        &mut self,
        interpreter: &mut Interpreter,
        data: &mut EVMData<'_, DB>,
        _is_static: bool,
    ) -> Return {
        let pc = interpreter.program_counter();
        let op = interpreter.contract.bytecode.bytecode()[pc];

        // Get opcode information
        let opcode_infos = spec_opcode_gas(data.env.cfg.spec_id);
        let opcode_info = &opcode_infos[op as usize];

        // Extract the push bytes
        let push_size = if opcode_info.is_push() { (op - opcode::PUSH1 + 1) as usize } else { 0 };
        let push_bytes = match push_size {
            0 => None,
            n => {
                let start = pc + 1;
                let end = start + n;
                Some(interpreter.contract.bytecode.bytecode()[start..end].to_vec())
            }
        };

        let total_gas_used = gas_used(
            data.env.cfg.spec_id,
            interpreter.gas.limit().saturating_sub(self.gas_inspector.borrow().gas_remaining()),
            interpreter.gas.refunded() as u64,
        );

        self.arena.arena[self.head].steps.push(DebugStep {
            pc,
            stack: interpreter.stack().data().clone(),
            memory: interpreter.memory.clone(),
            instruction: Instruction::OpCode(op),
            push_bytes,
            total_gas_used,
        });

        Return::Continue
    }

    fn call(
        &mut self,
        data: &mut EVMData<'_, DB>,
        call: &mut CallInputs,
        _: bool,
    ) -> (Return, Gas, Bytes) {
        self.enter(
            data.journaled_state.depth() as usize,
            call.context.code_address,
            call.context.scheme.into(),
        );
        if call.contract == CHEATCODE_ADDRESS {
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
        if let Err(err) = data.journaled_state.load_account(call.caller, data.db) {
            return (Return::Revert, None, Gas::new(call.gas_limit), err.encode_string())
        }

        let nonce = data.journaled_state.account(call.caller).info.nonce;
        self.enter(
            data.journaled_state.depth() as usize,
            get_create_address(call, nonce),
            CallKind::Create,
        );

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
