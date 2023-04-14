use bytes::Bytes;
use revm::{
    inspectors::GasInspector,
    interpreter::{opcode, CallInputs, CreateInputs, Gas, InstructionResult, Interpreter},
    primitives::B160,
    Database, EVMData, Inspector,
};

#[derive(Clone, Default)]
pub struct TracePrinter {
    gas_inspector: GasInspector,
}

impl<DB: Database> Inspector<DB> for TracePrinter {
    fn initialize_interp(
        &mut self,
        interp: &mut Interpreter,
        data: &mut EVMData<'_, DB>,
        is_static: bool,
    ) -> Return {
        self.gas_inspector.initialize_interp(interp, data, is_static);
        Return::Continue
    }

    // get opcode by calling `interp.contract.opcode(interp.program_counter())`.
    // all other information can be obtained from interp.
    fn step(
        &mut self,
        interp: &mut Interpreter,
        data: &mut EVMData<'_, DB>,
        is_static: bool,
    ) -> Return {
        let opcode = interp.current_opcode();
        let opcode_str = opcode::OPCODE_JUMPMAP[opcode as usize];

        let gas_remaining = self.gas_inspector.gas_remaining();

        println!(
            "depth:{}, PC:{}, gas:{:#x}({}), OPCODE: {:?}({:?})  refund:{:#x}({}) Stack:{:?}, Data size:{}, Data: 0x{}",
            data.journaled_state.depth(),
            interp.program_counter(),
            gas_remaining,
            gas_remaining,
            opcode_str.unwrap(),
            opcode,
            interp.gas.refunded(),
            interp.gas.refunded(),
            interp.stack.data(),
            interp.memory.data().len(),
            hex::encode(interp.memory.data()),
        );

        self.gas_inspector.step(interp, data, is_static);

        Return::Continue
    }

    fn step_end(
        &mut self,
        interp: &mut Interpreter,
        data: &mut EVMData<'_, DB>,
        is_static: bool,
        eval: revm::Return,
    ) -> Return {
        self.gas_inspector.step_end(interp, data, is_static, eval);
        Return::Continue
    }

    fn call(
        &mut self,
        _data: &mut EVMData<'_, DB>,
        inputs: &mut CallInputs,
        is_static: bool,
    ) -> (Return, Gas, Bytes) {
        println!(
            "SM CALL:   {:?},context:{:?}, is_static:{:?}, transfer:{:?}, input_size:{:?}",
            inputs.contract,
            inputs.context,
            is_static,
            inputs.transfer,
            inputs.input.len(),
        );
        (Return::Continue, Gas::new(0), Bytes::new())
    }

    fn call_end(
        &mut self,
        data: &mut EVMData<'_, DB>,
        inputs: &CallInputs,
        remaining_gas: Gas,
        ret: Return,
        out: Bytes,
        is_static: bool,
    ) -> (Return, Gas, Bytes) {
        self.gas_inspector.call_end(data, inputs, remaining_gas, ret, out.clone(), is_static);
        (ret, remaining_gas, out)
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
        (Return::Continue, None, Gas::new(0), Bytes::new())
    }

    fn create_end(
        &mut self,
        data: &mut EVMData<'_, DB>,
        inputs: &CreateInputs,
        ret: InstructionResult,
        address: Option<B160>,
        remaining_gas: Gas,
        out: Bytes,
    ) -> (InstructionResult, Option<B160>, Gas, Bytes) {
        self.gas_inspector.create_end(data, inputs, ret, address, remaining_gas, out.clone());
        (ret, address, remaining_gas, out)
    }
}
