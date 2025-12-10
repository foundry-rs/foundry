use crate::executors::{
    DURATION_BETWEEN_METRICS_REPORT, EarlyExit, Executor, FuzzTestTimer, RawCallResult,
    corpus::{GlobalCorpusMetrics, WorkerCorpus},
};
use alloy_dyn_abi::JsonAbiExt;
use alloy_json_abi::Function;
use alloy_primitives::{Address, Bytes, Log, U256, keccak256, map::HashMap};
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
    test_runner::{RngAlgorithm, TestCaseError, TestRng, TestRunner},
};
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use serde_json::json;
use std::{
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU32, Ordering},
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};

mod types;
pub use types::{CaseOutcome, CounterExampleOutcome, FuzzOutcome};

/// Corpus syncs across workers every `SYNC_INTERVAL` runs.
const SYNC_INTERVAL: u32 = 1000;

#[derive(Default)]
struct WorkerState {
    /// Worker identifier
    id: usize,
    /// First fuzz case this worker encountered (with global run number)
    first_case: Option<(u32, FuzzCase)>,
    /// Gas usage for all cases this worker ran
    gas_by_case: Vec<(u64, u64)>,
    /// Counterexample if this worker found one
    counterexample: (Bytes, RawCallResult),
    /// Traces collected by this worker
    ///
    /// Stores up to `max_traces_to_collect` which is `config.gas_report_samples / num_workers`
    traces: Vec<SparsedTraceArena>,
    /// Last breakpoints from this worker
    breakpoints: Option<Breakpoints>,
    /// Coverage collected by this worker
    coverage: Option<HitMaps>,
    /// Logs from all cases this worker ran
    logs: Vec<Log>,
    /// Deprecated cheatcodes seen by this worker
    deprecated_cheatcodes: HashMap<&'static str, Option<&'static str>>,
    /// Number of runs this worker completed
    runs: u32,
    /// Failure reason if this worker failed
    failure: Option<TestCaseError>,
    /// Last run timestamp in milliseconds
    ///
    /// Used to identify which worker ran last and collect its traces and call breakpoints
    last_run_timestamp: u128,
    /// Failed corpus replays
    failed_corpus_replays: usize,
}

impl WorkerState {
    fn new(worker_id: usize) -> Self {
        Self { id: worker_id, ..Default::default() }
    }
}

/// Shared state for coordinating parallel fuzz workers
struct SharedFuzzState {
    state: EvmFuzzState,
    /// Total runs across workers
    total_runs: Arc<AtomicU32>,
    /// Found failure
    ///
    /// The worker that found the failure sets it's ID.
    ///
    /// This ID is then used to correctly extract the failure reason and counterexample.
    failed_worker_id: OnceLock<usize>,
    /// Total rejects across workers
    total_rejects: Arc<AtomicU32>,
    /// Fuzz timer
    timer: FuzzTestTimer,
    /// Fail Fast coordinator
    early_exit: EarlyExit,
    /// Global corpus metrics
    global_corpus_metrics: GlobalCorpusMetrics,
}

impl SharedFuzzState {
    fn new(state: EvmFuzzState, timeout: Option<u32>, early_exit: EarlyExit) -> Self {
        Self {
            state,
            total_runs: Arc::new(AtomicU32::new(0)),
            failed_worker_id: OnceLock::new(),
            total_rejects: Arc::new(AtomicU32::new(0)),
            timer: FuzzTestTimer::new(timeout),
            early_exit,
            global_corpus_metrics: GlobalCorpusMetrics::default(),
        }
    }

    /// Increments the number of runs and returns the new value.
    fn increment_runs(&self) -> u32 {
        self.total_runs.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// Increments and returns the new value of the number of rejected tests.
    fn increment_rejects(&self) -> u32 {
        self.total_rejects.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// Returns `true` if the worker should continue running.
    fn should_continue(&self) -> bool {
        !(self.early_exit.should_stop() || self.timer.is_timed_out())
    }

    /// Returns true if the worker was able to claim the failure, false if failure was set by
    /// another worker
    fn try_claim_failure(&self, worker_id: usize) -> bool {
        let mut claimed = false;
        let _ = self.failed_worker_id.get_or_init(|| {
            claimed = true;
            self.early_exit.record_exit();
            worker_id
        });
        claimed
    }
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
        tokio_handle: &tokio::runtime::Handle,
    ) -> Result<FuzzTestResult> {
        // Stores the fuzz test execution data.
        let shared_state = SharedFuzzState::new(state, self.config.timeout, early_exit.clone());

        // Determine number of workers
        let num_workers = self.num_workers();
        debug!(num_workers);
        // Use single worker for deterministic behavior when replaying persisted failures
        let workers = (0..num_workers)
            .into_par_iter()
            .map(|worker_id| {
                let _guard = tokio_handle.enter();
                self.run_worker(
                    worker_id as usize,
                    func,
                    fuzz_fixtures,
                    address,
                    rd,
                    &shared_state,
                    progress,
                )
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(self.aggregate_results(workers, func, &shared_state))
    }

    /// Granular and single-step function that runs only one fuzz and returns either a `CaseOutcome`
    /// or a `CounterExampleOutcome`
    fn single_fuzz(
        &self,
        address: Address,
        calldata: Bytes,
        coverage_metrics: &mut WorkerCorpus,
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

    /// Aggregates the results from all workers
    fn aggregate_results(
        &self,
        mut workers: Vec<WorkerState>,
        func: &Function,
        shared_state: &SharedFuzzState,
    ) -> FuzzTestResult {
        let mut result = FuzzTestResult::default();

        // Find first case and last run worker. Set `failed_corpus_replays`.
        let mut first_case_candidate: Option<(u32, FuzzCase)> = None;
        let mut last_run_worker: Option<&WorkerState> = None;
        let mut last_run_timestamp = 0u128;
        for worker in &workers {
            if let Some((run, ref case)) = worker.first_case
                && first_case_candidate.as_ref().is_none_or(|&(r, _)| run < r)
            {
                first_case_candidate = Some((run, case.clone()));
            }

            if worker.last_run_timestamp > last_run_timestamp {
                last_run_timestamp = worker.last_run_timestamp;
                last_run_worker = Some(worker);
            }

            // Only retrieve from worker 0 i.e master worker which is responsible for replaying
            // persisted corpus.
            if worker.id == 0 {
                result.failed_corpus_replays = worker.failed_corpus_replays;
            }
        }
        result.first_case = first_case_candidate.map(|(_, case)| case).unwrap_or_default();
        let last_run_worker = last_run_worker.expect("at least one worker");

        if let Some(&failed_worker_id) = shared_state.failed_worker_id.get() {
            result.success = false;

            let failed_worker_idx = workers.iter().position(|w| w.id == failed_worker_id).unwrap();
            let failed_worker = &mut workers[failed_worker_idx];

            let (calldata, call) = std::mem::take(&mut failed_worker.counterexample);
            result.labels = call.labels;
            result.traces = call.traces.clone();
            result.breakpoints = call.cheatcodes.map(|c| c.breakpoints);

            match &failed_worker.failure {
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
        } else {
            result.success = true;
            result.traces = last_run_worker.traces.last().cloned();
            result.breakpoints = last_run_worker.breakpoints.clone();
        }

        for mut worker in workers {
            result.gas_by_case.append(&mut worker.gas_by_case);
            result.logs.append(&mut worker.logs);
            result.gas_report_traces.extend(worker.traces.into_iter().map(|t| t.arena));
            HitMaps::merge_opt(&mut result.line_coverage, worker.coverage);
            result.deprecated_cheatcodes.extend(worker.deprecated_cheatcodes);
        }

        if let Some(reason) = &result.reason
            && let Some(reason) = SkipReason::decode_self(reason)
        {
            result.skipped = true;
            result.reason = reason.0;
        }

        result
    }

    /// Runs a single fuzz worker
    #[allow(clippy::too_many_arguments)]
    #[instrument(name = "fuzz_worker", skip_all, fields(id = worker_id))]
    fn run_worker(
        &self,
        worker_id: usize,
        func: &Function,
        fuzz_fixtures: &FuzzFixtures,
        address: Address,
        rd: &RevertDecoder,
        shared_state: &SharedFuzzState,
        progress: Option<&ProgressBar>,
    ) -> Result<WorkerState> {
        // Prepare
        let dictionary_weight = self.config.dictionary.dictionary_weight.min(100);
        let strategy = proptest::prop_oneof![
            100 - dictionary_weight => fuzz_calldata(func.clone(), fuzz_fixtures),
            dictionary_weight => fuzz_calldata_from_state(func.clone(), &shared_state.state),
        ]
        .prop_map(move |calldata| BasicTxDetails {
            warp: None,
            roll: None,
            sender: Default::default(),
            call_details: CallDetails { target: Default::default(), calldata },
        });

        let mut corpus = WorkerCorpus::new(
            worker_id,
            self.config.corpus.clone(),
            strategy.boxed(),
            // Master worker replays the persisted corpus using the executor
            if worker_id == 0 { Some(&self.executor) } else { None },
            Some(func),
            None, // fuzzed_contracts for invariant tests
        )?;

        let mut worker = WorkerState::new(worker_id);
        let num_workers = self.num_workers();
        // We want to collect at least one trace which will be displayed to user.
        let max_traces_to_collect = std::cmp::max(1, self.config.gas_report_samples / num_workers);

        // Calculate worker-specific run limit when not using timer
        let worker_runs = if self.config.timeout.is_some() {
            // When using timer, workers run as many as possible
            u32::MAX
        } else {
            // Distribute runs evenly across workers, with worker 0 handling any remainder
            let base_runs = self.config.runs / num_workers;
            let remainder = self.config.runs % num_workers;
            if worker_id == 0 { base_runs + remainder } else { base_runs }
        };

        let mut runner_config = self.runner.config().clone();
        // Set the runner cases to worker_runs
        runner_config.cases = worker_runs;

        let mut runner = if let Some(seed) = self.config.seed {
            // For deterministic parallel fuzzing, derive a unique seed for each worker
            let worker_seed = if worker_id == 0 {
                // Master worker uses the provided seed as is.
                seed
            } else {
                // Derive a worker-specific seed using keccak256(seed || worker_id)
                let seed_data =
                    [&seed.to_be_bytes::<32>()[..], &worker_id.to_be_bytes()[..]].concat();
                U256::from_be_bytes(keccak256(seed_data).0)
            };
            trace!(target: "forge::test", ?worker_seed, "deterministic seed for worker {worker_id}");
            let rng = TestRng::from_seed(RngAlgorithm::ChaCha, &worker_seed.to_be_bytes::<32>());
            TestRunner::new_with_rng(runner_config, rng)
        } else {
            TestRunner::new(runner_config)
        };

        let mut persisted_failure = if self.persisted_failure.is_some() && worker_id == 0 {
            self.persisted_failure.as_ref()
        } else {
            None
        };

        // Offset to stagger corpus syncs across workers; so that workers don't sync at the same
        // time.
        let sync_offset = worker_id as u32 * 100;
        let mut runs_since_sync = 0;
        let sync_threshold = SYNC_INTERVAL + sync_offset;
        let mut last_metrics_report = Instant::now();
        // Continue while:
        // 1. Global state allows (not timed out, not at global limit, no failure found)
        // 2. Worker hasn't reached its specific run limit
        'stop: while shared_state.should_continue() && worker.runs < worker_runs {
            // If counterexample recorded, replay it first, without incrementing runs.
            let input = if worker_id == 0
                && let Some(failure) = persisted_failure.take()
                && failure.calldata.get(..4).is_some_and(|selector| func.selector() == selector)
            {
                failure.calldata.clone()
            } else {
                runs_since_sync += 1;
                if runs_since_sync >= sync_threshold {
                    let timer = Instant::now();
                    corpus.sync(
                        num_workers,
                        &self.executor,
                        Some(func),
                        None,
                        &shared_state.global_corpus_metrics,
                    )?;
                    trace!("Worker {worker_id} finished corpus sync in {:?}", timer.elapsed());
                    runs_since_sync = 0;
                }

                match corpus.new_input(&mut runner, &shared_state.state, func) {
                    Ok(input) => input,
                    Err(err) => {
                        worker.failure = Some(TestCaseError::fail(format!(
                            "failed to generate fuzzed input in worker {}: {err}",
                            worker.id
                        )));
                        shared_state.try_claim_failure(worker_id);
                        break 'stop;
                    }
                }
            };

            let mut inc_runs = || {
                let total_runs = shared_state.increment_runs();
                debug_assert!(
                    total_runs <= self.config.runs,
                    "worker runs were not distributed correctly"
                );
                worker.runs += 1;
                if let Some(progress) = progress {
                    progress.inc(1);
                }
                total_runs
            };

            worker.last_run_timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
            match self.single_fuzz(address, input, &mut corpus) {
                Ok(fuzz_outcome) => match fuzz_outcome {
                    FuzzOutcome::Case(case) => {
                        let total_runs = inc_runs();

                        if worker_id == 0 && self.config.corpus.collect_edge_coverage() {
                            if let Some(progress) = progress {
                                corpus.sync_metrics(&shared_state.global_corpus_metrics);
                                progress
                                    .set_message(format!("{}", shared_state.global_corpus_metrics));
                            } else if last_metrics_report.elapsed()
                                > DURATION_BETWEEN_METRICS_REPORT
                            {
                                corpus.sync_metrics(&shared_state.global_corpus_metrics);
                                // Display metrics inline.
                                let metrics = json!({
                                    "timestamp": SystemTime::now()
                                        .duration_since(UNIX_EPOCH)?
                                        .as_secs(),
                                    "test": func.name,
                                    "metrics": shared_state.global_corpus_metrics.load(),
                                });
                                let _ = sh_println!("{metrics}");
                                last_metrics_report = Instant::now();
                            }
                        }

                        worker.gas_by_case.push((case.case.gas, case.case.stipend));

                        if worker.first_case.is_none() {
                            worker.first_case = Some((total_runs, case.case));
                        }

                        if let Some(call_traces) = case.traces {
                            if worker.traces.len() == max_traces_to_collect as usize {
                                worker.traces.pop();
                            }
                            worker.traces.push(call_traces);
                            worker.breakpoints = Some(case.breakpoints);
                        }

                        // Always store logs from the last run in test_data.logs for display at
                        // verbosity >= 2. When show_logs is true,
                        // accumulate all logs. When false, only keep the last run's logs.
                        if self.config.show_logs {
                            worker.logs.extend(case.logs);
                        } else {
                            worker.logs = case.logs;
                        }

                        HitMaps::merge_opt(&mut worker.coverage, case.coverage);
                        worker.deprecated_cheatcodes = case.deprecated_cheatcodes;
                    }
                    FuzzOutcome::CounterExample(CounterExampleOutcome {
                        exit_reason: status,
                        counterexample: outcome,
                        ..
                    }) => {
                        inc_runs();

                        let reason = rd.maybe_decode(&outcome.1.result, status);
                        worker.logs.extend(outcome.1.logs.clone());
                        worker.counterexample = outcome;
                        worker.failure = Some(TestCaseError::fail(reason.unwrap_or_default()));
                        shared_state.try_claim_failure(worker_id);
                        break 'stop;
                    }
                },
                Err(err) => match err {
                    TestCaseError::Fail(_) => {
                        worker.failure = Some(err);
                        shared_state.try_claim_failure(worker_id);
                        break 'stop;
                    }
                    TestCaseError::Reject(_) => {
                        let max = self.config.max_test_rejects;

                        let total = shared_state.increment_rejects();

                        // Update progress bar to reflect rejected runs.
                        // TODO(dani): (pre-existing) conflicts with corpus metrics `set_message`
                        if !self.config.corpus.collect_edge_coverage()
                            && let Some(progress) = progress
                        {
                            progress.set_message(format!("([{total}] rejected)"));
                        }

                        if max > 0 && total > max {
                            worker.failure =
                                Some(TestCaseError::reject(FuzzError::TooManyRejects(max)));
                            shared_state.try_claim_failure(worker_id);
                            break 'stop;
                        }
                    }
                },
            }
        }

        if worker_id == 0 {
            worker.failed_corpus_replays = corpus.failed_replays;
        }

        // Logs stats
        trace!("worker {worker_id} fuzz stats");
        shared_state.state.log_stats();

        Ok(worker)
    }

    /// Determines the number of workers to run.
    fn num_workers(&self) -> u32 {
        if self.persisted_failure.is_some() {
            1
        } else if let Some(threads) = self.config.threads {
            threads as u32
        } else {
            rayon::current_num_threads() as u32
        }
    }
}
