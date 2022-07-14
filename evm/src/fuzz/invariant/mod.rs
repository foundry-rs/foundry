//! Fuzzing support abstracted over the [`Evm`](crate::Evm) used
use crate::fuzz::*;

mod executor;
mod filters;

use ethers::{
    abi::{Abi, Function},
    prelude::ArtifactId,
    types::{Address, Bytes, U256},
};
use parking_lot::RwLock;
pub use proptest::test_runner::Config as FuzzConfig;
use proptest::{
    option::weighted,
    strategy::SBoxedStrategy,
    test_runner::{TestError, TestRunner},
};

use std::{borrow::BorrowMut, cell::RefMut, collections::BTreeMap, sync::Arc};

use crate::executor::{Executor, RawCallResult};

use proptest::strategy::{Strategy, ValueTree};

pub type TargetedContracts = BTreeMap<Address, (String, Abi, Vec<Function>)>;
pub type FuzzRunIdentifiedContracts = Arc<RwLock<TargetedContracts>>;
pub type BasicTxDetails = (Address, (Address, Bytes));

/// Metadata on how to run invariant tests
#[derive(Debug, Clone, Copy, Default)]
pub struct InvariantTestOptions {
    /// The number of calls executed to attempt to break invariants in one run.
    pub depth: u32,
    /// Fails the invariant fuzzing if a reversion occurs
    pub fail_on_revert: bool,
    /// Allows randomly overriding an external call when running invariant tests
    pub call_override: bool,
}

/// Wrapper around any [`Executor`] implementor which provides fuzzing support using [`proptest`](https://docs.rs/proptest/1.0.0/proptest/).
///
/// After instantiation, calling `fuzz` will proceed to hammer the deployed smart contracts with
/// inputs, until it finds a counterexample sequence. The provided [`TestRunner`] contains all the
/// configuration which can be overridden via [environment variables](https://docs.rs/proptest/1.0.0/proptest/test_runner/struct.Config.html)
pub struct InvariantExecutor<'a> {
    // evm: RefCell<&'a mut E>,
    /// The VM todo executor
    pub evm: &'a mut Executor,
    runner: TestRunner,
    sender: Address,
    setup_contracts: &'a BTreeMap<Address, (String, Abi)>,
    project_contracts: &'a BTreeMap<ArtifactId, (Abi, Vec<u8>)>,
}

/// Given the executor state, asserts that no invariant has been broken. Otherwise, it fills the
/// external `invariant_doesnt_hold` map and returns `Err(())`
pub fn assert_invariants<'a>(
    sender: Address,
    abi: &Abi,
    executor: &'a RefCell<&mut &mut Executor>,
    invariant_address: Address,
    invariants: &'a [&Function],
    mut invariant_doesnt_hold: RefMut<BTreeMap<String, Option<InvariantFuzzError>>>,
    inputs: &[BasicTxDetails],
) -> eyre::Result<()> {
    let mut found_case = false;
    let mut inner_sequence = vec![];

    if let Some(ref fuzzer) = executor.borrow().inspector_config().fuzzer {
        if let Some(ref generator) = fuzzer.generator {
            inner_sequence.extend(generator.last_sequence.read().iter().cloned());
        }
    }

    for func in invariants {
        let RawCallResult { reverted, state_changeset, result, .. } = executor
            .borrow()
            .call_raw(
                sender,
                invariant_address,
                func.encode_input(&[]).expect("invariant should have no inputs").into(),
                U256::zero(),
            )
            .expect("EVM error");

        let err = if reverted {
            Some((*func, result))
        } else {
            // This will panic and get caught by the executor
            if !executor.borrow().is_success(
                invariant_address,
                reverted,
                state_changeset.expect("we should have a state changeset"),
                false,
            ) {
                Some((*func, result))
            } else {
                None
            }
        };

        if let Some((func, result)) = err {
            invariant_doesnt_hold.borrow_mut().insert(
                func.name.clone(),
                Some(InvariantFuzzError::new(
                    invariant_address,
                    Some(func),
                    abi,
                    &result,
                    inputs,
                    &inner_sequence,
                )),
            );
            found_case = true;
        }
    }

    if found_case {
        eyre::bail!("");
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
    /// The proptest error occurred as a result of a test case
    pub test_error: TestError<Vec<BasicTxDetails>>,
    /// The return reason of the offending call
    pub return_reason: Reason,
    /// The revert string of the offending call
    pub revert_reason: String,
    /// Address of the invariant asserter
    pub addr: Address,
    /// Function data for invariant check
    pub func: Option<ethers::prelude::Bytes>,
    /// Inner Fuzzing Sequence
    pub inner_sequence: Vec<Option<BasicTxDetails>>,
}

impl InvariantFuzzError {
    fn new(
        invariant_address: Address,
        error_func: Option<&Function>,
        abi: &Abi,
        result: &bytes::Bytes,
        inputs: &[BasicTxDetails],
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
                    match foundry_utils::decode_revert(result.as_ref(), Some(abi)) {
                        Ok(e) => e,
                        Err(e) => e.to_string(),
                    }
                )
                .into(),
                inputs.to_vec(),
            ),
            return_reason: "".into(),
            // return_reason: status,
            revert_reason: foundry_utils::decode_revert(result.as_ref(), Some(abi))
                .unwrap_or_default(),
            addr: invariant_address,
            func,
            inner_sequence: inner_sequence.to_vec(),
        }
    }
}

/// Given a TestRunner and a strategy, it generates calls. Used inside the Fuzzer inspector to
/// override external calls to test for potential reentrancy vulnerabilities..
#[derive(Debug, Clone)]
pub struct RandomCallGenerator {
    /// Runner that will generate the call from the strategy.
    pub runner: Arc<RwLock<TestRunner>>,
    /// Strategy to be used to generate calls from `target_reference`.
    pub strategy: SBoxedStrategy<Option<(Address, Bytes)>>,
    /// Reference to which contract we want a fuzzed calldata from.
    pub target_reference: Arc<RwLock<Address>>,
    /// Flag to know if a call has been overriden. Don't allow nested for now.
    pub used: bool,
    /// If set to `true`, consumes the next call from `last_sequence`, otherwise from the strategy.
    pub replay: bool,
    /// Saves the sequence of generated calls that can be replayed later on.
    pub last_sequence: Arc<RwLock<Vec<Option<BasicTxDetails>>>>,
}

impl RandomCallGenerator {
    pub fn new(
        runner: TestRunner,
        strategy: SBoxedStrategy<(Address, Bytes)>,
        target_reference: Arc<RwLock<Address>>,
    ) -> Self {
        let strategy = weighted(0.3, strategy).sboxed();

        RandomCallGenerator {
            runner: Arc::new(RwLock::new(runner)),
            strategy,
            target_reference,
            last_sequence: Arc::new(RwLock::new(vec![])),
            replay: false,
            used: false,
        }
    }
    pub fn set_replay(&mut self, status: bool) {
        self.replay = status;
        if status {
            // So it can later be popped.
            self.last_sequence.write().reverse()
        }
    }

    /// Gets the next call. Random if replay is not set. Otherwise, it pops from `last_sequence`.
    pub fn next(
        &mut self,
        original_caller: Address,
        original_target: Address,
    ) -> Option<BasicTxDetails> {
        if self.replay {
            self.last_sequence.write().pop().unwrap()
        } else {
            let mut testrunner = self.runner.write();
            let mut last_sequence = self.last_sequence.write();
            let new_caller = original_target;

            // Set which contract we mostly want to generate calldata from.
            *self.target_reference.write() = original_caller;

            // `original_caller` has a 80% chance of being the `new_target`.
            let choice = self
                .strategy
                .new_tree(&mut testrunner)
                .unwrap()
                .current()
                .map(|(new_target, calldata)| (new_caller, (new_target, calldata)));

            last_sequence.push(choice.clone());
            choice
        }
    }
}
