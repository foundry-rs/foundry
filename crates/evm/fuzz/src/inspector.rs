use crate::{invariant::RandomCallGenerator, strategies::EvmFuzzState};
use alloy_primitives::Bytes;
use foundry_evm_core::utils;
use foundry_utils::types::ToEthers;
use revm::{
    interpreter::{CallInputs, CallScheme, Gas, InstructionResult, Interpreter, InterpreterResult},
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
    fn step(&mut self, interpreter: &mut Interpreter, _: &mut EvmContext<'_, DB>) {
        // We only collect `stack` and `memory` data before and after calls.
        if self.collect {
            self.collect_data(interpreter);
            self.collect = false;
        }
    }

    #[inline]
    fn call(
        &mut self,
        data: &mut EvmContext<'_, DB>,
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
        ctx: &mut EvmContext<'_, DB>,
        result: InterpreterResult,
    ) -> InterpreterResult {
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
            state.values_mut().insert(utils::u256_to_h256_be(slot.to_ethers()).into());
        }

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
