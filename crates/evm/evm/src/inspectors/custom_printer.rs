//! Custom print inspector, it has step level information of execution.
//! It is a great tool if some debugging is needed.

use foundry_common::sh_println;
use foundry_evm_core::backend::DatabaseError;
use revm::{
    Database, Inspector,
    bytecode::opcode::OpCode,
    context::{ContextTr, JournalTr},
    inspector::{JournalExt, inspectors::GasInspector},
    interpreter::{
        CallInputs, CallOutcome, CreateInputs, CreateOutcome, Interpreter,
        interpreter::EthInterpreter,
        interpreter_types::{Jumps, MemoryTr},
    },
    primitives::{Address, U256},
};

/// Custom print [Inspector], it has step level information of execution.
///
/// It is a great tool if some debugging is needed.
#[derive(Clone, Debug, Default)]
pub struct CustomPrintTracer {
    gas_inspector: GasInspector,
}

impl<CTX, D> Inspector<CTX, EthInterpreter> for CustomPrintTracer
where
    D: Database<Error = DatabaseError>,
    CTX: ContextTr<Db = D>,
    CTX::Journal: JournalExt,
{
    fn initialize_interp(&mut self, interp: &mut Interpreter, _context: &mut CTX) {
        self.gas_inspector.initialize_interp(&interp.gas);
    }

    // get opcode by calling `interp.contract.opcode(interp.program_counter())`.
    // all other information can be obtained from interp.
    fn step(&mut self, interp: &mut Interpreter, context: &mut CTX) {
        let opcode = interp.bytecode.opcode();
        let name = OpCode::name_by_op(opcode);

        let gas_remaining = self.gas_inspector.gas_remaining();

        let memory_size = interp.memory.size();

        let _ = sh_println!(
            "depth:{}, PC:{}, gas:{:#x}({}), OPCODE: {:?}({:?})  refund:{:#x}({}) Stack:{:?}, Data size:{}",
            context.journal().depth(),
            interp.bytecode.pc(),
            gas_remaining,
            gas_remaining,
            name,
            opcode,
            interp.gas.refunded(),
            interp.gas.refunded(),
            interp.stack.data(),
            memory_size,
        );

        self.gas_inspector.step(&interp.gas);
    }

    fn step_end(&mut self, interpreter: &mut Interpreter, _context: &mut CTX) {
        self.gas_inspector.step_end(&interpreter.gas);
    }

    fn call_end(&mut self, _context: &mut CTX, _inputs: &CallInputs, outcome: &mut CallOutcome) {
        self.gas_inspector.call_end(outcome)
    }

    fn create_end(
        &mut self,
        _context: &mut CTX,
        _inputs: &CreateInputs,
        outcome: &mut CreateOutcome,
    ) {
        self.gas_inspector.create_end(outcome)
    }

    fn call(&mut self, _context: &mut CTX, inputs: &mut CallInputs) -> Option<CallOutcome> {
        let _ = sh_println!(
            "SM Address: {:?}, caller:{:?},target:{:?} is_static:{:?}, transfer:{:?}, input_size:{:?}",
            inputs.bytecode_address,
            inputs.caller,
            inputs.target_address,
            inputs.is_static,
            inputs.value,
            inputs.input.len(),
        );
        None
    }

    fn create(&mut self, _context: &mut CTX, inputs: &mut CreateInputs) -> Option<CreateOutcome> {
        let _ = sh_println!(
            "CREATE CALL: caller:{:?}, scheme:{:?}, value:{:?}, init_code:{:?}, gas:{:?}",
            inputs.caller,
            inputs.scheme,
            inputs.value,
            inputs.init_code,
            inputs.gas_limit
        );
        None
    }

    fn selfdestruct(&mut self, contract: Address, target: Address, value: U256) {
        let _ = sh_println!(
            "SELFDESTRUCT: contract: {:?}, refund target: {:?}, value {:?}",
            contract,
            target,
            value
        );
    }
}
