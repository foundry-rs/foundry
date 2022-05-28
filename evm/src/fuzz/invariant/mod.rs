//! Fuzzing support abstracted over the [`Evm`](crate::Evm) used
use crate::fuzz::*;

mod executor;
mod filters;

use ethers::{
    abi::{Abi, Function},
    prelude::ArtifactId,
    types::{Address, Bytes, U256},
};
pub use proptest::test_runner::Config as FuzzConfig;
use proptest::{
    strategy::SBoxedStrategy,
    test_runner::{TestError, TestRunner},
};
use revm::db::DatabaseRef;
use std::{
    borrow::{Borrow, BorrowMut},
    cell::RefMut,
    collections::BTreeMap,
    sync::{Arc, RwLock},
};

use crate::executor::{Executor, RawCallResult};

use proptest::strategy::{Strategy, ValueTree};

pub type TargetedContracts = BTreeMap<Address, (String, Abi, Vec<Function>)>;
pub type FuzzRunIdentifiedContracts = Arc<RwLock<TargetedContracts>>;
pub type BasicTxDetails = (Address, (Address, Bytes));

/// Wrapper around any [`Executor`] implementor which provides fuzzing support using [`proptest`](https://docs.rs/proptest/1.0.0/proptest/).
///
/// After instantiation, calling `fuzz` will proceed to hammer the deployed smart contracts with
/// inputs, until it finds a counterexample sequence. The provided [`TestRunner`] contains all the
/// configuration which can be overridden via [environment variables](https://docs.rs/proptest/1.0.0/proptest/test_runner/struct.Config.html)
pub struct InvariantExecutor<'a, DB: DatabaseRef + Clone> {
    // evm: RefCell<&'a mut E>,
    /// The VM todo executor
    pub evm: &'a mut Executor<DB>,
    runner: TestRunner,
    sender: Address,
    setup_contracts: &'a BTreeMap<Address, (String, Abi)>,
    project_contracts: &'a BTreeMap<ArtifactId, (Abi, Vec<u8>)>,
}

/// Given the executor state, asserts that no invariant has been broken. Otherwise, it fills the
/// external `invariant_doesnt_hold` map and returns `Err(())`
pub fn assert_invariants<'a, DB>(
    sender: Address,
    abi: &Abi,
    mut executor: RefMut<&mut &mut Executor<DB>>,
    invariant_address: Address,
    invariants: &'a [&Function],
    mut invariant_doesnt_hold: RefMut<BTreeMap<String, Option<InvariantFuzzError>>>,
    inputs: &[BasicTxDetails],
) -> eyre::Result<()>
where
    DB: DatabaseRef,
{
    let mut found_case = false;
    let inner_sequence = {
        let generator = &mut executor.inspector_config.fuzzer.as_mut().unwrap().generator;

        // // will need the exact depth and all to replay
        let sequence = generator.last_sequence.read().unwrap().clone();
        sequence
    };

    for func in invariants {
        let RawCallResult { reverted, state_changeset, result, .. } = executor
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
    pub inner_sequence: Vec<BasicTxDetails>,
}

impl InvariantFuzzError {
    fn new(
        invariant_address: Address,
        error_func: Option<&Function>,
        abi: &Abi,
        result: &bytes::Bytes,
        inputs: &[BasicTxDetails],
        inner_sequence: &[BasicTxDetails],
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
/// override external calls to test for reentrancy.
#[derive(Debug, Clone)]
pub struct RandomCallGenerator {
    /// Runner that will generate the call from the strategy.
    pub runner: Arc<RwLock<TestRunner>>,
    /// Strategy to be used to generate calls.
    pub strategy: SBoxedStrategy<Vec<BasicTxDetails>>,
    /// Flag to know if a call has been overriden.
    pub used: bool,
    /// If set to `true`, consumes the next call from `last_sequence`, otherwise from the strategy.
    pub replay: bool,
    /// Saves the sequence of generated calls that can be replayed later on.
    pub last_sequence: Arc<RwLock<Vec<BasicTxDetails>>>,
}

impl RandomCallGenerator {
    pub fn set_replay(&mut self, status: bool) {
        self.replay = status;
        if status {
            // So it can later be popped.
            self.last_sequence.write().unwrap().reverse()
        }
    }

    /// Gets the next call. Random if replay is not set. Otherwise, it pops from `last_sequence`.
    pub fn next(&mut self, original_caller: Address, original_target: Address) -> BasicTxDetails {
        if self.replay {
            self.last_sequence.write().unwrap().pop().unwrap()
        } else {
            let mut testrunner = self.runner.write().unwrap();
            let calldata;

            loop {
                let mut reentrant_call = self.strategy.new_tree(&mut testrunner).unwrap().current();

                let (_, (contract, data)) = reentrant_call.pop().unwrap();

                // Only accepting calls made to the one who called `original_target`.
                if contract == original_caller {
                    calldata = data;
                    break
                }
            }

            self.last_sequence
                .write()
                .unwrap()
                .push((original_target, (original_caller, calldata.clone())));

            (original_target, (original_caller, calldata))
        }
    }
}
