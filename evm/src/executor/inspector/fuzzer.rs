use crate::{
    fuzz::{invariant::RandomCallGenerator, strategies::EvmFuzzState},
    utils,
};
use bytes::Bytes;
use ethers::types::H160;
use revm::{Database, EVMData, Inspector};
use revm::interpreter::{CallInputs, CallScheme, Gas, InstructionResult, Interpreter};
use revm::primitives::{B160};

/// An inspector that can fuzz and collect data for that effect.
#[derive(Clone, Debug)]
pub struct Fuzzer {
    /// Given a strategy, it generates a random call.
    pub call_generator: Option<RandomCallGenerator>,
    /// If set, it collects `stack` and `memory` values for fuzzing purposes.
    pub collect: bool,
    /// If `collect` is set, we store the collected values in this fuzz dictionary.
    pub fuzz_state: EvmFuzzState,
}

impl<DB> Inspector<DB> for Fuzzer
where
    DB: Database,
{
    fn step(
        &mut self,
        interpreter: &mut Interpreter,
        _: &mut EVMData<'_, DB>,
        _is_static: bool,
    ) -> Return {
        // We only collect `stack` and `memory` data before and after calls.
        if self.collect {
            self.collect_data(interpreter);
            self.collect = false;
        }
        Return::Continue
    }

    fn call(
        &mut self,
        data: &mut EVMData<'_, DB>,
        call: &mut CallInputs,
        _: bool,
    ) -> (Return, Gas, Bytes) {
        // We don't want to override the very first call made to the test contract.
        if self.call_generator.is_some() && data.env.tx.caller != call.context.caller {
            self.override_call(call);
        }

        // We only collect `stack` and `memory` data before and after calls.
        // this will be turned off on the next `step`
        self.collect = true;

        (Return::Continue, Gas::new(call.gas_limit), Bytes::new())
    }

    fn call_end(
        &mut self,
        _: &mut EVMData<'_, DB>,
        _: &CallInputs,
        remaining_gas: Gas,
        status: Return,
        retdata: Bytes,
        _: bool,
    ) -> (Return, Gas, Bytes) {
        if let Some(ref mut call_generator) = self.call_generator {
            call_generator.used = false;
        }

        // We only collect `stack` and `memory` data before and after calls.
        // this will be turned off on the next `step`
        self.collect = true;

        (status, remaining_gas, retdata)
    }
}

impl Fuzzer {
    /// Collects `stack` and `memory` values into the fuzz dictionary.
    fn collect_data(&mut self, interpreter: &mut Interpreter) {
        let mut state = self.fuzz_state.write();

        for slot in interpreter.stack().data() {
            state.values_mut().insert(utils::u256_to_h256_be(*slot).into());
        }

        // TODO: disabled for now since it's flooding the dictionary
        // for index in 0..interpreter.memory.len() / 32 {
        //     let mut slot = [0u8; 32];
        //     slot.clone_from_slice(interpreter.memory.get_slice(index * 32, 32));

        //     state.insert(slot);
        // }
    }

    /// Overrides an external call and tries to call any method of msg.sender.
    fn override_call(&mut self, call: &mut CallInputs) {
        if let Some(ref mut call_generator) = self.call_generator {
            // We only override external calls which are not coming from the test contract.
            if call.context.caller != B160::from_slice(call_generator.test_address.as_bytes()) &&
                call.context.scheme == CallScheme::Call &&
                !call_generator.used
            {
                // There's only a 30% chance that an override happens.
                if let Some((sender, (contract, input))) =
                    call_generator.next(H160::from_slice(call.context.caller.as_bytes()), H160::from_slice(call.contract.as_bytes()))
                {
                    call.input = input.0;
                    call.context.caller = B160::from_slice(sender.as_bytes());
                    call.contract = B160::from_slice(contract.as_bytes());

                    // TODO: in what scenarios can the following be problematic
                    call.context.code_address = B160::from_slice(contract.as_bytes());
                    call.context.address = B160::from_slice(contract.as_bytes());

                    call_generator.used = true;
                }
            }
        }
    }
}
