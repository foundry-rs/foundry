use crate::{invariant::RandomCallGenerator, strategies::EvmFuzzState};
use revm::{
    Inspector,
    context::{ContextTr, Transaction},
    inspector::JournalExt,
    interpreter::{CallInput, CallInputs, CallOutcome, CallScheme, Interpreter},
};

/// An inspector that can fuzz and collect data for that effect.
#[derive(Clone, Debug)]
pub struct Fuzzer {
    /// If set, it collects `stack` and `memory` values for fuzzing purposes.
    pub collect: bool,
    /// Given a strategy, it generates a random call.
    pub call_generator: Option<RandomCallGenerator>,
    /// If `collect` is set, we store the collected values in this fuzz dictionary.
    pub fuzz_state: EvmFuzzState,
}

impl<CTX> Inspector<CTX> for Fuzzer
where
    CTX: ContextTr<Journal: JournalExt>,
{
    #[inline]
    fn step(&mut self, interp: &mut Interpreter, _context: &mut CTX) {
        // We only collect `stack` and `memory` data before and after calls.
        if self.collect {
            self.collect_data(interp);
        }
    }

    fn call(&mut self, ecx: &mut CTX, inputs: &mut CallInputs) -> Option<CallOutcome> {
        // We don't want to override the very first call made to the test contract.
        if self.call_generator.is_some() && ecx.tx().caller() != inputs.caller {
            self.override_call(inputs);
        }

        // We only collect `stack` and `memory` data before and after calls.
        // this will be turned off on the next `step`
        self.collect = true;

        None
    }

    fn call_end(&mut self, _context: &mut CTX, _inputs: &CallInputs, _outcome: &mut CallOutcome) {
        if let Some(ref mut call_generator) = self.call_generator {
            call_generator.used = false;
        }

        // We only collect `stack` and `memory` data before and after calls.
        // this will be turned off on the next `step`
        self.collect = true;
    }
}

impl Fuzzer {
    /// Collects `stack` and `memory` values into the fuzz dictionary.
    #[cold]
    fn collect_data(&mut self, interpreter: &Interpreter) {
        self.fuzz_state.collect_values(interpreter.stack.data().iter().copied().map(Into::into));

        // TODO: disabled for now since it's flooding the dictionary
        // for index in 0..interpreter.shared_memory.len() / 32 {
        //     let mut slot = [0u8; 32];
        //     slot.clone_from_slice(interpreter.shared_memory.get_slice(index * 32, 32));

        //     state.insert(slot);
        // }

        self.collect = false;
    }

    /// Overrides an external call and tries to call any method of msg.sender.
    fn override_call(&mut self, call: &mut CallInputs) {
        if let Some(ref mut call_generator) = self.call_generator {
            // We only override external calls which are not coming from the test contract.
            if call.caller != call_generator.test_address
                && call.scheme == CallScheme::Call
                && !call_generator.used
            {
                // There's only a 30% chance that an override happens.
                if let Some(tx) = call_generator.next(call.caller, call.target_address) {
                    call.input = CallInput::Bytes(tx.call_details.calldata.0.into());
                    call.caller = tx.sender;
                    call.target_address = tx.call_details.target;

                    // TODO: in what scenarios can the following be problematic
                    call.bytecode_address = tx.call_details.target;
                    call_generator.used = true;
                }
            }
        }
    }
}
