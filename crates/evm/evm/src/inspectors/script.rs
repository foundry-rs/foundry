use alloy_primitives::Address;
use revm::{
    interpreter::{opcode::ADDRESS, InstructionResult, Interpreter},
    Database, EvmContext, Inspector,
};

/// An inspector that enforces certain rules during script execution.
///
/// Currently, it only warns if the `ADDRESS` opcode is used within the script's main contract.
#[derive(Clone, Debug, Default)]
pub struct ScriptExecutionInspector {
    /// The address of the script contract being executed.
    pub script_address: Address,
}

impl<DB: Database> Inspector<DB> for ScriptExecutionInspector {
    #[cold]
    fn step(&mut self, interpreter: &mut Interpreter, _ecx: &mut EvmContext<DB>) {
        // Check for address(this) usage in the main script contract
        if interpreter.current_opcode() == ADDRESS &&
            interpreter.contract.target_address == self.script_address
        {
            // Log the reason for revert
            tracing::error!(
                target: "forge::script",
                "Usage of `address(this)` detected in script contract. Script contracts are ephemeral and their addresses should not be relied upon."
            );
            // Set the instruction result to Revert to stop execution
            interpreter.instruction_result = InstructionResult::Revert;
        }
        // Note: We don't return anything here as step returns void.
        // The original check returned InstructionResult::Continue, but that's the default
        // behavior.
    }
}
