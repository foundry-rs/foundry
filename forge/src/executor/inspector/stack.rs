use bytes::Bytes;
use ethers::types::{Address, U256};
use revm::{
    db::Database, CallContext, CreateScheme, EVMData, Gas, Inspector, Machine, Return, Transfer,
};
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
    fn initialize(&mut self, data: &mut EVMData<'_, DB>) {
        for inspector in &mut self.inspectors {
            inspector.initialize(data)
        }
    }

    fn initialize_machine(
        &mut self,
        machine: &mut Machine,
        data: &mut EVMData<'_, DB>,
        is_static: bool,
    ) -> Return {
        for inspector in &mut self.inspectors {
            let status = inspector.initialize_machine(machine, data, is_static);

            // Allow inspectors to exit early
            if status != Return::Continue {
                return status
            }
        }

        Return::Continue
    }

    fn step(
        &mut self,
        machine: &mut Machine,
        data: &mut EVMData<'_, DB>,
        is_static: bool,
    ) -> Return {
        for inspector in &mut self.inspectors {
            let status = inspector.initialize_machine(machine, data, is_static);

            // Allow inspectors to exit early
            if status != Return::Continue {
                return status
            }
        }

        Return::Continue
    }

    fn step_end(&mut self, status: Return, machine: &mut Machine) -> Return {
        for inspector in &mut self.inspectors {
            let status = inspector.step_end(status, machine);

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
        to: Address,
        context: &CallContext,
        transfer: &Transfer,
        input: &Bytes,
        gas_limit: u64,
        is_static: bool,
    ) -> (Return, Gas, Bytes) {
        for inspector in &mut self.inspectors {
            let (status, gas, retdata) =
                inspector.call(data, to, context, transfer, input, gas_limit, is_static);

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
        to: Address,
        context: &CallContext,
        transfer: &Transfer,
        input: &Bytes,
        gas_limit: u64,
        remaining_gas: u64,
        status: Return,
        retdata: &Bytes,
        is_static: bool,
    ) {
        for inspector in &mut self.inspectors {
            inspector.call_end(
                data,
                to,
                context,
                transfer,
                input,
                gas_limit,
                remaining_gas,
                status,
                retdata,
                is_static,
            );
        }
    }

    fn create(
        &mut self,
        data: &mut EVMData<'_, DB>,
        to: Address,
        scheme: &CreateScheme,
        value: U256,
        init_code: &Bytes,
        gas_limit: u64,
    ) -> (Return, Option<Address>, Gas, Bytes) {
        for inspector in &mut self.inspectors {
            let (status, addr, gas, retdata) =
                inspector.create(data, to, scheme, value, init_code, gas_limit);

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
        to: Address,
        scheme: &CreateScheme,
        value: U256,
        init_code: &Bytes,
        status: Return,
        address: Option<Address>,
        gas_limit: u64,
        remaining_gas: u64,
        retdata: &Bytes,
    ) {
        for inspector in &mut self.inspectors {
            inspector.create_end(
                data,
                to,
                scheme,
                value,
                init_code,
                status,
                address,
                gas_limit,
                remaining_gas,
                retdata,
            );
        }
    }

    fn selfdestruct(&mut self) {
        for inspector in &mut self.inspectors {
            inspector.selfdestruct();
        }
    }
}
