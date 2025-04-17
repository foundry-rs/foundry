//! Custom print inspector, it has step level information of execution.
//! It is a great tool if some debugging is needed.

use foundry_evm_core::evm::FoundryEvmContext;
use revm::{
    bytecode::opcode::OpCode,
    inspector::inspectors::GasInspector,
    interpreter::{
        interpreter::EthInterpreter,
        interpreter_types::{Jumps, LoopControl, MemoryTr},
        CallInputs, CallOutcome, CreateInputs, CreateOutcome, Interpreter,
    },
    primitives::{Address, U256},
    Inspector,
};

/// Custom print [Inspector], it has step level information of execution.
///
/// It is a great tool if some debugging is needed.
#[derive(Clone, Debug, Default)]
pub struct CustomPrintTracer {
    gas_inspector: GasInspector,
}

impl Inspector<FoundryEvmContext<'_>, EthInterpreter> for CustomPrintTracer {
    fn initialize_interp(
        &mut self,
        interp: &mut Interpreter,
        _context: &mut FoundryEvmContext<'_>,
    ) {
        self.gas_inspector.initialize_interp(&interp.control.gas);
    }

    // get opcode by calling `interp.contract.opcode(interp.program_counter())`.
    // all other information can be obtained from interp.
    fn step(&mut self, interp: &mut Interpreter, context: &mut FoundryEvmContext<'_>) {
        let opcode = interp.bytecode.opcode();
        let name = OpCode::name_by_op(opcode);

        let gas_remaining = self.gas_inspector.gas_remaining();

        let memory_size = interp.memory.size();

        println!(
            "depth:{}, PC:{}, gas:{:#x}({}), OPCODE: {:?}({:?})  refund:{:#x}({}) Stack:{:?}, Data size:{}",
            context.journaled_state.depth,
            interp.bytecode.pc(),
            gas_remaining,
            gas_remaining,
            name,
            opcode,
            interp.control.gas.refunded(),
            interp.control.gas.refunded(),
            interp.stack.data(),
            memory_size,
        );

        self.gas_inspector.step(&interp.control.gas);
    }

    fn step_end(&mut self, interp: &mut Interpreter, _context: &mut FoundryEvmContext<'_>) {
        self.gas_inspector.step_end(interp.control.gas_mut());
    }

    fn call_end(
        &mut self,
        _context: &mut FoundryEvmContext<'_>,
        _inputs: &CallInputs,
        outcome: &mut CallOutcome,
    ) {
        self.gas_inspector.call_end(outcome)
    }

    fn create_end(
        &mut self,
        _context: &mut FoundryEvmContext<'_>,
        _inputs: &CreateInputs,
        outcome: &mut CreateOutcome,
    ) {
        self.gas_inspector.create_end(outcome)
    }

    fn call(
        &mut self,
        _context: &mut FoundryEvmContext<'_>,
        inputs: &mut CallInputs,
    ) -> Option<CallOutcome> {
        println!(
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

    fn create(
        &mut self,
        _context: &mut FoundryEvmContext<'_>,
        inputs: &mut CreateInputs,
    ) -> Option<CreateOutcome> {
        println!(
            "CREATE CALL: caller:{:?}, scheme:{:?}, value:{:?}, init_code:{:?}, gas:{:?}",
            inputs.caller, inputs.scheme, inputs.value, inputs.init_code, inputs.gas_limit
        );
        None
    }

    fn selfdestruct(&mut self, contract: Address, target: Address, value: U256) {
        println!(
            "SELFDESTRUCT: contract: {:?}, refund target: {:?}, value {:?}",
            contract, target, value
        );
    }
}
