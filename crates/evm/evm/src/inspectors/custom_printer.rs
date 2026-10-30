//! Custom print inspector, it has step level information of execution.
//! It is a great tool if some debugging is needed.

use alloy_primitives::{Address, U256};
use evm2::{
    EvmTypesHost, Inspector,
    interpreter::{Interpreter, Message, MessageResult, opcode::OpCode},
};
use foundry_common::sh_println;

/// Custom print [Inspector], it has step level information of execution.
///
/// It is a great tool if some debugging is needed.
#[derive(Clone, Debug, Default)]
pub struct CustomPrintTracer;

impl<T: EvmTypesHost> Inspector<T> for CustomPrintTracer {
    fn step(&mut self, interp: &mut Interpreter<'_, '_, T>) {
        let opcode = interp.opcode();
        let name = OpCode::new_or_unknown(opcode).as_str();
        let gas = interp.gas();
        let gas_remaining = gas.remaining();
        let refunded = gas.refunded();

        let _ = sh_println!(
            "depth:{}, PC:{}, gas:{:#x}({}), OPCODE: {:?}({:?})  refund:{:#x}({}) Stack:{:?}, Data size:{}",
            interp.message().depth,
            interp.pc(),
            gas_remaining,
            gas_remaining,
            name,
            opcode,
            refunded,
            refunded,
            interp.stack().as_slice(),
            interp.memory().len(),
        );
    }

    fn call(
        &mut self,
        _interp: &mut Interpreter<'_, '_, T>,
        message: &mut Message<T>,
    ) -> Option<MessageResult<T>> {
        let _ = sh_println!(
            "SM Address: {:?}, caller:{:?},target:{:?} is_static:{:?}, transfer:{:?}, input_size:{:?}",
            message.code_address,
            message.caller,
            message.destination,
            message.is_static(),
            message.value,
            message.input.len(),
        );
        None
    }

    fn create(
        &mut self,
        _interp: &mut Interpreter<'_, '_, T>,
        message: &mut Message<T>,
    ) -> Option<MessageResult<T>> {
        let _ = sh_println!(
            "CREATE CALL: caller:{:?}, scheme:{:?}, value:{:?}, init_code:{:?}, gas:{:?}",
            message.caller,
            message.kind,
            message.value,
            message.input,
            message.gas_limit
        );
        None
    }

    fn selfdestruct(
        &mut self,
        contract: &Address,
        target: &Address,
        value: &U256,
        _host: &mut T::Host<'_>,
    ) {
        let _ = sh_println!(
            "SELFDESTRUCT: contract: {:?}, refund target: {:?}, value {:?}",
            contract,
            target,
            value
        );
    }
}
