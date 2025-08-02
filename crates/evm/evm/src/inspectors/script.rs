use alloy_evm::Database;
use alloy_primitives::{Address, Bytes};
use foundry_evm_core::backend::DatabaseError;
use revm::{
    Inspector,
    bytecode::opcode::ADDRESS,
    context::ContextTr,
    inspector::JournalExt,
    interpreter::{
        InstructionResult, Interpreter, InterpreterAction,
        interpreter::EthInterpreter,
        interpreter_types::{Jumps, LoopControl},
    },
};

/// An inspector that enforces certain rules during script execution.
///
/// Currently, it only warns if the `ADDRESS` opcode is used within the script's main contract.
#[derive(Clone, Debug, Default)]
pub struct ScriptExecutionInspector {
    /// The address of the script contract being executed.
    pub script_address: Address,
}

impl<CTX, D> Inspector<CTX, EthInterpreter> for ScriptExecutionInspector
where
    D: Database<Error = DatabaseError>,
    CTX: ContextTr<Db = D>,
    CTX::Journal: JournalExt,
{
    fn step(&mut self, interpreter: &mut Interpreter, _ecx: &mut CTX) {
        // Check if both target and bytecode address are the same as script contract address
        // (allow calling external libraries when bytecode address is different).
        if interpreter.bytecode.opcode() == ADDRESS
            && interpreter.input.target_address == self.script_address
            && interpreter.input.bytecode_address == Some(self.script_address)
        {
            interpreter.bytecode.set_action(InterpreterAction::new_return(
                InstructionResult::Revert,
                Bytes::from("Usage of `address(this)` detected in script contract. Script contracts are ephemeral and their addresses should not be relied upon."),
                interpreter.gas,
            ));
        }
    }
}
