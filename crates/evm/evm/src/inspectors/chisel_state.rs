use alloy_primitives::U256;
use foundry_evm_core::evm::FoundryEvmContext;
use revm::{
    interpreter::{interpreter_types::Jumps, InstructionResult, Interpreter},
    Database, Inspector,
};

/// An inspector for Chisel
#[derive(Clone, Debug, Default)]
pub struct ChiselState {
    /// The PC of the final instruction
    pub final_pc: usize,
    /// The final state of the REPL contract call
    pub state: Option<(Vec<U256>, Vec<u8>, InstructionResult)>,
}

impl ChiselState {
    /// Create a new Chisel state inspector.
    #[inline]
    pub fn new(final_pc: usize) -> Self {
        Self { final_pc, state: None }
    }
}

impl<DB: Database> Inspector<DB> for ChiselState {
    #[cold]
    fn step_end(&mut self, interp: &mut Interpreter, _context: &mut FoundryEvmContext<DB>) {
        // If we are at the final pc of the REPL contract execution, set the state.
        // Subtraction can't overflow because `pc` is always at least 1 in `step_end`.
        if self.final_pc == interp.bytecode.pc() - 1 {
            self.state = Some((
                interp.stack.data().clone(),
                interp.memory,
                interp.control.instruction_result,
            ))
        }
    }
}
