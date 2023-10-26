use alloy_primitives::{Address, Bytes};
use foundry_evm_core::{
    backend::DatabaseExt,
    constants::CHEATCODE_ADDRESS,
    debug::{DebugArena, DebugNode, DebugStep, Instruction},
    utils::{gas_used, get_create_address, CallKind},
};
use foundry_utils::error::SolError;
use revm::{
    interpreter::{
        opcode::{self, spec_opcode_gas},
        CallInputs, CreateInputs, Gas, InstructionResult, Interpreter, Memory,
    },
    EVMData, Inspector,
};

/// An inspector that collects debug nodes on every step of the interpreter.
#[derive(Clone, Default, Debug)]
pub struct Debugger {
    /// The arena of [DebugNode]s
    pub arena: DebugArena,
    /// The ID of the current [DebugNode].
    pub head: usize,
    /// The current execution address.
    pub context: Address,
}

impl Debugger {
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

impl<DB: DatabaseExt> Inspector<DB> for Debugger {
    #[inline]
    fn step(
        &mut self,
        interpreter: &mut Interpreter,
        data: &mut EVMData<'_, DB>,
    ) -> InstructionResult {
        let pc = interpreter.program_counter();
        let op = interpreter.current_opcode();

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
            interpreter.gas.limit().saturating_sub(interpreter.gas.remaining()),
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

        InstructionResult::Continue
    }

    #[inline]
    fn call(
        &mut self,
        data: &mut EVMData<'_, DB>,
        call: &mut CallInputs,
    ) -> (InstructionResult, Gas, Bytes) {
        self.enter(
            data.journaled_state.depth() as usize,
            call.context.code_address,
            call.context.scheme.into(),
        );
        if CHEATCODE_ADDRESS == call.contract {
            self.arena.arena[self.head].steps.push(DebugStep {
                memory: Memory::new(),
                instruction: Instruction::Cheatcode(
                    call.input[0..4].try_into().expect("malformed cheatcode call"),
                ),
                ..Default::default()
            });
        }

        (InstructionResult::Continue, Gas::new(call.gas_limit), Bytes::new())
    }

    #[inline]
    fn call_end(
        &mut self,
        _: &mut EVMData<'_, DB>,
        _: &CallInputs,
        gas: Gas,
        status: InstructionResult,
        retdata: Bytes,
    ) -> (InstructionResult, Gas, Bytes) {
        self.exit();

        (status, gas, retdata)
    }

    #[inline]
    fn create(
        &mut self,
        data: &mut EVMData<'_, DB>,
        call: &mut CreateInputs,
    ) -> (InstructionResult, Option<Address>, Gas, Bytes) {
        // TODO: Does this increase gas cost?
        if let Err(err) = data.journaled_state.load_account(call.caller, data.db) {
            let gas = Gas::new(call.gas_limit);
            return (
                InstructionResult::Revert,
                None,
                gas,
                alloy_primitives::Bytes(err.encode_string().0),
            )
        }

        let nonce = data.journaled_state.account(call.caller).info.nonce;
        self.enter(
            data.journaled_state.depth() as usize,
            get_create_address(call, nonce),
            CallKind::Create,
        );

        (InstructionResult::Continue, None, Gas::new(call.gas_limit), Bytes::new())
    }

    #[inline]
    fn create_end(
        &mut self,
        _: &mut EVMData<'_, DB>,
        _: &CreateInputs,
        status: InstructionResult,
        address: Option<Address>,
        gas: Gas,
        retdata: Bytes,
    ) -> (InstructionResult, Option<Address>, Gas, Bytes) {
        self.exit();

        (status, address, gas, retdata)
    }
}
