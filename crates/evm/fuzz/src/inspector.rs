use crate::{invariant::RandomCallGenerator, strategies::EvmFuzzState};
use foundry_common::mapping_slots::step as mapping_step;
use foundry_evm_core::constants::CHEATCODE_ADDRESS;
use revm::{
    Inspector,
    context::{ContextTr, JournalTr, Transaction},
    inspector::JournalExt,
    interpreter::{CallInput, CallInputs, CallOutcome, CallScheme, CallValue, Interpreter},
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
            if let Some(mapping_slots) = &mut self.fuzz_state.mapping_slots {
                mapping_step(mapping_slots, interp);
            }
        }
    }

    fn call(&mut self, ecx: &mut CTX, inputs: &mut CallInputs) -> Option<CallOutcome> {
        // We don't want to override the very first call made to the test contract.
        if self.call_generator.is_some() && ecx.tx().caller() != inputs.caller {
            self.override_call(ecx, inputs);
        }

        // We only collect `stack` and `memory` data before and after calls.
        // this will be turned off on the next `step`
        self.collect = true;

        None
    }

    fn call_end(&mut self, _context: &mut CTX, _inputs: &CallInputs, _outcome: &mut CallOutcome) {
        if let Some(ref mut call_generator) = self.call_generator {
            // Decrement depth when any call ends while inside an override
            if call_generator.override_depth > 0 {
                call_generator.override_depth -= 1;
            }
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

    /// Overrides an external call to simulate reentrancy attacks.
    ///
    /// This function detects reentrancy vulnerabilities by replacing external calls
    /// with callbacks that reenter the caller contract.
    ///
    /// For calls with value (ETH transfers):
    /// 1. Performs the ETH transfer via the journal first
    /// 2. Replaces the call with a reentrant callback (value = 0)
    ///
    /// For calls without value:
    /// - Replaces the call entirely with a reentrant callback
    ///
    /// This simulates malicious contracts that immediately reenter when called.
    fn override_call<CTX>(&mut self, ecx: &mut CTX, call: &mut CallInputs)
    where
        CTX: ContextTr<Journal: JournalExt>,
    {
        let Some(ref mut call_generator) = self.call_generator else {
            return;
        };

        // Skip if:
        // - Caller is test contract (don't override the initial calls from the test)
        // - Not a CALL scheme (only override CALLs, not STATICCALLs, DELEGATECALLs, etc.)
        // - Inside an override (prevent recursive overrides)
        // - Target is cheatcode address
        // - Neither caller nor target is a handler contract
        //
        // We override calls when either the caller OR target is a handler. This covers:
        // 1. EtherStore pattern: handler sends ETH out, attacker reenters handler
        // 2. Rari pattern: external protocol sends ETH to handler, handler reenters protocol
        let caller_is_handler = call_generator.is_handler(call.caller);
        let target_is_handler = call_generator.is_handler(call.target_address);
        if call.caller == call_generator.test_address
            || call.scheme != CallScheme::Call
            || call_generator.override_depth > 0
            || call.target_address == CHEATCODE_ADDRESS
            || (!caller_is_handler && !target_is_handler)
        {
            return;
        }

        // There's only a ~27% chance that an override happens (90% * 30% from strategy).
        let Some(tx) = call_generator.next(call.caller, call.target_address) else {
            return;
        };

        // For value transfers, perform the ETH transfer before injecting the callback.
        // This simulates a malicious receive() that gets the ETH and then reenters.
        let value = call.transfer_value().unwrap_or_default();
        let has_value = !value.is_zero() && call.gas_limit > 2300;
        if has_value && ecx.journal_mut().transfer(call.caller, call.target_address, value).is_err()
        {
            return;
        }

        // Replace the call with a reentrant callback
        call.input = CallInput::Bytes(tx.call_details.calldata.0.into());
        call.caller = tx.sender;
        call.target_address = tx.call_details.target;
        call.bytecode_address = tx.call_details.target;
        // Clear known_bytecode to force REVM to load bytecode from the new target.
        // Without this, REVM uses cached bytecode from the original target (e.g., empty
        // bytecode for EOA), causing the call to short-circuit before executing any code.
        call.known_bytecode = None;
        // Clear value since ETH was already transferred above
        call.value = CallValue::Transfer(alloy_primitives::U256::ZERO);

        // Track that we're inside an overridden call to avoid recursive overrides
        call_generator.override_depth = 1;
    }
}
