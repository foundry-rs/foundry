use revm::{Database, Inspector};
use revm::interpreter::{InstructionResult, Interpreter, Memory, Stack};

/// An inspector for Chisel
#[derive(Default)]
pub struct ChiselState {
    /// The PC of the final instruction
    pub final_pc: usize,
    /// The final state of the REPL contract call
    pub state: Option<(Stack, Memory, InstructionResult)>,
}

impl ChiselState {
    pub fn new(final_pc: usize) -> Self {
        Self { final_pc, state: None }
    }
}

impl<DB> Inspector<DB> for ChiselState
where
    DB: Database,
{
    fn step_end(
        &mut self,
        interp: &mut Interpreter,
        _: &mut revm::EVMData<'_, DB>,
        _: bool,
        eval: InstructionResult,
    ) -> InstructionResult {
        // If we are at the final pc of the REPL contract execution, set the state.
        if self.final_pc == interp.program_counter() - 1 {
            self.state = Some((interp.stack().clone(), interp.memory.clone(), eval))
        }
        // Pass on [InstructionResult] from arguments
        eval
    }
}
