use bytes::Bytes;
use revm::{
    interpreter::{opcode, CallInputs, CreateInputs, Gas, InstructionResult, Interpreter},
    primitives::B160,
    Database, EVMData, Inspector,
};

#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct TracePrinter;

impl<DB: Database> Inspector<DB> for TracePrinter {
    // get opcode by calling `interp.contract.opcode(interp.program_counter())`.
    // all other information can be obtained from interp.
    fn step(&mut self, interp: &mut Interpreter, data: &mut EVMData<'_, DB>) -> InstructionResult {
        let opcode = interp.current_opcode();
        let opcode_str = opcode::OPCODE_JUMPMAP[opcode as usize];
        let gas_remaining = interp.gas.remaining();
        println!(
            "depth:{}, PC:{}, gas:{:#x}({}), OPCODE: {:?}({:?})  refund:{:#x}({}) Stack:{:?}, Data size:{}, Data: 0x{}",
            data.journaled_state.depth(),
            interp.program_counter(),
            gas_remaining,
            gas_remaining,
            opcode_str.unwrap_or("<unknown>"),
            opcode,
            interp.gas.refunded(),
            interp.gas.refunded(),
            interp.stack.data(),
            interp.memory.data().len(),
            hex::encode(interp.memory.data()),
        );

        InstructionResult::Continue
    }

    fn call(
        &mut self,
        _data: &mut EVMData<'_, DB>,
        inputs: &mut CallInputs,
    ) -> (InstructionResult, Gas, Bytes) {
        println!(
            "SM CALL:   {:?},context:{:?}, is_static:{:?}, transfer:{:?}, input_size:{:?}",
            inputs.contract,
            inputs.context,
            inputs.is_static,
            inputs.transfer,
            inputs.input.len(),
        );
        (InstructionResult::Continue, Gas::new(0), Bytes::new())
    }

    fn create(
        &mut self,
        _data: &mut EVMData<'_, DB>,
        inputs: &mut CreateInputs,
    ) -> (InstructionResult, Option<B160>, Gas, Bytes) {
        println!(
            "CREATE CALL: caller:{:?}, scheme:{:?}, value:{:?}, init_code:{:?}, gas:{:?}",
            inputs.caller,
            inputs.scheme,
            inputs.value,
            hex::encode(&inputs.init_code),
            inputs.gas_limit
        );
        (InstructionResult::Continue, None, Gas::new(0), Bytes::new())
    }
}
