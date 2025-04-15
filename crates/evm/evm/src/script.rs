use alloy_primitives::Address;
use revm::{
    interpreter::{Interpreter, InstructionResult},
    primitives::opcode,
    EvmContext, Inspector,
};

use crate::InspectorExt;

/// An inspector that enforces certain rules during script execution.
///
/// Currently, it only warns if the `ADDRESS` opcode is used within the script's main contract.
#[derive(Clone, Debug, Default)]
pub struct ScriptExecutionInspector {
    /// The address of the script contract being executed.
    script_address: Option<Address>,
}

impl ScriptExecutionInspector {
    /// Creates a new script execution inspector.
    pub fn new(script_address: Option<Address>) -> Self {
        Self { script_address }
    }
}

impl<DB: InspectorExt> Inspector<DB> for ScriptExecutionInspector {
    fn step(&mut self, interpreter: &mut Interpreter, _ecx: &mut EvmContext<DB>) {
        // Check for address(this) usage in the main script contract
        if let Some(script_addr) = self.script_address {
            if interpreter.current_opcode() == opcode::ADDRESS && interpreter.contract.address == script_addr {
                // Log the reason for revert
                tracing::error!(
                    target: "forge::script",
                    script_address=%script_addr,
                    "REVERT: Usage of `address(this)` detected in script contract. Script contracts are ephemeral and their addresses should not be relied upon."
                );
                // Set the instruction result to Revert to stop execution
                interpreter.instruction_result = InstructionResult::Revert;
            }
        }
        // Note: We don't return anything here as step returns void.
        // The original check returned InstructionResult::Continue, but that's the default behavior.
    }
} 
