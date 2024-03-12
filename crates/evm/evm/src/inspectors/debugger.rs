use alloy_primitives::Address;
use foundry_common::{ErrorExt, SELECTOR_LEN};
use foundry_evm_core::{backend::DatabaseExt, constants::CHEATCODE_ADDRESS, debug::{DebugArena, DebugNode, DebugStep, Instruction}, opcodes, utils::gas_used};
use revm::{
    interpreter::{
        opcode::{self, spec_opcode_gas},
        CallInputs, CallOutcome, CreateInputs, CreateOutcome, Gas, InstructionResult, Interpreter,
        InterpreterResult,
    },
    EvmContext, Inspector,
};
use revm_inspectors::tracing::types::CallKind;

/// An inspector that collects debug nodes on every step of the interpreter.
#[derive(Clone, Debug, Default)]
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
    fn step(&mut self, interp: &mut Interpreter, ecx: &mut EvmContext<DB>) {
        let pc = interp.program_counter();
        let op = interp.current_opcode();

        // Get opcode information
        let opcode_infos = spec_opcode_gas(ecx.spec_id());
        let opcode_info = &opcode_infos[op as usize];

        // Extract the push bytes
        let push_size = if opcode_info.is_push() { (op - opcode::PUSH1 + 1) as usize } else { 0 };
        let push_bytes = match push_size {
            0 => None,
            n => {
                let start = pc + 1;
                let end = start + n;
                Some(interp.contract.bytecode.bytecode()[start..end].to_vec())
            }
        };

        let total_gas_used = gas_used(
            ecx.spec_id(),
            interp.gas.limit().saturating_sub(interp.gas.remaining()),
            interp.gas.refunded() as u64,
        );


        let head = self.arena.arena[self.head].steps.last().and_then(|previous| {
            previous.instruction.opcode().map_or(false, |opcode|  opcodes::modifies_memory(previous));

        })

        self.arena.arena[self.head].steps.push(DebugStep {
            pc,
            stack: interp.stack().data().clone(),
            memory: interp.shared_memory.context_memory().to_vec(),
            calldata: interp.contract().input.to_vec(),
            returndata: interp.return_data_buffer.to_vec(),
            instruction: Instruction::OpCode(op),
            push_bytes,
            total_gas_used,
        });
    }

    #[inline]
    fn call(&mut self, ecx: &mut EvmContext<DB>, inputs: &mut CallInputs) -> Option<CallOutcome> {
        self.enter(
            ecx.journaled_state.depth() as usize,
            inputs.context.code_address,
            inputs.context.scheme.into(),
        );

        if inputs.contract == CHEATCODE_ADDRESS {
            if let Some(selector) = inputs.input.get(..SELECTOR_LEN) {
                self.arena.arena[self.head].steps.push(DebugStep {
                    instruction: Instruction::Cheatcode(selector.try_into().unwrap()),
                    ..Default::default()
                });
            }
        }

        None
    }

    #[inline]
    fn call_end(
        &mut self,
        _context: &mut EvmContext<DB>,
        _inputs: &CallInputs,
        outcome: CallOutcome,
    ) -> CallOutcome {
        self.exit();

        outcome
    }

    #[inline]
    fn create(
        &mut self,
        ecx: &mut EvmContext<DB>,
        inputs: &mut CreateInputs,
    ) -> Option<CreateOutcome> {
        if let Err(err) = ecx.load_account(inputs.caller) {
            let gas = Gas::new(inputs.gas_limit);
            return Some(CreateOutcome::new(
                InterpreterResult {
                    result: InstructionResult::Revert,
                    output: err.abi_encode_revert(),
                    gas,
                },
                None,
            ));
        }

        let nonce = ecx.journaled_state.account(inputs.caller).info.nonce;
        self.enter(
            ecx.journaled_state.depth() as usize,
            inputs.created_address(nonce),
            CallKind::Create,
        );

        None
    }

    #[inline]
    fn create_end(
        &mut self,
        _context: &mut EvmContext<DB>,
        _inputs: &CreateInputs,
        outcome: CreateOutcome,
    ) -> CreateOutcome {
        self.exit();

        outcome
    }
}
