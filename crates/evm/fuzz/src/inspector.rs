use crate::{invariant::RandomCallGenerator, strategies::EvmFuzzState};
use alloy_primitives::U256;
use revm::{
    interpreter::{CallInputs, CallOutcome, CallScheme, Interpreter},
    Database, EvmContext, Inspector,
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
    fn step(&mut self, interp: &mut Interpreter, _context: &mut EvmContext<DB>) {
        // We only collect `stack` and `memory` data before and after calls.
        if self.collect {
            self.collect_data(interp);
            self.collect = false;
        }
    }

    #[inline]
    fn call(&mut self, ecx: &mut EvmContext<DB>, inputs: &mut CallInputs) -> Option<CallOutcome> {
        // We don't want to override the very first call made to the test contract.
        if self.call_generator.is_some() && ecx.env.tx.caller != inputs.context.caller {
            self.override_call(inputs);
        }

        // We only collect `stack` and `memory` data before and after calls.
        // this will be turned off on the next `step`
        self.collect = true;

        None
    }

    #[inline]
    fn call_end(
        &mut self,
        _context: &mut EvmContext<DB>,
        _inputs: &CallInputs,
        outcome: CallOutcome,
    ) -> CallOutcome {
        if let Some(ref mut call_generator) = self.call_generator {
            call_generator.used = false;
        }

        // We only collect `stack` and `memory` data before and after calls.
        // this will be turned off on the next `step`
        self.collect = true;

        outcome
    }
}

impl Fuzzer {
    /// Collects `stack` and `memory` values into the fuzz dictionary.
    fn collect_data(&mut self, interpreter: &Interpreter) {
        self.fuzz_state.collect_values(interpreter.stack().data().iter().map(U256::to_be_bytes));

        // TODO: disabled for now since it's flooding the dictionary
        // for index in 0..interpreter.shared_memory.len() / 32 {
        //     let mut slot = [0u8; 32];
        //     slot.clone_from_slice(interpreter.shared_memory.get_slice(index * 32, 32));

        //     state.insert(slot);
        // }
    }

    /// Overrides an external call and tries to call any method of msg.sender.
    fn override_call(&mut self, call: &mut CallInputs) {
        if let Some(ref mut call_generator) = self.call_generator {
            // We only override external calls which are not coming from the test contract.
            if call.context.caller != call_generator.test_address &&
                call.context.scheme == CallScheme::Call &&
                !call_generator.used
            {
                // There's only a 30% chance that an override happens.
                if let Some((sender, (contract, input))) =
                    call_generator.next(call.context.caller, call.contract)
                {
                    *call.input = input.0;
                    call.context.caller = sender;
                    call.contract = contract;

                    // TODO: in what scenarios can the following be problematic
                    call.context.code_address = contract;
                    call.context.address = contract;

                    call_generator.used = true;
                }
            }
        }
    }
}
