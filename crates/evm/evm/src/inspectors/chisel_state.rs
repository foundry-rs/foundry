use alloy_primitives::U256;
use foundry_evm_core::backend::DatabaseError;
use revm::{
    Database, Inspector,
    context::ContextTr,
    inspector::JournalExt,
    interpreter::{
        InstructionResult, Interpreter, interpreter::EthInterpreter, interpreter_types::Jumps,
    },
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

impl<CTX, D> Inspector<CTX, EthInterpreter> for ChiselState
where
    D: Database<Error = DatabaseError>,
    CTX: ContextTr<Db = D>,
    CTX::Journal: JournalExt,
{
    #[cold]
    fn step_end(&mut self, interp: &mut Interpreter, _context: &mut CTX) {
        // If we are at the final pc of the REPL contract execution, set the state.
        // Subtraction can't overflow because `pc` is always at least 1 in `step_end`.
        if self.final_pc == interp.bytecode.pc() - 1 {
            self.state = Some((
                interp.stack.data().clone(),
                interp.memory.context_memory().to_vec(),
                interp.control.instruction_result,
            ))
        }
    }
}
