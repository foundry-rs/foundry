use alloy_primitives::Address;
use arrayvec::ArrayVec;
use foundry_common::ErrorExt;
use foundry_evm_core::{
    backend::DatabaseExt,
    debug::{DebugArena, DebugNode, DebugStep},
    utils::gas_used,
};
use revm::{
    interpreter::{
        opcode, CallInputs, CallOutcome, CreateInputs, CreateOutcome, Gas, InstructionResult,
        Interpreter, InterpreterResult,
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
    fn step(&mut self, interp: &mut Interpreter, ecx: &mut EvmContext<DB>) {
        let pc = interp.program_counter();
        let op = interp.current_opcode();

        // Extract the push bytes
        let push_size = if (opcode::PUSH1..=opcode::PUSH32).contains(&op) {
            (op - opcode::PUSH0) as usize
        } else {
            0
        };
        let push_bytes = (push_size > 0).then(|| {
            let start = pc + 1;
            let end = start + push_size;
            let slice = &interp.contract.bytecode.bytecode()[start..end];
            assert!(slice.len() <= 32);
            let mut array = ArrayVec::new();
            array.try_extend_from_slice(slice).unwrap();
            array
        });

        let total_gas_used = gas_used(
            ecx.spec_id(),
            interp.gas.limit().saturating_sub(interp.gas.remaining()),
            interp.gas.refunded() as u64,
        );

        // Reuse the memory from the previous step if the previous opcode did not modify it.
        let memory = self.arena.arena[self.head]
            .steps
            .last()
            .filter(|step| !step.opcode_modifies_memory())
            .map(|step| step.memory.clone())
            .unwrap_or_else(|| interp.shared_memory.context_memory().to_vec().into());

        self.arena.arena[self.head].steps.push(DebugStep {
            pc,
            stack: interp.stack().data().clone(),
            memory,
            calldata: interp.contract().input.clone(),
            returndata: interp.return_data_buffer.clone(),
            instruction: op,
            push_bytes: push_bytes.unwrap_or_default(),
            total_gas_used,
        });
    }

    fn call(&mut self, ecx: &mut EvmContext<DB>, inputs: &mut CallInputs) -> Option<CallOutcome> {
        self.enter(
            ecx.journaled_state.depth() as usize,
            inputs.bytecode_address,
            inputs.scheme.into(),
        );

        None
    }

    fn call_end(
        &mut self,
        _context: &mut EvmContext<DB>,
        _inputs: &CallInputs,
        outcome: CallOutcome,
    ) -> CallOutcome {
        self.exit();

        outcome
    }

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
