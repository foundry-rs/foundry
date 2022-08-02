//! Fuzzing support abstracted over the [`Evm`](crate::Evm) used
use crate::{fuzz::*, CALLER};
mod executor;
use crate::{
    decode::decode_revert,
    executor::{Executor, RawCallResult},
};
use ethers::{
    abi::{Abi, Function},
    types::{Address, Bytes, U256},
};
pub use executor::{InvariantExecutor, InvariantFailures};
use parking_lot::{Mutex, RwLock};
pub use proptest::test_runner::Config as FuzzConfig;
use proptest::{
    option::weighted,
    strategy::{SBoxedStrategy, Strategy, ValueTree},
    test_runner::{TestError, TestRunner},
};
use std::{collections::BTreeMap, sync::Arc};

pub type TargetedContracts = BTreeMap<Address, (String, Abi, Vec<Function>)>;
pub type FuzzRunIdentifiedContracts = Arc<Mutex<TargetedContracts>>;

/// (Sender, (TargetContract, Calldata))
pub type BasicTxDetails = (Address, (Address, Bytes));

/// Test contract which is testing its invariants.
#[derive(Debug, Clone)]
pub struct InvariantContract<'a> {
    /// Address of the test contract.
    pub address: Address,
    /// Invariant functions present in the test contract.
    pub invariant_functions: Vec<&'a Function>,
    /// Abi of the test contract.
    pub abi: &'a Abi,
}

/// Metadata on how to run invariant tests
#[derive(Debug, Clone, Copy, Default)]
pub struct InvariantTestOptions {
    /// The number of calls executed to attempt to break invariants in one run.
    pub depth: u32,
    /// Fails the invariant fuzzing if a revert occurs
    pub fail_on_revert: bool,
    /// Allows overriding an unsafe external call when running invariant tests. eg. reetrancy
    /// checks
    pub call_override: bool,
}

/// Given the executor state, asserts that no invariant has been broken. Otherwise, it fills the
/// external `invariant_failures.failed_invariant` map and returns a generic error.
pub fn assert_invariants(
    invariant_contract: &InvariantContract,
    executor: &Executor,
    calldata: &[BasicTxDetails],
    invariant_failures: &mut InvariantFailures,
) -> eyre::Result<()> {
    let mut found_case = false;
    let mut inner_sequence = vec![];

    if let Some(ref fuzzer) = executor.inspector_config().fuzzer {
        if let Some(ref call_generator) = fuzzer.call_generator {
            inner_sequence.extend(call_generator.last_sequence.read().iter().cloned());
        }
    }

    for func in &invariant_contract.invariant_functions {
        let mut call_result = executor
            .call_raw(
                CALLER,
                invariant_contract.address,
                func.encode_input(&[]).expect("invariant should have no inputs").into(),
                U256::zero(),
            )
            .expect("EVM error");

        let err = if call_result.reverted {
            Some(*func)
        } else {
            // This will panic and get caught by the executor
            if !executor.is_success(
                invariant_contract.address,
                call_result.reverted,
                call_result.state_changeset.take().expect("we should have a state changeset"),
                false,
            ) {
                Some(*func)
            } else {
                None
            }
        };

        if let Some(broken_invariant) = err {
            let invariant_error = invariant_failures
                .failed_invariants
                .get(&broken_invariant.name)
                .expect("to have been initialized.");

            // We only care about invariants which we haven't broken yet.
            if invariant_error.is_none() {
                invariant_failures.failed_invariants.insert(
                    broken_invariant.name.clone(),
                    Some(InvariantFuzzError::new(
                        invariant_contract,
                        Some(broken_invariant),
                        calldata,
                        call_result,
                        &inner_sequence,
                    )),
                );
                found_case = true;
            }
        }
    }

    if found_case {
        let before = invariant_failures.broken_invariants_count;

        invariant_failures.broken_invariants_count = invariant_failures
            .failed_invariants
            .iter()
            .filter(|(_function, error)| error.is_some())
            .count();

        eyre::bail!(
            "{} new invariants have been broken.",
            invariant_failures.broken_invariants_count - before
        );
    }
    Ok(())
}

/// The outcome of an invariant fuzz test
#[derive(Debug)]
pub struct InvariantFuzzTestResult {
    pub invariants: BTreeMap<String, Option<InvariantFuzzError>>,
    /// Every successful fuzz test case
    pub cases: Vec<FuzzedCases>,
    /// Number of reverted fuzz calls
    pub reverts: usize,
}

#[derive(Debug, Clone)]
pub struct InvariantFuzzError {
    /// The proptest error occurred as a result of a test case.
    pub test_error: TestError<Vec<BasicTxDetails>>,
    /// The return reason of the offending call.
    pub return_reason: Reason,
    /// The revert string of the offending call.
    pub revert_reason: String,
    /// Address of the invariant asserter.
    pub addr: Address,
    /// Function data for invariant check.
    pub func: Option<ethers::prelude::Bytes>,
    /// Inner fuzzing Sequence coming from overriding calls.
    pub inner_sequence: Vec<Option<BasicTxDetails>>,
}

impl InvariantFuzzError {
    fn new(
        invariant_contract: &InvariantContract,
        error_func: Option<&Function>,
        calldata: &[BasicTxDetails],
        call_result: RawCallResult,
        inner_sequence: &[Option<BasicTxDetails>],
    ) -> Self {
        let mut func = None;
        let origin: String;

        if let Some(f) = error_func {
            func = Some(f.short_signature().into());
            origin = f.name.clone();
        } else {
            origin = "Revert".to_string();
        }

        InvariantFuzzError {
            test_error: proptest::test_runner::TestError::Fail(
                format!(
                    "{}, reason: '{}'",
                    origin,
                    match decode_revert(
                        call_result.result.as_ref(),
                        Some(invariant_contract.abi),
                        Some(call_result.status)
                    ) {
                        Ok(e) => e,
                        Err(e) => e.to_string(),
                    }
                )
                .into(),
                calldata.to_vec(),
            ),
            return_reason: "".into(),
            revert_reason: decode_revert(
                call_result.result.as_ref(),
                Some(invariant_contract.abi),
                Some(call_result.status),
            )
            .unwrap_or_default(),
            addr: invariant_contract.address,
            func,
            inner_sequence: inner_sequence.to_vec(),
        }
    }
}

/// Given a TestRunner and a strategy, it generates calls. Used inside the Fuzzer inspector to
/// override external calls to test for potential reentrancy vulnerabilities..
#[derive(Debug, Clone)]
pub struct RandomCallGenerator {
    /// Address of the test contract.
    pub test_address: Address,
    /// Runner that will generate the call from the strategy.
    pub runner: Arc<RwLock<TestRunner>>,
    /// Strategy to be used to generate calls from `target_reference`.
    pub strategy: SBoxedStrategy<Option<(Address, Bytes)>>,
    /// Reference to which contract we want a fuzzed calldata from.
    pub target_reference: Arc<RwLock<Address>>,
    /// Flag to know if a call has been overriden. Don't allow nesting for now.
    pub used: bool,
    /// If set to `true`, consumes the next call from `last_sequence`, otherwise queries it from
    /// the strategy.
    pub replay: bool,
    /// Saves the sequence of generated calls that can be replayed later on.
    pub last_sequence: Arc<RwLock<Vec<Option<BasicTxDetails>>>>,
}

impl RandomCallGenerator {
    pub fn new(
        test_address: Address,
        runner: TestRunner,
        strategy: SBoxedStrategy<(Address, Bytes)>,
        target_reference: Arc<RwLock<Address>>,
    ) -> Self {
        let strategy = weighted(0.9, strategy).sboxed();

        RandomCallGenerator {
            test_address,
            runner: Arc::new(RwLock::new(runner)),
            strategy,
            target_reference,
            last_sequence: Arc::new(RwLock::new(vec![])),
            replay: false,
            used: false,
        }
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
            let new_caller = original_target;

            // Set which contract we mostly (80% chance) want to generate calldata from.
            *self.target_reference.write() = original_caller;

            // `original_caller` has a 80% chance of being the `new_target`.
            let choice = self
                .strategy
                .new_tree(&mut self.runner.write())
                .unwrap()
                .current()
                .map(|(new_target, calldata)| (new_caller, (new_target, calldata)));

            self.last_sequence.write().push(choice.clone());
            choice
        }
    }
}
