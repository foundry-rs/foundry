use super::{Cheatcodes, LogCollector, Tracer};
use bytes::Bytes;
use ethers::types::Address;
use revm::{db::Database, CallInputs, CreateInputs, EVMData, Gas, Inspector, Interpreter, Return};

/// Helper macro to call the same method on multiple inspectors without resorting to dynamic
/// dispatch
macro_rules! call_inspectors {
    ($id:ident, [ $($inspector:expr),+ ], $call:block) => {
        $({
            if let Some($id) = $inspector {
                $call;
            }
        })+
    }
}

/// An inspector that calls multiple inspectors in sequence.
///
/// The order in which inspectors are called match the order in which they were added to the stack.
///
/// If a call to an inspector returns a value other than [Return::Continue] (or equivalent) the
/// remaining inspectors are not called.
#[derive(Default)]
pub struct InspectorStack {
    pub tracer: Option<Tracer>,
    pub logs: Option<LogCollector>,
    pub cheatcodes: Option<Cheatcodes>,
}

impl InspectorStack {
    pub fn new() -> Self {
        Default::default()
    }
}

impl<DB> Inspector<DB> for InspectorStack
where
    DB: Database,
{
    fn initialize_interp(
        &mut self,
        interpreter: &mut Interpreter,
        data: &mut EVMData<'_, DB>,
        is_static: bool,
    ) -> Return {
        call_inspectors!(inspector, [&mut self.tracer, &mut self.logs, &mut self.cheatcodes], {
            let status = inspector.initialize_interp(interpreter, data, is_static);

            // Allow inspectors to exit early
            if status != Return::Continue {
                return status
            }
        });

        Return::Continue
    }

    fn step(
        &mut self,
        interpreter: &mut Interpreter,
        data: &mut EVMData<'_, DB>,
        is_static: bool,
    ) -> Return {
        call_inspectors!(inspector, [&mut self.tracer, &mut self.logs, &mut self.cheatcodes], {
            let status = inspector.step(interpreter, data, is_static);

            // Allow inspectors to exit early
            if status != Return::Continue {
                return status
            }
        });

        Return::Continue
    }

    fn step_end(
        &mut self,
        interpreter: &mut Interpreter,
        data: &mut EVMData<'_, DB>,
        is_static: bool,
        status: Return,
    ) -> Return {
        call_inspectors!(inspector, [&mut self.tracer, &mut self.logs, &mut self.cheatcodes], {
            let status = inspector.step_end(interpreter, data, is_static, status);

            // Allow inspectors to exit early
            if status != Return::Continue {
                return status
            }
        });

        Return::Continue
    }

    fn call(
        &mut self,
        data: &mut EVMData<'_, DB>,
        call: &CallInputs,
        is_static: bool,
    ) -> (Return, Gas, Bytes) {
        call_inspectors!(inspector, [&mut self.tracer, &mut self.logs, &mut self.cheatcodes], {
            let (status, gas, retdata) = inspector.call(data, call, is_static);

            // Allow inspectors to exit early
            if status != Return::Continue {
                return (status, gas, retdata)
            }
        });

        (Return::Continue, Gas::new(call.gas_limit), Bytes::new())
    }

    fn call_end(
        &mut self,
        data: &mut EVMData<'_, DB>,
        call: &CallInputs,
        remaining_gas: Gas,
        status: Return,
        retdata: Bytes,
        is_static: bool,
    ) -> (Return, Gas, Bytes) {
        call_inspectors!(inspector, [&mut self.tracer, &mut self.logs, &mut self.cheatcodes], {
            let (new_status, new_gas, new_retdata) =
                inspector.call_end(data, call, remaining_gas, status, retdata.clone(), is_static);

            // If the inspector returns a different status we assume it wants to tell us something
            if new_status != status {
                return (new_status, new_gas, new_retdata)
            }
        });

        (status, remaining_gas, retdata)
    }

    fn create(
        &mut self,
        data: &mut EVMData<'_, DB>,
        call: &CreateInputs,
    ) -> (Return, Option<Address>, Gas, Bytes) {
        call_inspectors!(inspector, [&mut self.tracer, &mut self.logs, &mut self.cheatcodes], {
            let (status, addr, gas, retdata) = inspector.create(data, call);

            // Allow inspectors to exit early
            if status != Return::Continue {
                return (status, addr, gas, retdata)
            }
        });

        (Return::Continue, None, Gas::new(call.gas_limit), Bytes::new())
    }

    fn create_end(
        &mut self,
        data: &mut EVMData<'_, DB>,
        call: &CreateInputs,
        status: Return,
        address: Option<Address>,
        remaining_gas: Gas,
        retdata: Bytes,
    ) -> (Return, Option<Address>, Gas, Bytes) {
        call_inspectors!(inspector, [&mut self.tracer, &mut self.logs, &mut self.cheatcodes], {
            let (new_status, new_address, new_gas, new_retdata) =
                inspector.create_end(data, call, status, address, remaining_gas, retdata.clone());

            if new_status != status {
                return (new_status, new_address, new_gas, new_retdata)
            }
        });

        (status, address, remaining_gas, retdata)
    }

    fn selfdestruct(&mut self) {
        call_inspectors!(inspector, [&mut self.tracer, &mut self.logs, &mut self.cheatcodes], {
            Inspector::<DB>::selfdestruct(inspector);
        });
    }
}
