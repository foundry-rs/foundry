use alloy_primitives::U256;
use evm2::{EvmTypesHost, Inspector, interpreter::Interpreter};

/// An inspector for Chisel
#[derive(Clone, Debug, Default)]
pub struct ChiselState {
    /// The PC of the final instruction
    pub final_pc: usize,
    /// The final state of the REPL contract call
    pub state: Option<(Vec<U256>, Vec<u8>)>,
    /// Whether the executing instruction is the final one.
    at_final_pc: bool,
}

impl ChiselState {
    /// Create a new Chisel state inspector.
    #[inline]
    pub const fn new(final_pc: usize) -> Self {
        Self { final_pc, state: None, at_final_pc: false }
    }
}

impl<T: EvmTypesHost> Inspector<T> for ChiselState {
    #[cold]
    fn step(&mut self, interp: &mut Interpreter<'_, '_, T>) {
        self.at_final_pc = interp.pc() == self.final_pc;
    }

    #[cold]
    fn step_end(&mut self, interp: &mut Interpreter<'_, '_, T>) {
        // If we are at the final pc of the REPL contract execution, set the state.
        // evm2 leaves `pc` on an instruction that halts, such as an assembly `return`.
        let pc = interp.pc();
        if pc.checked_sub(1) == Some(self.final_pc) || pc == self.final_pc && self.at_final_pc {
            self.state =
                Some((interp.stack().as_slice().to_vec(), interp.memory().as_slice().to_vec()))
        }
    }
}
