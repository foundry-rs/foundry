use crate::executors::{
    DURATION_BETWEEN_METRICS_REPORT, EarlyExit, Executor, FuzzTestTimer, RawCallResult,
};
use alloy_dyn_abi::JsonAbiExt;
use alloy_json_abi::Function;
use alloy_primitives::{Address, Bytes, Log, U256, map::HashMap};
use eyre::Result;
use foundry_common::sh_println;
use foundry_config::FuzzConfig;
use foundry_evm_core::{
    Breakpoints,
    constants::{CHEATCODE_ADDRESS, MAGIC_ASSUME},
    decode::{RevertDecoder, SkipReason},
};
use foundry_evm_coverage::HitMaps;
use foundry_evm_fuzz::{
    BaseCounterExample, BasicTxDetails, CallDetails, CounterExample, FuzzCase, FuzzError,
    FuzzFixtures, FuzzTestResult,
    strategies::{EvmFuzzState, fuzz_calldata, fuzz_calldata_from_state},
};
use foundry_evm_traces::SparsedTraceArena;
use indicatif::ProgressBar;
use proptest::{
    strategy::Strategy,
    test_runner::{TestCaseError, TestRunner},
};
use serde_json::json;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

mod types;
use crate::executors::corpus::CorpusManager;
pub use types::{CaseOutcome, CounterExampleOutcome, FuzzOutcome};

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
    // Stores logs for all fuzz cases (when show_logs is true) or just the last run (when show_logs
    // is false)
    logs: Vec<Log>,
    // Deprecated cheatcodes mapped to their replacements.
    deprecated_cheatcodes: HashMap<&'static str, Option<&'static str>>,
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
        Self { executor, runner, sender, config, persisted_failure }
    }

    /// Fuzzes the provided function, assuming it is available at the contract at `address`
    /// If `should_fail` is set to `true`, then it will stop only when there's a success
    /// test case.
    ///
    /// Returns a list of all the consumed gas and calldata of every fuzz case.
    #[allow(clippy::too_many_arguments)]
    pub fn fuzz(
        &mut self,
        func: &Function,
        fuzz_fixtures: &FuzzFixtures,
        state: EvmFuzzState,
        address: Address,
        rd: &RevertDecoder,
        progress: Option<&ProgressBar>,
        early_exit: &EarlyExit,
    ) -> Result<FuzzTestResult> {
        let state = &state;
        // Stores the fuzz test execution data.
        let mut test_data = FuzzTestData::default();
        let dictionary_weight = self.config.dictionary.dictionary_weight.min(100);
        let strategy = proptest::prop_oneof![
            100 - dictionary_weight => fuzz_calldata(func.clone(), fuzz_fixtures),
            dictionary_weight => fuzz_calldata_from_state(func.clone(), state),
        ]
        .prop_map(move |calldata| BasicTxDetails {
            warp: None,
            roll: None,
            sender: Default::default(),
            call_details: CallDetails { target: Default::default(), calldata },
        });
        // We want to collect at least one trace which will be displayed to user.
        let max_traces_to_collect = std::cmp::max(1, self.config.gas_report_samples) as usize;

        let mut corpus_manager = CorpusManager::new(
            self.config.corpus.clone(),
            strategy.boxed(),
            &self.executor,
            Some(func),
            None,
        )?;

        // Start timer for this fuzz test.
        let timer = FuzzTestTimer::new(self.config.timeout);
        let mut last_metrics_report = Instant::now();
        let max_runs = self.config.runs;
        let continue_campaign = |runs: u32| {
            if early_exit.should_stop() {
                return false;
            }

            if timer.is_enabled() { !timer.is_timed_out() } else { runs < max_runs }
        };

        'stop: while continue_campaign(test_data.runs) {
            // If counterexample recorded, replay it first, without incrementing runs.
            let input = if let Some(failure) = self.persisted_failure.take()
                && failure.calldata.get(..4).is_some_and(|selector| func.selector() == selector)
            {
                failure.calldata.clone()
            } else {
                // If running with progress, then increment current run.
                if let Some(progress) = progress {
                    progress.inc(1);
                    // Display metrics in progress bar.
                    if self.config.corpus.collect_edge_coverage() {
                        progress.set_message(format!("{}", &corpus_manager.metrics));
                    }
                } else if self.config.corpus.collect_edge_coverage()
                    && last_metrics_report.elapsed() > DURATION_BETWEEN_METRICS_REPORT
                {
                    // Display metrics inline.
                    let metrics = json!({
                        "timestamp": SystemTime::now()
                            .duration_since(UNIX_EPOCH)?
                            .as_secs(),
                        "test": func.name,
                        "metrics": &corpus_manager.metrics,
                    });
                    let _ = sh_println!("{}", serde_json::to_string(&metrics)?);
                    last_metrics_report = Instant::now();
                };

                if let Some(cheats) = self.executor.inspector_mut().cheatcodes.as_mut()
                    && let Some(seed) = self.config.seed
                {
                    cheats.set_seed(seed.wrapping_add(U256::from(test_data.runs)));
                }
                test_data.runs += 1;

                match corpus_manager.new_input(&mut self.runner, state, func) {
                    Ok(input) => input,
                    Err(err) => {
                        test_data.failure = Some(TestCaseError::fail(format!(
                            "failed to generate fuzzed input: {err}"
                        )));
                        break 'stop;
                    }
                }
            };

            match self.single_fuzz(address, input, &mut corpus_manager) {
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

                        // Always store logs from the last run in test_data.logs for display at
                        // verbosity >= 2. When show_logs is true,
                        // accumulate all logs. When false, only keep the last run's logs.
                        if self.config.show_logs {
                            test_data.logs.extend(case.logs);
                        } else {
                            test_data.logs = case.logs;
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
                            // Discard run and apply max rejects if configured. Saturate to handle
                            // the case of replayed failure, which doesn't count as a run.
                            test_data.runs = test_data.runs.saturating_sub(1);
                            test_data.rejects += 1;

                            // Update progress bar to reflect rejected runs.
                            if let Some(progress) = progress {
                                progress.set_message(format!("([{}] rejected)", test_data.rejects));
                                progress.dec(1);
                            }

                            if self.config.max_test_rejects > 0
                                && test_data.rejects >= self.config.max_test_rejects
                            {
                                test_data.failure = Some(TestCaseError::reject(
                                    FuzzError::TooManyRejects(self.config.max_test_rejects),
                                ));
                                break 'stop;
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

        // test_data.logs already contains the appropriate logs:
        // - For failed tests: logs from the counterexample
        // - For successful tests with show_logs=true: all logs from all runs
        // - For successful tests with show_logs=false: logs from the last run only
        let result_logs = test_data.logs;

        let mut result = FuzzTestResult {
            first_case: test_data.first_case.unwrap_or_default(),
            gas_by_case: test_data.gas_by_case,
            success: test_data.failure.is_none(),
            skipped: false,
            reason: None,
            counterexample: None,
            logs: result_logs,
            labels: call.labels,
            traces: last_run_traces,
            breakpoints: last_run_breakpoints,
            gas_report_traces: traces.into_iter().map(|a| a.arena).collect(),
            line_coverage: test_data.coverage,
            deprecated_cheatcodes: test_data.deprecated_cheatcodes,
            failed_corpus_replays: corpus_manager.failed_replays(),
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

        Ok(result)
    }

    /// Granular and single-step function that runs only one fuzz and returns either a `CaseOutcome`
    /// or a `CounterExampleOutcome`
    fn single_fuzz(
        &mut self,
        address: Address,
        calldata: Bytes,
        coverage_metrics: &mut CorpusManager,
    ) -> Result<FuzzOutcome, TestCaseError> {
        let mut call = self
            .executor
            .call_raw(self.sender, address, calldata.clone(), U256::ZERO)
            .map_err(|e| TestCaseError::fail(e.to_string()))?;
        let new_coverage = coverage_metrics.merge_edge_coverage(&mut call);
        coverage_metrics.process_inputs(
            &[BasicTxDetails {
                warp: None,
                roll: None,
                sender: self.sender,
                call_details: CallDetails { target: address, calldata: calldata.clone() },
            }],
            new_coverage,
        );

        // Handle `vm.assume`.
        if call.result.as_ref() == MAGIC_ASSUME {
            return Err(TestCaseError::reject(FuzzError::AssumeReject));
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
}
