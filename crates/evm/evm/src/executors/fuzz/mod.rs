use crate::executors::{Executor, RawCallResult};
use alloy_dyn_abi::JsonAbiExt;
use alloy_json_abi::Function;
use alloy_primitives::{Address, Bytes, Log, U256};
use eyre::Result;
use foundry_common::evm::Breakpoints;
use foundry_config::FuzzConfig;
use foundry_evm_core::{constants::MAGIC_ASSUME, decode::RevertDecoder};
use foundry_evm_coverage::HitMaps;
use foundry_evm_fuzz::{
    strategies::{fuzz_calldata, fuzz_calldata_from_state, EvmFuzzState},
    BaseCounterExample, CounterExample, FuzzCase, FuzzError, FuzzFixtures, FuzzTestResult,
};
use foundry_evm_traces::SparsedTraceArena;
use indicatif::ProgressBar;
use proptest::test_runner::{TestCaseError, TestError, TestRunner};
use std::cell::RefCell;

mod types;
pub use types::{CaseOutcome, CounterExampleOutcome, FuzzOutcome};

/// Contains data collected during fuzz test runs.
#[derive(Default)]
pub struct FuzzTestData {
    // Stores the first fuzz case.
    pub first_case: Option<FuzzCase>,
    // Stored gas usage per fuzz case.
    pub gas_by_case: Vec<(u64, u64)>,
    // Stores the result and calldata of the last failed call, if any.
    pub counterexample: (Bytes, RawCallResult),
    // Stores up to `max_traces_to_collect` traces.
    pub traces: Vec<SparsedTraceArena>,
    // Stores breakpoints for the last fuzz case.
    pub breakpoints: Option<Breakpoints>,
    // Stores coverage information for all fuzz cases.
    pub coverage: Option<HitMaps>,
    // Stores logs for all fuzz cases
    pub logs: Vec<Log>,
}

/// Wrapper around an [`Executor`] which provides fuzzing support using [`proptest`].
///
/// After instantiation, calling `fuzz` will proceed to hammer the deployed smart contract with
/// inputs, until it finds a counterexample. The provided [`TestRunner`] contains all the
/// configuration which can be overridden via [environment variables](proptest::test_runner::Config)
pub struct FuzzedExecutor {
    /// The EVM executor
    executor: Executor,
    /// The fuzzer
    runner: TestRunner,
    /// The account that calls tests
    sender: Address,
    /// The fuzz configuration
    config: FuzzConfig,
}

impl FuzzedExecutor {
    /// Instantiates a fuzzed executor given a testrunner
    pub fn new(
        executor: Executor,
        runner: TestRunner,
        sender: Address,
        config: FuzzConfig,
    ) -> Self {
        Self { executor, runner, sender, config }
    }

    /// Fuzzes the provided function, assuming it is available at the contract at `address`
    /// If `should_fail` is set to `true`, then it will stop only when there's a success
    /// test case.
    ///
    /// Returns a list of all the consumed gas and calldata of every fuzz case
    pub fn fuzz(
        &self,
        func: &Function,
        fuzz_fixtures: &FuzzFixtures,
        address: Address,
        should_fail: bool,
        rd: &RevertDecoder,
        progress: Option<&ProgressBar>,
    ) -> FuzzTestResult {
        // Stores the fuzz test execution data.
        let execution_data = RefCell::new(FuzzTestData::default());
        let state = self.build_fuzz_state();
        let dictionary_weight = self.config.dictionary.dictionary_weight.min(100);
        let strat = proptest::prop_oneof![
            100 - dictionary_weight => fuzz_calldata(func.clone(), fuzz_fixtures),
            dictionary_weight => fuzz_calldata_from_state(func.clone(), &state),
        ];
        // We want to collect at least one trace which will be displayed to user.
        let max_traces_to_collect = std::cmp::max(1, self.config.gas_report_samples) as usize;
        let show_logs = self.config.show_logs;

        let run_result = self.runner.clone().run(&strat, |calldata| {
            let fuzz_res = self.single_fuzz(address, should_fail, calldata)?;

            // If running with progress then increment current run.
            if let Some(progress) = progress {
                progress.inc(1);
            };

            match fuzz_res {
                FuzzOutcome::Case(case) => {
                    let mut data = execution_data.borrow_mut();
                    data.gas_by_case.push((case.case.gas, case.case.stipend));
                    if data.first_case.is_none() {
                        data.first_case.replace(case.case);
                    }
                    if let Some(call_traces) = case.traces {
                        if data.traces.len() == max_traces_to_collect {
                            data.traces.pop();
                        }
                        data.traces.push(call_traces);
                        data.breakpoints.replace(case.breakpoints);
                    }
                    if show_logs {
                        data.logs.extend(case.logs);
                    }
                    // Collect and merge coverage if `forge snapshot` context.
                    match &mut data.coverage {
                        Some(prev) => prev.merge(case.coverage.unwrap()),
                        opt => *opt = case.coverage,
                    }

                    Ok(())
                }
                FuzzOutcome::CounterExample(CounterExampleOutcome {
                    exit_reason: status,
                    counterexample: outcome,
                    ..
                }) => {
                    // We cannot use the calldata returned by the test runner in `TestError::Fail`,
                    // since that input represents the last run case, which may not correspond with
                    // our failure - when a fuzz case fails, proptest will try
                    // to run at least one more case to find a minimal failure
                    // case.
                    let reason = rd.maybe_decode(&outcome.1.result, Some(status));
                    execution_data.borrow_mut().logs.extend(outcome.1.logs.clone());
                    execution_data.borrow_mut().counterexample = outcome;
                    // HACK: we have to use an empty string here to denote `None`.
                    Err(TestCaseError::fail(reason.unwrap_or_default()))
                }
            }
        });

        let fuzz_result = execution_data.into_inner();
        let (calldata, call) = fuzz_result.counterexample;

        let mut traces = fuzz_result.traces;
        let (last_run_traces, last_run_breakpoints) = if run_result.is_ok() {
            (traces.pop(), fuzz_result.breakpoints)
        } else {
            (call.traces.clone(), call.cheatcodes.map(|c| c.breakpoints))
        };

        let mut result = FuzzTestResult {
            first_case: fuzz_result.first_case.unwrap_or_default(),
            gas_by_case: fuzz_result.gas_by_case,
            success: run_result.is_ok(),
            reason: None,
            counterexample: None,
            logs: fuzz_result.logs,
            labeled_addresses: call.labels,
            traces: last_run_traces,
            breakpoints: last_run_breakpoints,
            gas_report_traces: traces.into_iter().map(|a| a.arena).collect(),
            coverage: fuzz_result.coverage,
        };

        match run_result {
            // Currently the only operation that can trigger proptest global rejects is the
            // `vm.assume` cheatcode, thus we surface this info to the user when the fuzz test
            // aborts due to too many global rejects, making the error message more actionable.
            Err(TestError::Abort(reason)) if reason.message() == "Too many global rejects" => {
                result.reason = Some(
                    FuzzError::TooManyRejects(self.runner.config().max_global_rejects).to_string(),
                );
            }
            Err(TestError::Abort(reason)) => {
                result.reason = Some(reason.to_string());
            }
            Err(TestError::Fail(reason, _)) => {
                let reason = reason.to_string();
                result.reason = if reason.is_empty() { None } else { Some(reason) };

                let args = if let Some(data) = calldata.get(4..) {
                    func.abi_decode_input(data, false).unwrap_or_default()
                } else {
                    vec![]
                };

                result.counterexample = Some(CounterExample::Single(
                    BaseCounterExample::from_fuzz_call(calldata, args, call.traces),
                ));
            }
            _ => {}
        }

        state.log_stats();

        result
    }

    /// Granular and single-step function that runs only one fuzz and returns either a `CaseOutcome`
    /// or a `CounterExampleOutcome`
    pub fn single_fuzz(
        &self,
        address: Address,
        should_fail: bool,
        calldata: alloy_primitives::Bytes,
    ) -> Result<FuzzOutcome, TestCaseError> {
        let mut call = self
            .executor
            .call_raw(self.sender, address, calldata.clone(), U256::ZERO)
            .map_err(|_| TestCaseError::fail(FuzzError::FailedContractCall))?;

        // When the `assume` cheatcode is called it returns a special string
        if call.result.as_ref() == MAGIC_ASSUME {
            return Err(TestCaseError::reject(FuzzError::AssumeReject))
        }

        let breakpoints = call
            .cheatcodes
            .as_ref()
            .map_or_else(Default::default, |cheats| cheats.breakpoints.clone());

        let success = self.executor.is_raw_call_mut_success(address, &mut call, should_fail);
        if success {
            Ok(FuzzOutcome::Case(CaseOutcome {
                case: FuzzCase { calldata, gas: call.gas_used, stipend: call.stipend },
                traces: call.traces,
                coverage: call.coverage,
                breakpoints,
                logs: call.logs,
            }))
        } else {
            Ok(FuzzOutcome::CounterExample(CounterExampleOutcome {
                exit_reason: call.exit_reason,
                counterexample: (calldata, call),
                breakpoints,
            }))
        }
    }

    /// Stores fuzz state for use with [fuzz_calldata_from_state]
    pub fn build_fuzz_state(&self) -> EvmFuzzState {
        if let Some(fork_db) = self.executor.backend().active_fork_db() {
            EvmFuzzState::new(fork_db, self.config.dictionary)
        } else {
            EvmFuzzState::new(self.executor.backend().mem_db(), self.config.dictionary)
        }
    }
}
