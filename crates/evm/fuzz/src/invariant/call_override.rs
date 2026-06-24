use crate::{BasicTxDetails, CallDetails};
use alloy_primitives::Address;
use parking_lot::{Mutex, RwLock};
use proptest::{
    option::weighted,
    strategy::{SBoxedStrategy, Strategy, ValueTree},
    test_runner::TestRunner,
};
use std::{collections::HashSet, sync::Arc};

/// Given a TestRunner and a strategy, it generates calls. Used inside the Fuzzer inspector to
/// override external calls to test for potential reentrancy vulnerabilities.
///
/// The key insight is that we only override calls TO handler contracts (targeted contracts).
/// This simulates a malicious contract that reenters when receiving ETH via its receive() function.
#[derive(Clone, Debug)]
pub struct RandomCallGenerator {
    /// Address of the test contract.
    pub test_address: Address,
    /// Addresses of handler contracts that can be reentered.
    /// We only inject callbacks when the call target is one of these.
    pub handler_addresses: Arc<RwLock<HashSet<Address>>>,
    /// Runner that will generate the call from the strategy.
    pub runner: Arc<Mutex<TestRunner>>,
    /// Strategy to be used to generate calls from `target_reference`.
    pub strategy: SBoxedStrategy<Option<CallDetails>>,
    /// Reference to which contract we want a fuzzed calldata from.
    pub target_reference: Arc<RwLock<Address>>,
    /// Tracks the call depth when an override is active. When > 0, we're inside an overridden
    /// call and should not override nested calls. Incremented when we override a call,
    /// decremented when any call ends while inside an override.
    pub override_depth: usize,
    /// If set to `true`, consumes the next call from `last_sequence`, otherwise queries it from
    /// the strategy.
    pub replay: bool,
    /// Saves the sequence of generated calls that can be replayed later on.
    pub last_sequence: Arc<RwLock<Vec<Option<BasicTxDetails>>>>,
}

impl RandomCallGenerator {
    pub fn new(
        test_address: Address,
        handler_addresses: HashSet<Address>,
        runner: TestRunner,
        strategy: impl Strategy<Value = CallDetails> + Send + Sync + 'static,
        target_reference: Arc<RwLock<Address>>,
    ) -> Self {
        Self {
            test_address,
            handler_addresses: Arc::new(RwLock::new(handler_addresses)),
            runner: Arc::new(Mutex::new(runner)),
            strategy: weighted(0.9, strategy).sboxed(),
            target_reference,
            last_sequence: Arc::default(),
            replay: false,
            override_depth: 0,
        }
    }

    /// Check if the given address is a handler that can be reentered.
    pub fn is_handler(&self, address: Address) -> bool {
        self.handler_addresses.read().contains(&address)
    }

    /// All `self.next()` calls will now pop `self.last_sequence`. Used to replay an invariant
    /// failure.
    pub fn set_replay(&mut self, status: bool) {
        self.replay = status;
        if status {
            // So it can later be popped.
            self.last_sequence.write().reverse();
        }
    }

    /// Gets the next call. Random if replay is not set. Otherwise, it pops from `last_sequence`.
    pub fn next(
        &mut self,
        original_caller: Address,
        original_target: Address,
    ) -> Option<BasicTxDetails> {
        if self.replay {
            self.last_sequence.write().pop().expect(
                "to have same size as the number of (unsafe) external calls of the sequence.",
            )
        } else {
            // TODO: Do we want it to be 80% chance only too ?
            let sender = original_target;

            // Set which contract we mostly (80% chance) want to generate calldata from.
            *self.target_reference.write() = original_caller;

            // `original_caller` has a 80% chance of being the `new_target`.
            let choice = self.strategy.new_tree(&mut self.runner.lock()).unwrap().current().map(
                |call_details| BasicTxDetails { warp: None, roll: None, sender, call_details },
            );

            self.last_sequence.write().push(choice.clone());
            choice
        }
    }
}
