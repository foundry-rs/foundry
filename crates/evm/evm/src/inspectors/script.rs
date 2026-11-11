use alloy_primitives::Address;
use evm2::{
    EvmTypesHost, Inspector,
    interpreter::{Interpreter, op},
};

/// Revert reason for `address(this)` in a script contract.
const ADDRESS_IN_SCRIPT: &[u8] = b"Usage of `address(this)` detected in script contract. Script contracts are ephemeral and their addresses should not be relied upon.";

/// An inspector that enforces certain rules during script execution.
///
/// Currently, it reverts if the `ADDRESS` opcode is used within the script's main contract.
#[derive(Clone, Debug, Default)]
pub struct ScriptExecutionInspector {
    /// The address of the script contract being executed.
    pub script_address: Address,
}

impl<T: EvmTypesHost> Inspector<T> for ScriptExecutionInspector {
    fn step(&mut self, interp: &mut Interpreter<'_, '_, T>) {
        // Check if both target and bytecode address are the same as script contract address
        // (allow calling external libraries when bytecode address is different).
        let message = interp.message();
        if interp.opcode() == op::ADDRESS
            && message.destination == self.script_address
            && message.code_address == self.script_address
        {
            foundry_cheatcodes::revert_frame(interp, ADDRESS_IN_SCRIPT);
        }
    }
}
