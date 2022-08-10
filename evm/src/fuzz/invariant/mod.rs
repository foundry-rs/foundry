//! Fuzzing support abstracted over the [`Evm`](crate::Evm) used
use crate::{
    fuzz::*,
    trace::{load_contracts, TraceKind},
    CALLER,
};
mod filters;
pub use filters::ArtifactFilters;
mod call_override;
pub use call_override::{set_up_inner_replay, RandomCallGenerator};
use foundry_common::contracts::{ContractsByAddress, ContractsByArtifact};
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
use parking_lot::Mutex;
pub use proptest::test_runner::Config as FuzzConfig;
use proptest::test_runner::TestError;
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

    /// Replays the error case and collects all necessary traces.
    pub fn replay(
        &self,
        mut executor: Executor,
        known_contracts: Option<&ContractsByArtifact>,
        mut ided_contracts: ContractsByAddress,
        logs: &mut Vec<Log>,
        traces: &mut Vec<(TraceKind, CallTraceArena)>,
    ) -> Option<CounterExample> {
        let mut counterexample_sequence = vec![];
        let calls = match self.test_error {
            // Don't use at the moment.
            TestError::Abort(_) => return None,
            TestError::Fail(_, ref calls) => calls,
        };

        // We want traces for a failed case.
        executor.set_tracing(true);

        set_up_inner_replay(&mut executor, &self.inner_sequence);

        // Replay each call from the sequence until we break the invariant.
        for (sender, (addr, bytes)) in calls.iter() {
            let call_result = executor
                .call_raw_committing(*sender, *addr, bytes.0.clone(), 0.into())
                .expect("bad call to evm");

            logs.extend(call_result.logs);
            traces.push((TraceKind::Execution, call_result.traces.clone().unwrap()));

            // Identify newly generated contracts, if they exist.
            ided_contracts.extend(load_contracts(
                vec![(TraceKind::Execution, call_result.traces.unwrap())],
                known_contracts,
            ));

            counterexample_sequence.push(BaseCounterExample::create(
                *sender,
                *addr,
                bytes,
                &ided_contracts,
            ));

            // Checks the invariant.
            if let Some(func) = &self.func {
                let error_call_result = executor
                    .call_raw(CALLER, self.addr, func.0.clone(), 0.into())
                    .expect("bad call to evm");

                if error_call_result.reverted {
                    logs.extend(error_call_result.logs);
                    traces.push((TraceKind::Execution, error_call_result.traces.unwrap()));
                    break
                }
            }
        }

        (!counterexample_sequence.is_empty())
            .then_some(CounterExample::Sequence(counterexample_sequence))
    }
}
