use crate::{
    fuzz::{invariant::RandomCallGenerator, strategies::EvmFuzzState},
    utils::{self, b160_to_h160, h160_to_b160},
};
use revm::{
    primitives::Bytes,
    interpreter::{CallInputs, CallScheme, Gas, InstructionResult, Interpreter},
    Database, EVMData, Inspector,
};

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

impl<DB: Database> Inspector<DB> for Fuzzer {
    #[inline]
    fn step(
        &mut self,
        interpreter: &mut Interpreter,
        _: &mut EVMData<'_, DB>,
    ) -> InstructionResult {
        // We only collect `stack` and `memory` data before and after calls.
        if self.collect {
            self.collect_data(interpreter);
            self.collect = false;
        }
        InstructionResult::Continue
    }

    #[inline]
    fn call(
        &mut self,
        data: &mut EVMData<'_, DB>,
        call: &mut CallInputs,
    ) -> (InstructionResult, Gas, Bytes) {
        // We don't want to override the very first call made to the test contract.
        if self.call_generator.is_some() && data.env.tx.caller != call.context.caller {
            self.override_call(call);
        }

        // We only collect `stack` and `memory` data before and after calls.
        // this will be turned off on the next `step`
        self.collect = true;

        (InstructionResult::Continue, Gas::new(call.gas_limit), Bytes::new())
    }

    #[inline]
    fn call_end(
        &mut self,
        _: &mut EVMData<'_, DB>,
        _: &CallInputs,
        remaining_gas: Gas,
        status: InstructionResult,
        retdata: Bytes,
    ) -> (InstructionResult, Gas, Bytes) {
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
    fn collect_data(&mut self, interpreter: &Interpreter) {
        let mut state = self.fuzz_state.write();

        for slot in interpreter.stack().data() {
            state.values_mut().insert(utils::u256_to_h256_be(utils::ru256_to_u256(*slot)).into());
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
            if call.context.caller != h160_to_b160(call_generator.test_address) &&
                call.context.scheme == CallScheme::Call &&
                !call_generator.used
            {
                // There's only a 30% chance that an override happens.
                if let Some((sender, (contract, input))) = call_generator
                    .next(b160_to_h160(call.context.caller), b160_to_h160(call.contract))
                {
                    *call.input = input.0;
                    call.context.caller = h160_to_b160(sender);
                    call.contract = h160_to_b160(contract);

                    // TODO: in what scenarios can the following be problematic
                    call.context.code_address = h160_to_b160(contract);
                    call.context.address = h160_to_b160(contract);

                    call_generator.used = true;
                }
            }
        }
    }
}
