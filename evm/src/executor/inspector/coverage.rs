use crate::{coverage::HitMaps, executor::inspector::utils::get_create_address};
use bytes::Bytes;
use ethers::types::Address;
use revm::{CallInputs, CreateInputs, Database, EVMData, Gas, Inspector, Interpreter, Return};

#[derive(Default, Debug)]
pub struct CoverageCollector {
    /// Maps that track instruction hit data.
    pub maps: HitMaps,

    /// The execution addresses, with the topmost one being the current address.
    context: Vec<Address>,
}

impl CoverageCollector {
    pub fn enter(&mut self, address: Address) {
        self.context.push(address);
    }

    pub fn exit(&mut self) {
        self.context.pop();
    }
}

impl<DB> Inspector<DB> for CoverageCollector
where
    DB: Database,
{
    fn call(
        &mut self,
        _: &mut EVMData<'_, DB>,
        call: &mut CallInputs,
        _: bool,
    ) -> (Return, Gas, Bytes) {
        self.enter(call.context.code_address);

        (Return::Continue, Gas::new(call.gas_limit), Bytes::new())
    }

    fn step(
        &mut self,
        interpreter: &mut Interpreter,
        _: &mut EVMData<'_, DB>,
        _is_static: bool,
    ) -> Return {
        if let Some(context) = self.context.last() {
            self.maps.entry(*context).or_default().hit(interpreter.program_counter());
        }

        Return::Continue
    }

    fn call_end(
        &mut self,
        _: &mut EVMData<'_, DB>,
        _: &CallInputs,
        gas: Gas,
        status: Return,
        retdata: Bytes,
        _: bool,
    ) -> (Return, Gas, Bytes) {
        self.exit();

        (status, gas, retdata)
    }

    fn create(
        &mut self,
        data: &mut EVMData<'_, DB>,
        call: &mut CreateInputs,
    ) -> (Return, Option<Address>, Gas, Bytes) {
        data.journaled_state.load_account(call.caller, data.db);
        let nonce = data.journaled_state.account(call.caller).info.nonce;
        self.enter(get_create_address(call, nonce));

        (Return::Continue, None, Gas::new(call.gas_limit), Bytes::new())
    }

    fn create_end(
        &mut self,
        _: &mut EVMData<'_, DB>,
        _: &CreateInputs,
        status: Return,
        address: Option<Address>,
        gas: Gas,
        retdata: Bytes,
    ) -> (Return, Option<Address>, Gas, Bytes) {
        self.exit();

        (status, address, gas, retdata)
    }
}
