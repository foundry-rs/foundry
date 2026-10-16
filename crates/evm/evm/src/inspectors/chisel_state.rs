use alloy_primitives::U256;
use evm2::{EvmTypesHost, Inspector, interpreter::Interpreter};

/// An inspector for Chisel
#[derive(Clone, Debug, Default)]
pub struct ChiselState {
    /// The PC of the final instruction
    pub final_pc: usize,
    /// The final state of the REPL contract call
    pub state: Option<(Vec<U256>, Vec<u8>)>,
}

impl ChiselState {
    /// Create a new Chisel state inspector.
    #[inline]
    pub const fn new(final_pc: usize) -> Self {
        Self { final_pc, state: None }
    }
}

impl<T: EvmTypesHost> Inspector<T> for ChiselState {
    #[cold]
    fn step_end(&mut self, interp: &mut Interpreter<'_, '_, T>) {
        // If we are at the final pc of the REPL contract execution, set the state.
        // Subtraction can't overflow because `pc` is always at least 1 in `step_end`.
        if self.final_pc == interp.pc() - 1 {
            self.state =
                Some((interp.stack().as_slice().to_vec(), interp.memory().as_slice().to_vec()))
        }
    }
}
