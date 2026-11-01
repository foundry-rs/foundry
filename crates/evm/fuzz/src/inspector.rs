use crate::invariant::RandomCallGenerator;
use alloy_primitives::{Address, Bytes, U256};
use evm2::{
    EvmTypes, Inspector,
    interpreter::{Interpreter, Message, MessageKind, MessageResult},
};
use foundry_evm_core::constants::CHEATCODE_ADDRESS;

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

/// An inspector that overrides calls to simulate reentrancy and records sub-calls for the
/// invariant fuzzer.
#[derive(Clone, Debug, Default)]
pub struct Fuzzer {
    /// Given a strategy, it generates a random call.
    pub call_generator: Option<RandomCallGenerator>,
    /// Whether sub-calls should be buffered for later corpus seeding.
    record_calls: bool,
    /// Sub-calls observed since the last drain.
    observed_calls: Vec<ObservedCall>,
    /// Current EVM call depth. 0 means no active call, 1 means top-level call.
    call_depth: u32,
    /// Additional network-specific cheatcode addresses that must not be overridden.
    extra_cheatcode_addresses: &'static [Address],
}

impl<T: EvmTypes> Inspector<T> for Fuzzer {
    fn call(
        &mut self,
        interp: &mut Interpreter<'_, '_, T>,
        message: &mut Message<T>,
    ) -> Option<MessageResult<T>> {
        // We don't want to override the very first call made to the test contract.
        if self.call_generator.is_some() && interp.host().tx_env().origin != message.caller {
            self.override_call(interp, message);
        }

        self.call_depth = self.call_depth.saturating_add(1);
        if self.should_record_observed_call(message.kind) {
            self.observed_calls.push(ObservedCall {
                depth: self.call_depth - 1,
                caller: message.caller,
                target: message.destination,
                calldata: message.input.clone(),
                value: (!message.value.is_zero()).then_some(message.value),
            });
        }

        None
    }

    fn call_end(
        &mut self,
        _interp: &mut Interpreter<'_, '_, T>,
        _message: &Message<T>,
        _result: &mut MessageResult<T>,
    ) {
        if let Some(ref mut call_generator) = self.call_generator {
            // Decrement depth when any call ends while inside an override
            if call_generator.override_depth > 0 {
                call_generator.override_depth -= 1;
            }
        }

        self.call_depth = self.call_depth.saturating_sub(1);
    }
}

impl Fuzzer {
    /// Sets additional network-specific cheatcode addresses that must not be overridden.
    pub const fn with_extra_cheatcode_addresses(mut self, addresses: &'static [Address]) -> Self {
        self.extra_cheatcode_addresses = addresses;
        self
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

    #[inline]
    const fn should_record_observed_call(&self, kind: MessageKind) -> bool {
        self.record_calls && self.call_depth > 1 && matches!(kind, MessageKind::Call)
    }

    #[inline]
    fn is_cheatcode_address(&self, address: Address) -> bool {
        address == CHEATCODE_ADDRESS || self.extra_cheatcode_addresses.contains(&address)
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
    fn override_call<T: EvmTypes>(
        &mut self,
        interp: &mut Interpreter<'_, '_, T>,
        call: &mut Message<T>,
    ) {
        let target_is_cheatcode = self.is_cheatcode_address(call.destination);
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
        if call.caller == call_generator.test_address
            || !matches!(call.kind, MessageKind::Call)
            || call_generator.override_depth > 0
            || target_is_cheatcode
        {
            return;
        }
        {
            let handlers = call_generator.handler_addresses.read();
            if !handlers.contains(&call.caller) && !handlers.contains(&call.destination) {
                return;
            }
        }

        // There's only a ~27% chance that an override happens (90% * 30% from strategy).
        let Some(tx) = call_generator.next(call.caller, call.destination) else {
            return;
        };

        // For value transfers, perform the ETH transfer before injecting the callback.
        // This simulates a malicious receive() that gets the ETH and then reenters.
        let state = interp.host().state_mut();
        let has_value = !call.value.is_zero() && call.gas_limit > 2300;
        if has_value && state.transfer(&call.caller, &call.destination, &call.value).is_err() {
            return;
        }

        // Replace the call with a reentrant callback. The message carries the code it runs, so
        // load the new target's code instead of running the original target's.
        let target = tx.call_details.target;
        call.code = state
            .account(&target)
            .and_then(|mut account| account.load_code())
            .expect("failed to load account");
        call.input = tx.call_details.calldata;
        call.caller = tx.sender;
        call.destination = target;
        call.call_target = target;
        call.code_address = target;
        // Clear value since ETH was already transferred above
        call.value = U256::ZERO;

        // Track that we're inside an overridden call to avoid recursive overrides
        call_generator.override_depth = 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BasicTxDetails, CallDetails};
    use alloy_network::TransactionBuilder;
    use alloy_primitives::{B256, keccak256, map::AddressSet};
    use evm2::{
        Evm,
        bytecode::Bytecode,
        evm::{AccountInfo, InMemoryDB},
    };
    use foundry_evm_core::{
        constants::MONAD_CHEATCODE_ADDRESS,
        evm::{
            EthEvmNetwork, EvmEnv, EvmFactoryFor, FoundryEvmFactory, FoundryEvmNetwork, SpecFor,
            TransactionRequestFor, TypesFor,
        },
    };
    use parking_lot::RwLock;
    use proptest::{strategy::Just, test_runner::TestRunner};
    use std::sync::Arc;

    const CALLER: Address = Address::with_last_byte(0xCA);
    const HANDLER: Address = Address::with_last_byte(0xA1);
    const VICTIM: Address = Address::with_last_byte(0xA2);
    const REENTRANT: Address = Address::with_last_byte(0xA3);

    /// `call(gas(), VICTIM, 0, 0, 0, 0, 0); stop`.
    fn handler_code() -> Vec<u8> {
        let mut code = [0x60, 0x00].repeat(5);
        code.push(0x73);
        code.extend_from_slice(VICTIM.as_slice());
        code.extend([0x5a, 0xf1, 0x00]);
        code
    }

    /// `mstore(0, 7); mstore(32, 1); sstore(keccak256(0, 64), 2); stop`.
    const VICTIM_CODE: &[u8] = &[
        0x60, 0x07, 0x60, 0x00, 0x52, 0x60, 0x01, 0x60, 0x20, 0x52, 0x60, 0x40, 0x60, 0x00, 0x20,
        0x60, 0x02, 0x90, 0x55, 0x00,
    ];

    /// `sstore(0, 1); stop`.
    const REENTRANT_CODE: &[u8] = &[0x60, 0x01, 0x60, 0x00, 0x55, 0x00];

    /// Calls `HANDLER`, running `handler`, from `CALLER` with `fuzzer` on `FEN`'s default spec.
    fn run<FEN: FoundryEvmNetwork>(
        fuzzer: Fuzzer,
        handler: &[u8],
    ) -> (Evm<'static, TypesFor<FEN>>, Fuzzer)
    where
        TransactionRequestFor<FEN>: Default,
    {
        let chain_id = 31337;
        let mut db = InMemoryDB::default();
        db.insert_account_info(&CALLER, AccountInfo::default().with_balance(U256::from(1e18)));
        for (address, code) in
            [(HANDLER, handler), (VICTIM, VICTIM_CODE), (REENTRANT, REENTRANT_CODE)]
        {
            let code = Bytecode::new_legacy(Bytes::copy_from_slice(code));
            let account = AccountInfo::default().with_balance(U256::from(7)).with_code(code);
            db.insert_account_info(&address, account);
        }
        let factory = EvmFactoryFor::<FEN>::default();
        let spec = SpecFor::<FEN>::default();
        let env = EvmEnv::new(spec, factory.version(spec, chain_id), Default::default());
        let mut evm = factory.create_evm(db, env);
        evm.set_inspector(fuzzer);
        let request = TransactionRequestFor::<FEN>::default()
            .with_from(CALLER)
            .with_to(HANDLER)
            .with_nonce(0)
            .with_chain_id(chain_id)
            .with_gas_limit(1_000_000)
            .with_max_fee_per_gas(0)
            .with_max_priority_fee_per_gas(0);
        let tx = FEN::tx_from_request(request).unwrap();
        assert!(evm.transact(&tx).unwrap().commit().status);
        let fuzzer = *evm.clear_inspector_as::<Fuzzer>().unwrap();
        (evm, fuzzer)
    }

    fn slot<T: EvmTypes>(evm: &mut Evm<'_, T>, address: Address, slot: U256) -> U256 {
        evm.state_mut().storage_slot_untracked(&address, &slot).unwrap()
    }

    fn fuzzer_suite<FEN: FoundryEvmNetwork>()
    where
        TransactionRequestFor<FEN>: Default,
    {
        // Observation: the sub-call. The handler writes the mapping slot.
        let fuzzer = Fuzzer::default().with_call_recording(true);
        let (mut evm, mut fuzzer) = run::<FEN>(fuzzer, &handler_code());
        let mapping_slot = keccak256([B256::with_last_byte(7), B256::with_last_byte(1)].concat());
        assert_eq!(slot(&mut evm, VICTIM, mapping_slot.into()), U256::from(2));
        assert_eq!(
            fuzzer.take_observed_calls(),
            [ObservedCall {
                depth: 1,
                caller: HANDLER,
                target: VICTIM,
                calldata: Bytes::new(),
                value: None,
            }]
        );

        // Override: the handler's call to the victim reenters through `REENTRANT` instead.
        let callback = BasicTxDetails {
            warp: None,
            roll: None,
            sender: VICTIM,
            call_details: CallDetails { target: REENTRANT, calldata: Bytes::new(), value: None },
        };
        let mut generator = RandomCallGenerator::new(
            Address::with_last_byte(0x7E),
            AddressSet::from_iter([HANDLER]),
            TestRunner::deterministic(),
            Just(callback.call_details.clone()),
            Arc::new(RwLock::new(Address::ZERO)),
        );
        generator.last_sequence.write().push(Some(callback));
        generator.set_replay(true);
        let fuzzer = Fuzzer { call_generator: Some(generator), ..Default::default() };
        let (mut evm, _) = run::<FEN>(fuzzer, &handler_code());
        assert_eq!(slot(&mut evm, REENTRANT, U256::ZERO), U256::ONE);
        assert_eq!(slot(&mut evm, VICTIM, mapping_slot.into()), U256::ZERO);
    }

    #[test]
    fn fuzzer_networks() {
        fuzzer_suite::<EthEvmNetwork>();
        fuzzer_suite::<foundry_evm_core::evm::TempoEvmNetwork>();
        #[cfg(feature = "optimism")]
        fuzzer_suite::<foundry_evm_core::evm::OpEvmNetwork>();
        #[cfg(feature = "base")]
        fuzzer_suite::<foundry_evm_core::evm::BaseEvmNetwork>();
        #[cfg(feature = "monad")]
        fuzzer_suite::<foundry_evm_core::evm::MonadEvmNetwork>();
    }

    #[test]
    fn network_cheatcode_addresses_are_opt_in() {
        let ethereum = Fuzzer::default();
        assert!(!ethereum.is_cheatcode_address(MONAD_CHEATCODE_ADDRESS));

        let monad = Fuzzer::default().with_extra_cheatcode_addresses(&[MONAD_CHEATCODE_ADDRESS]);
        assert!(monad.is_cheatcode_address(MONAD_CHEATCODE_ADDRESS));
    }

    #[test]
    fn observed_calls_are_disabled_by_default() {
        let (_, mut fuzzer) = run::<EthEvmNetwork>(Fuzzer::default(), &handler_code());
        assert!(fuzzer.take_observed_calls().is_empty());
    }

    #[test]
    fn observed_calls_record_only_valued_subcalls() {
        // `mstore(0, 0xcafebabe); call(gas(), VICTIM, 7, 28, 4, 0, 0);
        // delegatecall(gas(), VICTIM, 0, 0, 0, 0); stop`.
        let mut handler = vec![0x63, 0xca, 0xfe, 0xba, 0xbe, 0x60, 0x00, 0x52];
        handler.extend([0x60, 0x00, 0x60, 0x00, 0x60, 0x04, 0x60, 0x1c, 0x60, 0x07, 0x73]);
        handler.extend_from_slice(VICTIM.as_slice());
        handler.extend([0x5a, 0xf1, 0x50]);
        handler.extend([0x60, 0x00, 0x60, 0x00, 0x60, 0x00, 0x60, 0x00, 0x73]);
        handler.extend_from_slice(VICTIM.as_slice());
        handler.extend([0x5a, 0xf4, 0x50, 0x00]);

        let fuzzer = Fuzzer::default().with_call_recording(true);
        let (_, mut fuzzer) = run::<EthEvmNetwork>(fuzzer, &handler);

        // The top-level call and the delegate call are not recorded.
        assert_eq!(
            fuzzer.take_observed_calls(),
            [ObservedCall {
                depth: 1,
                caller: HANDLER,
                target: VICTIM,
                calldata: Bytes::from_static(&[0xca, 0xfe, 0xba, 0xbe]),
                value: Some(U256::from(7)),
            }]
        );
        assert!(fuzzer.take_observed_calls().is_empty());
    }
}
