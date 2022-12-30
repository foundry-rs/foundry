use revm::{Database, Inspector};

/// An inspector for Chisel
#[derive(Default)]
pub struct ChiselState {
    /// The PC of the final instruction
    pub final_pc: usize,
    /// The final state of the REPL contract call
    pub state: Option<(revm::Stack, revm::Memory, revm::Return)>,
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
        interp: &mut revm::Interpreter,
        _: &mut revm::EVMData<'_, DB>,
        _: bool,
        eval: revm::Return,
    ) -> revm::Return {
        // If we are at the final pc of the REPL contract execution, set the state.
        if self.final_pc == interp.program_counter() - 1 {
            self.state = Some((interp.stack().clone(), interp.memory.clone(), eval))
        }
        // Pass on [revm::Return] from arguments
        eval
    }
}
