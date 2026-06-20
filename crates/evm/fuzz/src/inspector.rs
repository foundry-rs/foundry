use crate::invariant::RandomCallGenerator;
use alloy_primitives::{Address, B256, Bytes, U256, map::AddressMap};
use foundry_common::mapping_slots::{MappingSlots, step as mapping_step};
use foundry_evm_core::constants::CHEATCODE_ADDRESS;
use revm::{
    Inspector,
    context::{ContextTr, JournalTr, Transaction},
    interpreter::{CallInput, CallInputs, CallOutcome, CallScheme, CallValue, Interpreter},
};

/// A sub-call observed by the [`Fuzzer`] inspector.
///
/// `depth` is 1-indexed relative to the top-level call: depth 1 is a direct call
/// from the top-level callee, depth 2 is a sub-call of that call, and so on. The
/// top-level call itself is never recorded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservedCall {
    pub depth: u32,
    pub caller: Address,
    pub target: Address,
    pub calldata: Bytes,
    pub value: Option<U256>,
}

/// An inspector that can fuzz and collect data for that effect.
#[derive(Clone, Debug)]
pub struct Fuzzer {
    /// If set, it collects `stack` and `memory` values for fuzzing purposes.
    pub collect: bool,
    /// Given a strategy, it generates a random call.
    pub call_generator: Option<RandomCallGenerator>,
    /// If `collect` is set, we store collected values until the invariant worker drains them.
    pub collected_values: Vec<B256>,
    /// Maximum number of stack words staged before the invariant worker drains them.
    pub max_collected_values: usize,
    /// Mapping accesses observed during execution, used for storage slot sampling.
    pub mapping_slots: Option<AddressMap<MappingSlots>>,
    /// Whether sub-calls should be buffered for later corpus seeding.
    record_calls: bool,
    /// Sub-calls observed since the last drain.
    observed_calls: Vec<ObservedCall>,
    /// Current EVM call depth. 0 means no active call, 1 means top-level call.
    call_depth: u32,
}

impl<CTX: ContextTr> Inspector<CTX> for Fuzzer {
    #[inline]
    fn step(&mut self, interp: &mut Interpreter, _context: &mut CTX) {
        // We only collect `stack` and `memory` data before and after calls.
        if self.collect {
            self.collect_data(interp);
            if let Some(mapping_slots) = &mut self.mapping_slots {
                mapping_step(mapping_slots, interp);
            }
        }
    }

    fn call(&mut self, ecx: &mut CTX, inputs: &mut CallInputs) -> Option<CallOutcome> {
        // We don't want to override the very first call made to the test contract.
        if self.call_generator.is_some() && ecx.tx().caller() != inputs.caller {
            self.override_call(ecx, inputs);
        }

        self.call_depth = self.call_depth.saturating_add(1);
        self.record_observed_call(
            inputs.caller,
            inputs.target_address,
            inputs.input.bytes(ecx),
            inputs.transfer_value().filter(|value| !value.is_zero()),
            inputs.scheme,
        );

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

        self.call_depth = self.call_depth.saturating_sub(1);
    }
}

impl Fuzzer {
    /// Constructs a new `Fuzzer` inspector.
    pub const fn new(
        max_collected_values: usize,
        mapping_slots: Option<AddressMap<MappingSlots>>,
    ) -> Self {
        Self {
            collect: true,
            call_generator: None,
            collected_values: Vec::new(),
            max_collected_values,
            mapping_slots,
            record_calls: false,
            observed_calls: Vec::new(),
            call_depth: 0,
        }
    }

    /// Enables or disables sub-call buffering.
    pub const fn with_call_recording(mut self, record_calls: bool) -> Self {
        self.record_calls = record_calls;
        self
    }

    /// Enables or disables sub-call buffering on an existing inspector.
    pub const fn set_call_recording(&mut self, record_calls: bool) {
        self.record_calls = record_calls;
    }

    /// Returns the buffered sub-calls observed since the last drain.
    pub fn take_observed_calls(&mut self) -> Vec<ObservedCall> {
        std::mem::take(&mut self.observed_calls)
    }

    fn record_observed_call(
        &mut self,
        caller: Address,
        target: Address,
        calldata: Bytes,
        value: Option<U256>,
        scheme: CallScheme,
    ) {
        if self.record_calls && self.call_depth > 1 && matches!(scheme, CallScheme::Call) {
            self.observed_calls.push(ObservedCall {
                depth: self.call_depth - 1,
                caller,
                target,
                calldata,
                value,
            });
        }
    }

    /// Collects `stack` and `memory` values into the fuzz dictionary.
    #[cold]
    fn collect_data(&mut self, interpreter: &Interpreter) {
        let remaining = self.max_collected_values.saturating_sub(self.collected_values.len());
        self.collected_values
            .extend(interpreter.stack.data().iter().take(remaining).copied().map(B256::from));

        // TODO: disabled for now since it's flooding the dictionary
        // for index in 0..interpreter.shared_memory.len() / 32 {
        //     let mut slot = [0u8; 32];
        //     slot.clone_from_slice(interpreter.shared_memory.get_slice(index * 32, 32));

        //     state.insert(slot);
        // }

        self.collect = false;
    }

    /// Drains values observed by the inspector since the last call.
    pub fn drain_collected_values(&mut self) -> Vec<B256> {
        std::mem::take(&mut self.collected_values)
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
    fn override_call<CTX: ContextTr>(&mut self, ecx: &mut CTX, call: &mut CallInputs) {
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
        call.input = CallInput::Bytes(tx.call_details.calldata);
        call.caller = tx.sender;
        call.target_address = tx.call_details.target;
        call.bytecode_address = tx.call_details.target;
        let target = ecx
            .journal_mut()
            .load_account_with_code(tx.call_details.target)
            .expect("failed to load account");
        // Clear known_bytecode to force REVM to load bytecode from the new target.
        // Without this, REVM uses cached bytecode from the original target (e.g., empty
        // bytecode for EOA), causing the call to short-circuit before executing any code.
        call.known_bytecode = (target.info.code_hash, target.info.code.clone().unwrap_or_default());
        // Clear value since ETH was already transferred above
        call.value = CallValue::Transfer(alloy_primitives::U256::ZERO);

        // Track that we're inside an overridden call to avoid recursive overrides
        call_generator.override_depth = 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fuzzer(record_calls: bool) -> Fuzzer {
        Fuzzer::new(16, None).with_call_recording(record_calls)
    }

    #[test]
    fn observed_calls_are_disabled_by_default() {
        let mut fuzzer = Fuzzer::new(16, None);
        fuzzer.call_depth = 2;

        fuzzer.record_observed_call(
            Address::from([0xaa; 20]),
            Address::from([0x11; 20]),
            Bytes::from_static(&[0xde, 0xad, 0xbe, 0xef]),
            Some(U256::from(1)),
            CallScheme::Call,
        );

        assert!(fuzzer.take_observed_calls().is_empty());
    }

    #[test]
    fn observed_calls_skip_top_level_call() {
        let mut fuzzer = fuzzer(true);
        fuzzer.call_depth = 1;

        fuzzer.record_observed_call(
            Address::from([0xaa; 20]),
            Address::from([0x11; 20]),
            Bytes::from_static(&[0xde, 0xad, 0xbe, 0xef]),
            None,
            CallScheme::Call,
        );

        assert!(fuzzer.take_observed_calls().is_empty());
    }

    #[test]
    fn observed_calls_record_subcall_depth_target_calldata_and_value() {
        let mut fuzzer = fuzzer(true);
        let caller = Address::from([0x11; 20]);
        let target = Address::from([0x22; 20]);
        let calldata = Bytes::from_static(&[0xca, 0xfe, 0xba, 0xbe]);
        let value = Some(U256::from(7));
        fuzzer.call_depth = 3;

        fuzzer.record_observed_call(caller, target, calldata.clone(), value, CallScheme::Call);

        assert_eq!(
            fuzzer.take_observed_calls(),
            vec![ObservedCall { depth: 2, caller, target, calldata, value }]
        );
    }

    #[test]
    fn observed_calls_skip_non_call_schemes() {
        let mut fuzzer = fuzzer(true);
        fuzzer.call_depth = 2;

        fuzzer.record_observed_call(
            Address::from([0x11; 20]),
            Address::from([0x22; 20]),
            Bytes::from_static(&[0xde, 0xad, 0xbe, 0xef]),
            None,
            CallScheme::DelegateCall,
        );

        assert!(fuzzer.take_observed_calls().is_empty());
    }

    #[test]
    fn take_observed_calls_drains_buffer() {
        let mut fuzzer = fuzzer(true);
        fuzzer.call_depth = 2;
        fuzzer.record_observed_call(
            Address::from([0xaa; 20]),
            Address::from([0x33; 20]),
            Bytes::from_static(&[0x12, 0x34, 0x56, 0x78]),
            None,
            CallScheme::Call,
        );

        assert_eq!(fuzzer.take_observed_calls().len(), 1);
        assert!(fuzzer.take_observed_calls().is_empty());
    }
}
