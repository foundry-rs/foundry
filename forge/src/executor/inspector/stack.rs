use bytes::Bytes;
use ethers::types::Address;
use revm::{db::Database, CallInputs, CreateInputs, EVMData, Gas, Inspector, Interpreter, Return};
use std::any::Any;

/// A wrapper trait for [Inspector]s that allows for downcasting to a concrete type.
pub trait DowncastableInspector<DB>: Inspector<DB> + Any
where
    DB: Database,
{
    fn as_any(&self) -> &dyn Any;
}

impl<DB, T> DowncastableInspector<DB> for T
where
    DB: Database,
    T: Inspector<DB> + Any,
{
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// An inspector that calls multiple inspectors in sequence.
///
/// The order in which inspectors are called match the order in which they were added to the stack.
///
/// If a call to an inspector returns a value other than [Return::Continue] (or equivalent) the
/// remaining inspectors are not called.
pub struct InspectorStack<DB> {
    // We use a Vec because ordering matters
    inspectors: Vec<Box<dyn DowncastableInspector<DB>>>,
}

impl<DB> InspectorStack<DB>
where
    DB: Database,
{
    pub fn new() -> Self {
        Default::default()
    }

    /// Add a new inspector to the stack.
    pub fn insert<T: DowncastableInspector<DB> + 'static>(&mut self, inspector: T) {
        self.inspectors.push(Box::new(inspector));
    }

    /// Get a reference to an inspector.
    pub fn get<T: 'static>(&self) -> Option<&T> {
        self.inspectors.iter().find_map(|inspector| inspector.as_ref().as_any().downcast_ref())
    }
}

impl<DB> Default for InspectorStack<DB>
where
    DB: Database,
{
    fn default() -> Self {
        Self { inspectors: Vec::new() }
    }
}

impl<DB> Inspector<DB> for InspectorStack<DB>
where
    DB: Database,
{
    fn initialize_interp(
        &mut self,
        interpreter: &mut Interpreter,
        data: &mut EVMData<'_, DB>,
        is_static: bool,
    ) -> Return {
        for inspector in &mut self.inspectors {
            let status = inspector.initialize_interp(interpreter, data, is_static);

            // Allow inspectors to exit early
            if status != Return::Continue {
                return status
            }
        }

        Return::Continue
    }

    fn step(
        &mut self,
        interpreter: &mut Interpreter,
        data: &mut EVMData<'_, DB>,
        is_static: bool,
    ) -> Return {
        for inspector in &mut self.inspectors {
            let status = inspector.step(interpreter, data, is_static);

            // Allow inspectors to exit early
            if status != Return::Continue {
                return status
            }
        }

        Return::Continue
    }

    fn step_end(
        &mut self,
        interpreter: &mut Interpreter,
        data: &mut EVMData<'_, DB>,
        is_static: bool,
        status: Return,
    ) -> Return {
        for inspector in &mut self.inspectors {
            let status = inspector.step_end(interpreter, data, is_static, status);

            // Allow inspectors to exit early
            if status != Return::Continue {
                return status
            }
        }

        Return::Continue
    }

    fn call(
        &mut self,
        data: &mut EVMData<'_, DB>,
        call: &CallInputs,
        is_static: bool,
    ) -> (Return, Gas, Bytes) {
        for inspector in &mut self.inspectors {
            let (status, gas, retdata) = inspector.call(data, call, is_static);

            // Allow inspectors to exit early
            if status != Return::Continue {
                return (status, gas, retdata)
            }
        }

        (Return::Continue, Gas::new(0), Bytes::new())
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
        for inspector in &mut self.inspectors {
            let (new_status, new_gas, new_retdata) =
                inspector.call_end(data, call, remaining_gas, status, retdata.clone(), is_static);

            // If the inspector returns a different status we assume it wants to tell us something
            if new_status != status {
                return (new_status, new_gas, new_retdata)
            }
        }

        (status, remaining_gas, retdata)
    }

    fn create(
        &mut self,
        data: &mut EVMData<'_, DB>,
        call: &CreateInputs,
    ) -> (Return, Option<Address>, Gas, Bytes) {
        for inspector in &mut self.inspectors {
            let (status, addr, gas, retdata) = inspector.create(data, call);

            // Allow inspectors to exit early
            if status != Return::Continue {
                return (status, addr, gas, retdata)
            }
        }

        (Return::Continue, None, Gas::new(0), Bytes::new())
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
        for inspector in &mut self.inspectors {
            let (new_status, new_address, new_gas, new_retdata) =
                inspector.create_end(data, call, status, address, remaining_gas, retdata.clone());

            if new_status != status {
                return (new_status, new_address, new_gas, new_retdata)
            }
        }

        (status, address, remaining_gas, retdata)
    }

    fn selfdestruct(&mut self) {
        for inspector in &mut self.inspectors {
            inspector.selfdestruct();
        }
    }
}
