use super::{BasicTxDetails, InvariantContract};
use crate::executors::{invariant::shrink::CallSequenceShrinker, Executor, RawCallResult};
use alloy_json_abi::Function;
use alloy_primitives::{Address, Bytes, Log};
use eyre::Result;
use foundry_common::contracts::{ContractsByAddress, ContractsByArtifact};
use foundry_evm_core::{constants::CALLER, decode::RevertDecoder};
use foundry_evm_fuzz::{
    invariant::FuzzRunIdentifiedContracts, BaseCounterExample, CounterExample, FuzzedCases, Reason,
};
use foundry_evm_traces::{load_contracts, CallTraceArena, TraceKind, Traces};
use parking_lot::RwLock;
use proptest::test_runner::TestError;
use revm::primitives::U256;
use std::{borrow::Cow, sync::Arc};

/// Stores information about failures and reverts of the invariant tests.
#[derive(Clone, Default)]
pub struct InvariantFailures {
    /// Total number of reverts.
    pub reverts: usize,
    /// How many different invariants have been broken.
    pub broken_invariants_count: usize,
    /// The latest revert reason of a run.
    pub revert_reason: Option<String>,
    /// Maps a broken invariant to its specific error.
    pub error: Option<InvariantFuzzError>,
}

impl InvariantFailures {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn into_inner(self) -> (usize, Option<InvariantFuzzError>) {
        (self.reverts, self.error)
    }
}

/// The outcome of an invariant fuzz test
#[derive(Debug)]
pub struct InvariantFuzzTestResult {
    pub error: Option<InvariantFuzzError>,
    /// Every successful fuzz test case
    pub cases: Vec<FuzzedCases>,
    /// Number of reverted fuzz calls
    pub reverts: usize,

    /// The entire inputs of the last run of the invariant campaign, used for
    /// replaying the run for collecting traces.
    pub last_run_inputs: Vec<BasicTxDetails>,

    /// Additional traces used for gas report construction.
    pub gas_report_traces: Vec<Vec<CallTraceArena>>,
}

#[derive(Clone, Debug)]
pub enum InvariantFuzzError {
    Revert(FailedInvariantCaseData),
    BrokenInvariant(FailedInvariantCaseData),
    MaxAssumeRejects(u32),
}

impl InvariantFuzzError {
    pub fn revert_reason(&self) -> Option<String> {
        match self {
            Self::BrokenInvariant(case_data) | Self::Revert(case_data) => {
                (!case_data.revert_reason.is_empty()).then(|| case_data.revert_reason.clone())
            }
            Self::MaxAssumeRejects(allowed) => Some(format!(
                "The `vm.assume` cheatcode rejected too many inputs ({allowed} allowed)"
            )),
        }
    }
}

#[derive(Clone, Debug)]
pub struct FailedInvariantCaseData {
    pub logs: Vec<Log>,
    pub traces: Option<CallTraceArena>,
    /// The proptest error occurred as a result of a test case.
    pub test_error: TestError<Vec<BasicTxDetails>>,
    /// The return reason of the offending call.
    pub return_reason: Reason,
    /// The revert string of the offending call.
    pub revert_reason: String,
    /// Address of the invariant asserter.
    pub addr: Address,
    /// Function data for invariant check.
    pub func: Option<Bytes>,
    /// Inner fuzzing Sequence coming from overriding calls.
    pub inner_sequence: Vec<Option<BasicTxDetails>>,
    /// Shrink the failed test case to the smallest sequence.
    pub shrink: bool,
    /// Shrink run limit
    pub shrink_run_limit: usize,
}

impl FailedInvariantCaseData {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        invariant_contract: &InvariantContract<'_>,
        targeted_contracts: &FuzzRunIdentifiedContracts,
        error_func: Option<&Function>,
        calldata: &[BasicTxDetails],
        call_result: RawCallResult,
        inner_sequence: &[Option<BasicTxDetails>],
        shrink: bool,
        shrink_run_limit: usize,
    ) -> Self {
        let (func, origin) = if let Some(f) = error_func {
            (Some(f.selector().to_vec().into()), f.name.as_str())
        } else {
            (None, "Revert")
        };

        // Collect abis of fuzzed and invariant contracts to decode custom error.
        let targets = targeted_contracts.targets.lock();
        let abis = targets
            .iter()
            .map(|contract| &contract.1 .1)
            .chain(std::iter::once(invariant_contract.abi));

        let revert_reason = RevertDecoder::new()
            .with_abis(abis)
            .decode(call_result.result.as_ref(), Some(call_result.exit_reason));

        Self {
            logs: call_result.logs,
            traces: call_result.traces,
            test_error: proptest::test_runner::TestError::Fail(
                format!("{origin}, reason: {revert_reason}").into(),
                calldata.to_vec(),
            ),
            return_reason: "".into(),
            revert_reason,
            addr: invariant_contract.address,
            func,
            inner_sequence: inner_sequence.to_vec(),
            shrink,
            shrink_run_limit,
        }
    }

    /// Replays the error case and collects all necessary traces.
    pub fn replay(
        &self,
        mut executor: Executor,
        known_contracts: &ContractsByArtifact,
        mut ided_contracts: ContractsByAddress,
        logs: &mut Vec<Log>,
        traces: &mut Traces,
    ) -> Result<Option<CounterExample>> {
        let mut counterexample_sequence = vec![];
        let mut calls = match self.test_error {
            // Don't use at the moment.
            TestError::Abort(_) => return Ok(None),
            TestError::Fail(_, ref calls) => calls.clone(),
        };

        if self.shrink {
            calls = self.shrink_sequence(&calls, &executor)?.into_iter().cloned().collect();
        } else {
            trace!(target: "forge::test", "Shrinking disabled.");
        }

        // We want traces for a failed case.
        executor.set_tracing(true);

        set_up_inner_replay(&mut executor, &self.inner_sequence);

        // Replay each call from the sequence until we break the invariant.
        for (sender, (addr, bytes)) in calls.iter() {
            let call_result =
                executor.call_raw_committing(*sender, *addr, bytes.clone(), U256::ZERO)?;

            logs.extend(call_result.logs);
            traces.push((TraceKind::Execution, call_result.traces.clone().unwrap()));

            // Identify newly generated contracts, if they exist.
            ided_contracts.extend(load_contracts(
                vec![(TraceKind::Execution, call_result.traces.clone().unwrap())],
                known_contracts,
            ));

            counterexample_sequence.push(BaseCounterExample::create(
                *sender,
                *addr,
                bytes,
                &ided_contracts,
                call_result.traces,
            ));

            // Checks the invariant.
            if let Some(func) = &self.func {
                let error_call_result =
                    executor.call_raw(CALLER, self.addr, func.clone(), U256::ZERO)?;

                traces.push((TraceKind::Execution, error_call_result.traces.clone().unwrap()));

                logs.extend(error_call_result.logs);
                if error_call_result.reverted {
                    break
                }
            }
        }

        Ok((!counterexample_sequence.is_empty())
            .then_some(CounterExample::Sequence(counterexample_sequence)))
    }

    /// Tries to shrink the failure case to its smallest sequence of calls.
    ///
    /// If the number of calls is small enough, we can guarantee maximal shrinkage
    fn shrink_sequence<'a>(
        &self,
        calls: &'a [BasicTxDetails],
        executor: &Executor,
    ) -> Result<Vec<&'a BasicTxDetails>> {
        trace!(target: "forge::test", "Shrinking.");

        // Special case test: the invariant is *unsatisfiable* - it took 0 calls to
        // break the invariant -- consider emitting a warning.
        if let Some(func) = &self.func {
            let error_call_result =
                executor.call_raw(CALLER, self.addr, func.clone(), U256::ZERO)?;
            if error_call_result.reverted {
                return Ok(vec![]);
            }
        }

        let mut shrinker = CallSequenceShrinker::new((0..calls.len()).collect());
        let mut shrink_attempts = 0;
        loop {
            // Exit if we reached the configured max limit shrink runs.
            if shrink_attempts > self.shrink_run_limit {
                break
            }

            // Check candidate sequence result.
            match self.check_sequence(executor.clone(), calls, shrinker.current()) {
                false => {
                    // If candidate sequence still fails then shrink more if possible.
                    if !shrinker.simplify() {
                        break
                    }
                }
                true => {
                    // If candidate sequence pass then restore last removed call and shrink other
                    // calls if possible.
                    if !shrinker.complicate() {
                        break
                    }
                }
            }

            shrink_attempts += 1;
        }

        // We recreate the call sequence in the same order as they reproduce the failure,
        // otherwise we could end up with inverted sequence.
        // E.g. in a sequence of:
        // 1. Alice calls acceptOwnership and reverts
        // 2. Bob calls transferOwnership to Alice
        // 3. Alice calls acceptOwnership and test fails
        // we shrink to indices of [2, 1] and we recreate call sequence in same order.
        Ok(shrinker.current().iter().map(|idx| &calls[*idx]).collect())
    }

    fn check_sequence(
        &self,
        mut executor: Executor,
        calls: &[BasicTxDetails],
        sequence: Vec<usize>,
    ) -> bool {
        // Apply the shrinked candidate sequence.
        sequence.iter().for_each(|call_index| {
            let (sender, (addr, bytes)) = &calls[*call_index];
            executor.call_raw_committing(*sender, *addr, bytes.clone(), U256::ZERO).unwrap();
        });

        // Check the invariant for candidate sequence.
        // If sequence fails then we can continue with shrinking - the removed call does not affect
        // failure.
        //
        // If sequence doesn't fail then we have to restore last removed call and continue with next
        // call - removed call is a required step for reproducing the failure.
        if let Some(func) = &self.func {
            let mut call_result =
                executor.call_raw(CALLER, self.addr, func.clone(), U256::ZERO).unwrap();
            executor.is_raw_call_success(
                self.addr,
                Cow::Owned(call_result.state_changeset.take().unwrap()),
                &call_result,
                false,
            )
        } else {
            false
        }
    }
}

/// Sets up the calls generated by the internal fuzzer, if they exist.
fn set_up_inner_replay(executor: &mut Executor, inner_sequence: &[Option<BasicTxDetails>]) {
    if let Some(fuzzer) = &mut executor.inspector.fuzzer {
        if let Some(call_generator) = &mut fuzzer.call_generator {
            call_generator.last_sequence = Arc::new(RwLock::new(inner_sequence.to_owned()));
            call_generator.set_replay(true);
        }
    }
}
