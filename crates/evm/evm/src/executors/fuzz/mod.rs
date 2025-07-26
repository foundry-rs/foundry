use crate::executors::{
    COVERAGE_MAP_SIZE, DURATION_BETWEEN_METRICS_REPORT, Executor, FuzzTestTimer, RawCallResult,
};
use alloy_dyn_abi::JsonAbiExt;
use alloy_json_abi::Function;
use alloy_primitives::{Address, Bytes, Log, U256, map::HashMap};
use eyre::Result;
use foundry_common::{evm::Breakpoints, sh_println};
use foundry_config::FuzzConfig;
use foundry_evm_core::{
    constants::{CHEATCODE_ADDRESS, MAGIC_ASSUME},
    decode::{RevertDecoder, SkipReason},
};
use foundry_evm_coverage::HitMaps;
use foundry_evm_fuzz::{
    BaseCounterExample, CounterExample, FuzzCase, FuzzError, FuzzFixtures, FuzzTestResult,
    strategies::{EvmFuzzState, fuzz_calldata, fuzz_calldata_from_state},
};
use foundry_evm_traces::SparsedTraceArena;
use indicatif::ProgressBar;
use proptest::{
    strategy::{Strategy, ValueTree},
    test_runner::{TestCaseError, TestRunner},
};
use serde::Serialize;
use serde_json::json;
use std::{
    fmt,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

mod types;
pub use types::{CaseOutcome, CounterExampleOutcome, FuzzOutcome};

#[derive(Serialize, Default)]
struct FuzzCoverageMetrics {
    // Number of edges seen during the fuzz test.
    cumulative_edges_seen: usize,
    // Number of features (new hitcount bin of previously hit edge) seen during the fuzz test.
    cumulative_features_seen: usize,
}

impl fmt::Display for FuzzCoverageMetrics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f)?;
        writeln!(f, "        - cumulative edges seen: {}", self.cumulative_edges_seen)?;
        writeln!(f, "        - cumulative features seen: {}", self.cumulative_features_seen)?;
        Ok(())
    }
}

impl FuzzCoverageMetrics {
    /// Records number of new edges or features explored during the campaign.
    pub fn update_seen(&mut self, is_edge: bool) {
        if is_edge {
            self.cumulative_edges_seen += 1;
        } else {
            self.cumulative_features_seen += 1;
        }
    }
}

/// Contains data collected during fuzz test runs.
#[derive(Default)]
struct FuzzTestData {
    // Stores the first fuzz case.
    first_case: Option<FuzzCase>,
    // Stored gas usage per fuzz case.
    gas_by_case: Vec<(u64, u64)>,
    // Stores the result and calldata of the last failed call, if any.
    counterexample: (Bytes, RawCallResult),
    // Stores up to `max_traces_to_collect` traces.
    traces: Vec<SparsedTraceArena>,
    // Stores breakpoints for the last fuzz case.
    breakpoints: Option<Breakpoints>,
    // Stores coverage information for all fuzz cases.
    coverage: Option<HitMaps>,
    // Stores logs for all fuzz cases
    logs: Vec<Log>,
    // Deprecated cheatcodes mapped to their replacements.
    deprecated_cheatcodes: HashMap<&'static str, Option<&'static str>>,
    // Coverage metrics collected during the fuzz test.
    coverage_metrics: FuzzCoverageMetrics,
    // Runs performed in fuzz test.
    runs: u32,
    // Current assume rejects of the fuzz run.
    rejects: u32,
    // Test failure.
    failure: Option<TestCaseError>,
}

/// Wrapper around an [`Executor`] which provides fuzzing support using [`proptest`].
///
/// After instantiation, calling `fuzz` will proceed to hammer the deployed smart contract with
/// inputs, until it finds a counterexample. The provided [`TestRunner`] contains all the
/// configuration which can be overridden via [environment variables](proptest::test_runner::Config)
pub struct FuzzedExecutor {
    /// The EVM executor.
    executor: Executor,
    /// The fuzzer
    runner: TestRunner,
    /// The account that calls tests.
    sender: Address,
    /// The fuzz configuration.
    config: FuzzConfig,
    /// The persisted counterexample to be replayed, if any.
    persisted_failure: Option<BaseCounterExample>,
    /// History of binned hitcount of edges seen during fuzzing.
    history_map: Vec<u8>,
}

impl FuzzedExecutor {
    /// Instantiates a fuzzed executor given a testrunner
    pub fn new(
        executor: Executor,
        runner: TestRunner,
        sender: Address,
        config: FuzzConfig,
        persisted_failure: Option<BaseCounterExample>,
    ) -> Self {
        Self {
            executor,
            runner,
            sender,
            config,
            persisted_failure,
            history_map: vec![0u8; COVERAGE_MAP_SIZE],
        }
    }

    /// Fuzzes the provided function, assuming it is available at the contract at `address`
    /// If `should_fail` is set to `true`, then it will stop only when there's a success
    /// test case.
    ///
    /// Returns a list of all the consumed gas and calldata of every fuzz case.
    pub fn fuzz(
        &mut self,
        func: &Function,
        fuzz_fixtures: &FuzzFixtures,
        deployed_libs: &[Address],
        address: Address,
        rd: &RevertDecoder,
        progress: Option<&ProgressBar>,
    ) -> FuzzTestResult {
        // Stores the fuzz test execution data.
        let mut test_data = FuzzTestData::default();
        let state = self.build_fuzz_state(deployed_libs);
        let dictionary_weight = self.config.dictionary.dictionary_weight.min(100);
        let strategy = proptest::prop_oneof![
            100 - dictionary_weight => fuzz_calldata(func.clone(), fuzz_fixtures),
            dictionary_weight => fuzz_calldata_from_state(func.clone(), &state),
        ];
        // We want to collect at least one trace which will be displayed to user.
        let max_traces_to_collect = std::cmp::max(1, self.config.gas_report_samples) as usize;

        // Start timer for this fuzz test.
        let timer = FuzzTestTimer::new(self.config.timeout);
        let mut last_metrics_report = Instant::now();
        let max_runs = self.config.runs;
        let continue_campaign = |runs: u32| {
            if timer.is_enabled() { !timer.is_timed_out() } else { runs < max_runs }
        };

        'stop: while continue_campaign(test_data.runs) {
            // If counterexample recorded, replay it first, without incrementing runs.
            let input = if let Some(failure) = self.persisted_failure.take() {
                failure.calldata
            } else {
                // If running with progress, then increment current run.
                if let Some(progress) = progress {
                    progress.inc(1);
                    // Display metrics in progress bar.
                    if self.config.show_edge_coverage {
                        progress.set_message(format!("{}", &test_data.coverage_metrics));
                    }
                } else if self.config.show_edge_coverage
                    && last_metrics_report.elapsed() > DURATION_BETWEEN_METRICS_REPORT
                {
                    // Display metrics inline.
                    let metrics = json!({
                        "timestamp": SystemTime::now()
                            .duration_since(UNIX_EPOCH).unwrap()
                            .as_secs(),
                        "test": func.name,
                        "metrics": &test_data.coverage_metrics,
                    });
                    let _ = sh_println!("{}", serde_json::to_string(&metrics).unwrap());
                    last_metrics_report = Instant::now();
                };

                test_data.runs += 1;

                let Ok(strategy) = strategy.new_tree(&mut self.runner) else {
                    test_data.failure =
                        Some(TestCaseError::fail("no input generated to call fuzzed target"));
                    break 'stop;
                };
                strategy.current()
            };

            match self.single_fuzz(address, input, &mut test_data.coverage_metrics) {
                Ok(fuzz_outcome) => match fuzz_outcome {
                    FuzzOutcome::Case(case) => {
                        test_data.gas_by_case.push((case.case.gas, case.case.stipend));

                        if test_data.first_case.is_none() {
                            test_data.first_case.replace(case.case);
                        }

                        if let Some(call_traces) = case.traces {
                            if test_data.traces.len() == max_traces_to_collect {
                                test_data.traces.pop();
                            }
                            test_data.traces.push(call_traces);
                            test_data.breakpoints.replace(case.breakpoints);
                        }

                        if self.config.show_logs {
                            test_data.logs.extend(case.logs);
                        }

                        HitMaps::merge_opt(&mut test_data.coverage, case.coverage);
                        test_data.deprecated_cheatcodes = case.deprecated_cheatcodes;
                    }
                    FuzzOutcome::CounterExample(CounterExampleOutcome {
                        exit_reason: status,
                        counterexample: outcome,
                        ..
                    }) => {
                        let reason = rd.maybe_decode(&outcome.1.result, status);
                        test_data.logs.extend(outcome.1.logs.clone());
                        test_data.counterexample = outcome;
                        test_data.failure = Some(TestCaseError::fail(reason.unwrap_or_default()));
                        break 'stop;
                    }
                },
                Err(err) => {
                    match err {
                        TestCaseError::Fail(_) => {
                            test_data.failure = Some(err);
                            break 'stop;
                        }
                        TestCaseError::Reject(_) => {
                            // Apply max rejects only if configured, otherwise silently discard run.
                            if self.config.max_test_rejects > 0 {
                                test_data.rejects += 1;
                                if test_data.rejects >= self.config.max_test_rejects {
                                    test_data.failure = Some(err);
                                    break 'stop;
                                }
                            }
                        }
                    }
                }
            }
        }

        let (calldata, call) = test_data.counterexample;
        let mut traces = test_data.traces;
        let (last_run_traces, last_run_breakpoints) = if test_data.failure.is_none() {
            (traces.pop(), test_data.breakpoints)
        } else {
            (call.traces.clone(), call.cheatcodes.map(|c| c.breakpoints))
        };

        let mut result = FuzzTestResult {
            first_case: test_data.first_case.unwrap_or_default(),
            gas_by_case: test_data.gas_by_case,
            success: test_data.failure.is_none(),
            skipped: false,
            reason: None,
            counterexample: None,
            logs: test_data.logs,
            labeled_addresses: call.labels,
            traces: last_run_traces,
            breakpoints: last_run_breakpoints,
            gas_report_traces: traces.into_iter().map(|a| a.arena).collect(),
            line_coverage: test_data.coverage,
            deprecated_cheatcodes: test_data.deprecated_cheatcodes,
        };

        match test_data.failure {
            Some(TestCaseError::Fail(reason)) => {
                let reason = reason.to_string();
                result.reason = (!reason.is_empty()).then_some(reason);
                let args = if let Some(data) = calldata.get(4..) {
                    func.abi_decode_input(data).unwrap_or_default()
                } else {
                    vec![]
                };
                result.counterexample = Some(CounterExample::Single(
                    BaseCounterExample::from_fuzz_call(calldata, args, call.traces),
                ));
            }
            Some(TestCaseError::Reject(reason)) => {
                let reason = reason.to_string();
                result.reason = (!reason.is_empty()).then_some(reason);
            }
            None => {}
        }

        if let Some(reason) = &result.reason
            && let Some(reason) = SkipReason::decode_self(reason)
        {
            result.skipped = true;
            result.reason = reason.0;
        }

        state.log_stats();

        result
    }

    /// Granular and single-step function that runs only one fuzz and returns either a `CaseOutcome`
    /// or a `CounterExampleOutcome`
    fn single_fuzz(
        &mut self,
        address: Address,
        calldata: Bytes,
        coverage_metrics: &mut FuzzCoverageMetrics,
    ) -> Result<FuzzOutcome, TestCaseError> {
        let mut call = self
            .executor
            .call_raw(self.sender, address, calldata.clone(), U256::ZERO)
            .map_err(|e| TestCaseError::fail(e.to_string()))?;

        if self.config.show_edge_coverage {
            let (new_coverage, is_edge) = call.merge_edge_coverage(&mut self.history_map);
            if new_coverage {
                coverage_metrics.update_seen(is_edge);
            }
        }

        // Handle `vm.assume`.
        if call.result.as_ref() == MAGIC_ASSUME {
            return Err(TestCaseError::reject(FuzzError::TooManyRejects(
                self.config.max_test_rejects,
            )));
        }

        let (breakpoints, deprecated_cheatcodes) =
            call.cheatcodes.as_ref().map_or_else(Default::default, |cheats| {
                (cheats.breakpoints.clone(), cheats.deprecated.clone())
            });

        // Consider call success if test should not fail on reverts and reverter is not the
        // cheatcode or test address.
        let success = if !self.config.fail_on_revert
            && call
                .reverter
                .is_some_and(|reverter| reverter != address && reverter != CHEATCODE_ADDRESS)
        {
            true
        } else {
            self.executor.is_raw_call_mut_success(address, &mut call, false)
        };

        if success {
            Ok(FuzzOutcome::Case(CaseOutcome {
                case: FuzzCase { calldata, gas: call.gas_used, stipend: call.stipend },
                traces: call.traces,
                coverage: call.line_coverage,
                breakpoints,
                logs: call.logs,
                deprecated_cheatcodes,
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
    pub fn build_fuzz_state(&self, deployed_libs: &[Address]) -> EvmFuzzState {
        if let Some(fork_db) = self.executor.backend().active_fork_db() {
            EvmFuzzState::new(fork_db, self.config.dictionary, deployed_libs)
        } else {
            EvmFuzzState::new(
                self.executor.backend().mem_db(),
                self.config.dictionary,
                deployed_libs,
            )
        }
    }
}
