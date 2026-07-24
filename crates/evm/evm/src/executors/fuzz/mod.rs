use crate::executors::{
    DURATION_BETWEEN_METRICS_REPORT, EarlyExit, Executor, FuzzTestTimer, RawCallResult,
    corpus::{GlobalCorpusMetrics, ReplayTarget, StatelessReplayTarget, WorkerCorpus},
    should_ignore_revert,
};
use alloy_dyn_abi::JsonAbiExt;
use alloy_json_abi::Function;
use alloy_primitives::{
    Address, Bytes, Log, U256, keccak256,
    map::{AddressHashMap, HashMap},
};
use eyre::Result;
use foundry_common::sh_println;
use foundry_config::FuzzConfig;
use foundry_evm_core::{
    Breakpoints,
    constants::{CHEATCODE_ADDRESS, MAGIC_ASSUME},
    decode::{RevertDecoder, SkipReason},
    evm::FoundryEvmNetwork,
};
use foundry_evm_coverage::HitMaps;
use foundry_evm_fuzz::{
    BaseCounterExample, BasicTxDetails, CallDetails, CounterExample, FuzzCase, FuzzError,
    FuzzFixtures, FuzzRunMetadata, FuzzTestResult,
    strategies::{EvmFuzzState, fuzz_calldata, fuzz_calldata_from_state, fuzz_msg_value},
};
use foundry_evm_traces::SparsedTraceArena;
use indicatif::ProgressBar;
use proptest::{
    strategy::{Just, Strategy},
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

mod frontier;
mod types;
use frontier::{FuzzBranchFrontier, FuzzBranchFrontierArtifact, FuzzFrontierRecorder};
pub use types::{CaseOutcome, CounterExampleOutcome, FuzzOutcome};

/// Corpus syncs across workers every `SYNC_INTERVAL` runs.
const SYNC_INTERVAL: u32 = 1000;

/// Minimum number of runs per worker.
/// This is mainly to reduce the overall number of rayon jobs.
const MIN_RUNS_PER_WORKER: u32 = 64;

struct WorkerState<FEN: FoundryEvmNetwork> {
    /// Worker identifier
    id: usize,
    /// First fuzz case this worker encountered (with global run number)
    first_case: Option<(u32, FuzzCase)>,
    /// Gas usage for all cases this worker ran
    gas_by_case: Vec<(u64, u64)>,
    /// Counterexample if this worker found one
    counterexample: Option<(BasicTxDetails, RawCallResult<FEN>)>,
    /// Traces collected by this worker
    ///
    /// Stores up to `max_traces_to_collect` which is `config.gas_report_samples / num_workers`
    traces: Vec<SparsedTraceArena>,
    /// Runtime bytecodes for the last collected trace.
    debug_bytecodes: AddressHashMap<Bytes>,
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
    /// Fuzz run metadata that produced the failure.
    failure_run: Option<FuzzRunMetadata>,
    /// Last run timestamp in milliseconds
    ///
    /// Used to identify which worker ran last and collect its traces and call breakpoints
    last_run_timestamp: u128,
    /// Failed corpus replays
    failed_corpus_replays: usize,
    /// Branch frontiers captured for symbolic follow-up.
    frontiers: Vec<FuzzBranchFrontier>,
}

impl<FEN: FoundryEvmNetwork> WorkerState<FEN> {
    fn new(worker_id: usize) -> Self {
        Self {
            id: worker_id,
            first_case: None,
            gas_by_case: Vec::new(),
            counterexample: None,
            traces: Vec::new(),
            debug_bytecodes: HashMap::default(),
            breakpoints: None,
            coverage: None,
            logs: Vec::new(),
            deprecated_cheatcodes: HashMap::default(),
            runs: 0,
            failure: None,
            failure_run: None,
            last_run_timestamp: 0,
            failed_corpus_replays: 0,
            frontiers: Vec::new(),
        }
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
    /// Global corpus metrics
    global_corpus_metrics: GlobalCorpusMetrics,

    /// Global test suite early exit.
    global_early_exit: EarlyExit,
    /// Local fuzz early exit.
    local_early_exit: EarlyExit,
}

impl SharedFuzzState {
    fn new(state: EvmFuzzState, timeout: Option<u32>, early_exit: EarlyExit) -> Self {
        Self {
            state,
            total_runs: Arc::new(AtomicU32::new(0)),
            failed_worker_id: OnceLock::new(),
            total_rejects: Arc::new(AtomicU32::new(0)),
            timer: FuzzTestTimer::new(timeout),
            global_corpus_metrics: GlobalCorpusMetrics::default(),
            global_early_exit: early_exit,
            local_early_exit: EarlyExit::new(true),
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
        !(self.global_early_exit.should_stop()
            || self.local_early_exit.should_stop()
            || self.timer.is_timed_out())
    }

    /// Returns true if the worker was able to claim the failure, false if failure was set by
    /// another worker
    fn try_claim_failure(&self, worker_id: usize) -> bool {
        let mut claimed = false;
        let _ = self.failed_worker_id.get_or_init(|| {
            claimed = true;
            self.local_early_exit.record_failure();
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
pub struct FuzzedExecutor<FEN: FoundryEvmNetwork> {
    /// The EVM executor.
    executor_f: Executor<FEN>,
    /// The fuzzer
    runner: TestRunner,
    /// The account that calls tests.
    sender: Address,
    /// The fuzz configuration.
    config: FuzzConfig,
    /// The persisted counterexample to be replayed, if any.
    persisted_failure: Option<BaseCounterExample>,
    /// The number of parallel workers.
    num_workers: usize,
}

impl<FEN: FoundryEvmNetwork> FuzzedExecutor<FEN> {
    /// Instantiates a fuzzed executor given a testrunner
    pub fn new(
        executor: Executor<FEN>,
        runner: TestRunner,
        sender: Address,
        config: FuzzConfig,
        persisted_failure: Option<BaseCounterExample>,
    ) -> Self {
        let run_limit = if config.run.is_some() { 1 } else { config.runs };
        let max_workers = if run_limit == 0 {
            0
        } else if config.run.is_some() {
            1
        } else {
            Ord::max(1, run_limit / MIN_RUNS_PER_WORKER)
        };
        let num_workers = Ord::min(rayon::current_num_threads(), max_workers as usize);
        Self { executor_f: executor, runner, sender, config, persisted_failure, num_workers }
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
        let shared_state = SharedFuzzState::new(state, self.config.timeout, early_exit.clone());

        let worker_ids = self.worker_ids();
        debug!(n = worker_ids.len(), "spawning workers");
        let workers = worker_ids
            .into_par_iter()
            .map(|worker_id| {
                let _guard = tokio_handle.enter();
                let _guard = info_span!("fuzz_worker", id = worker_id).entered();
                let timer = Instant::now();
                let r = self.run_worker(
                    worker_id,
                    func,
                    fuzz_fixtures,
                    address,
                    rd,
                    &shared_state,
                    progress,
                );
                debug!("finished in {:?}", timer.elapsed());
                r
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(self.aggregate_results(workers, func, &shared_state))
    }

    /// Replays the persisted single-call counterexample exactly once.
    ///
    /// Unlike [`Self::fuzz`], this never falls through to generated inputs if the persisted
    /// calldata now succeeds or is rejected by `vm.assume`.
    pub fn replay_persisted_failure(
        &mut self,
        func: &Function,
        address: Address,
        rd: &RevertDecoder,
    ) -> Result<FuzzTestResult> {
        let Some(failure) = self.persisted_failure.as_ref() else {
            return Ok(FuzzTestResult {
                skipped: true,
                reason: Some("no persisted fuzz failure to replay".to_string()),
                ..Default::default()
            });
        };

        let seed = failure.fuzz.seed.or(self.config.seed);
        if let Some(cheats) = self.executor_f.inspector_mut().cheatcodes.as_mut()
            && let Some(seed) = seed
        {
            let run = failure.fuzz.run.unwrap_or(1);
            let worker = failure.fuzz.worker.unwrap_or(0) as usize;
            cheats.set_seed(Self::fuzz_run_seed(seed, worker, run));
        }

        let mut tx = BasicTxDetails {
            warp: None,
            roll: None,
            sender: self.sender,
            call_details: CallDetails {
                target: address,
                calldata: failure.calldata.clone(),
                value: failure.value,
            },
        };
        self.resolve_stateless_tx(&mut tx)?;
        let mut call = self.executor_f.call_raw(
            tx.sender,
            tx.call_details.target,
            tx.call_details.calldata.clone(),
            tx.call_details.value.unwrap_or_default(),
        )?;
        if call.result.as_ref() == MAGIC_ASSUME {
            return Ok(FuzzTestResult {
                skipped: true,
                reason: Some("persisted fuzz failure rejected by `vm.assume`".to_string()),
                ..Default::default()
            });
        }
        if call.reverter == Some(CHEATCODE_ADDRESS)
            && let Some(reason) = SkipReason::decode(&call.result)
        {
            return Ok(FuzzTestResult { skipped: true, reason: reason.0, ..Default::default() });
        }

        let (breakpoints, deprecated_cheatcodes) =
            call.cheatcodes.as_ref().map_or_else(Default::default, |cheats| {
                (cheats.breakpoints.clone(), cheats.deprecated.clone())
            });
        let success =
            should_ignore_revert::<FEN>(self.config.fail_on_revert, address, call.reverter)
                || self.executor_f.is_raw_call_mut_success(address, &mut call, false);

        let mut result = FuzzTestResult {
            success,
            labels: call.labels.clone(),
            traces: call.traces.clone(),
            debug_bytecodes: call.debug_bytecodes.clone(),
            breakpoints: Some(breakpoints),
            deprecated_cheatcodes,
            ..Default::default()
        };

        if success {
            result.first_case = FuzzCase { gas: call.gas_used, stipend: call.stipend };
            result.gas_by_case.push((call.gas_used, call.stipend));
            result.line_coverage = call.line_coverage;
            result.logs = call.logs;
            result.gas_report_traces.extend(call.traces.into_iter().map(|trace| trace.arena));
        } else {
            let reason = if call.reverter == Some(CHEATCODE_ADDRESS) {
                SkipReason::decode(&call.result)
                    .map(|reason| reason.to_string())
                    .or_else(|| rd.maybe_decode(&call.result, call.exit_reason))
            } else {
                rd.maybe_decode(&call.result, call.exit_reason)
            };
            result.reason = reason;
            let args = tx
                .call_details
                .calldata
                .get(4..)
                .map_or_else(Vec::new, |data| func.abi_decode_input(data).unwrap_or_default());
            result.counterexample = Some(CounterExample::Single(
                BaseCounterExample::from_fuzz_tx(&tx, args, call.traces).with_fuzz_metadata(
                    FuzzRunMetadata::new(
                        seed,
                        failure.fuzz.run,
                        Some(failure.fuzz.worker.unwrap_or(0)),
                    ),
                ),
            ));
            result.logs = call.logs;
        }

        Ok(result)
    }

    /// Granular and single-step function that runs only one fuzz and returns either a `CaseOutcome`
    /// or a `CounterExampleOutcome`
    fn single_fuzz(
        &self,
        executor: &Executor<FEN>,
        address: Address,
        mut tx: BasicTxDetails,
        coverage_metrics: &mut WorkerCorpus,
        frontier_recorder: &mut FuzzFrontierRecorder,
        fuzz_run: Option<&FuzzRunMetadata>,
    ) -> Result<FuzzOutcome<FEN>, TestCaseError> {
        tx.sender = self.sender;
        tx.call_details.target = address;
        tx.warp = None;
        tx.roll = None;
        self.resolve_stateless_tx_with_executor(executor, &mut tx)
            .map_err(|e| TestCaseError::fail(e.to_string()))?;
        let mut call = executor
            .call_raw(
                tx.sender,
                tx.call_details.target,
                tx.call_details.calldata.clone(),
                tx.call_details.value.unwrap_or_default(),
            )
            .map_err(|e| TestCaseError::fail(e.to_string()))?;
        let cmp_values = call.evm_cmp_values.take().unwrap_or_default();
        let new_coverage = coverage_metrics.merge_edge_coverage(&mut call);
        // `new_coverage` is only meaningful when edge coverage is collected; otherwise
        // `merge_edge_coverage` always returns `false`, so record it as unknown for frontiers.
        let frontier_new_coverage =
            self.config.corpus.collect_edge_coverage().then_some(new_coverage);
        frontier_recorder.capture_stateless_call(fuzz_run, &tx, &cmp_values, frontier_new_coverage);
        coverage_metrics.process_inputs(&[tx.clone()], &[cmp_values], new_coverage, None);

        // Handle `vm.assume`.
        if call.result.as_ref() == MAGIC_ASSUME {
            return Err(TestCaseError::reject(FuzzError::AssumeReject));
        }

        let (breakpoints, deprecated_cheatcodes) =
            call.cheatcodes.as_ref().map_or_else(Default::default, |cheats| {
                (cheats.breakpoints.clone(), cheats.deprecated.clone())
            });

        // Consider call success if test should not fail on reverts and reverter is not the test
        // address or one of the network's cheatcode contracts.
        let success =
            should_ignore_revert::<FEN>(self.config.fail_on_revert, address, call.reverter)
                || executor.is_raw_call_mut_success(address, &mut call, false);

        if success {
            Ok(FuzzOutcome::Case(CaseOutcome {
                case: FuzzCase { gas: call.gas_used, stipend: call.stipend },
                traces: call.traces,
                debug_bytecodes: call.debug_bytecodes,
                coverage: call.line_coverage,
                breakpoints,
                logs: call.logs,
                deprecated_cheatcodes,
            }))
        } else {
            Ok(FuzzOutcome::CounterExample(CounterExampleOutcome {
                exit_reason: call.exit_reason,
                counterexample: (tx, call),
                breakpoints,
            }))
        }
    }

    fn resolve_stateless_tx(&self, tx: &mut BasicTxDetails) -> Result<()> {
        self.resolve_stateless_tx_with_executor(&self.executor_f, tx)
    }

    fn resolve_stateless_tx_with_executor(
        &self,
        executor: &Executor<FEN>,
        tx: &mut BasicTxDetails,
    ) -> Result<()> {
        tx.call_details.value = match tx.call_details.value {
            Some(requested) if !requested.is_zero() => {
                let value = requested.min(executor.get_balance(tx.sender)?);
                (!value.is_zero()).then_some(value)
            }
            _ => None,
        };
        Ok(())
    }

    /// Aggregates the results from all workers
    fn aggregate_results(
        &self,
        mut workers: Vec<WorkerState<FEN>>,
        func: &Function,
        shared_state: &SharedFuzzState,
    ) -> FuzzTestResult {
        self.write_branch_frontiers(&mut workers, func);

        let mut result = FuzzTestResult::default();
        if workers.is_empty() {
            result.success = true;
            return result;
        }

        // Find first case and last run worker. Set `failed_corpus_replays`.
        let mut first_case_candidate = None;
        let mut last_run_worker = None;
        for (i, worker) in workers.iter().enumerate() {
            if let Some((run, ref case)) = worker.first_case
                && first_case_candidate.as_ref().is_none_or(|&(r, _)| run < r)
            {
                first_case_candidate = Some((run, case.clone()));
            }

            if last_run_worker.is_none_or(|(t, _)| worker.last_run_timestamp > t) {
                last_run_worker = Some((worker.last_run_timestamp, i));
            }

            // Only set replays from master which is responsible for replaying persisted corpus.
            if worker.id == 0 {
                result.failed_corpus_replays = worker.failed_corpus_replays;
            }
        }
        result.first_case = first_case_candidate.map(|(_, case)| case).unwrap_or_default();
        let (_, last_run_worker_idx) = last_run_worker.expect("at least one worker");

        if let Some(&failed_worker_id) = shared_state.failed_worker_id.get() {
            result.success = false;

            let failed_worker_idx = workers.iter().position(|w| w.id == failed_worker_id).unwrap();
            let failed_worker = &mut workers[failed_worker_idx];

            let counterexample = failed_worker.counterexample.take();
            if let Some((_, call)) = &counterexample {
                result.labels.clone_from(&call.labels);
                result.traces.clone_from(&call.traces);
                result.debug_bytecodes.clone_from(&call.debug_bytecodes);
                result.breakpoints = call.cheatcodes.as_ref().map(|c| c.breakpoints.clone());
            }

            match &failed_worker.failure {
                Some(TestCaseError::Fail(reason)) => {
                    let reason = reason.to_string();
                    result.reason = (!reason.is_empty()).then_some(reason);
                    if let Some((tx, call)) = counterexample {
                        let args =
                            tx.call_details.calldata.get(4..).map_or_else(Vec::new, |data| {
                                func.abi_decode_input(data).unwrap_or_default()
                            });
                        let fuzz = failed_worker.failure_run.unwrap_or_default();
                        result.counterexample = Some(CounterExample::Single(
                            BaseCounterExample::from_fuzz_tx(&tx, args, call.traces)
                                .with_fuzz_metadata(FuzzRunMetadata::new(
                                    fuzz.seed.or(self.config.seed),
                                    fuzz.run,
                                    fuzz.worker,
                                )),
                        ));
                    }
                }
                Some(TestCaseError::Reject(reason)) => {
                    let reason = reason.to_string();
                    result.reason = (!reason.is_empty()).then_some(reason);
                }
                None => {}
            }
        } else {
            let last_run_worker = &workers[last_run_worker_idx];
            result.success = true;
            result.traces = last_run_worker.traces.last().cloned();
            result.debug_bytecodes.clone_from(&last_run_worker.debug_bytecodes);
            result.breakpoints = last_run_worker.breakpoints.clone();
        }

        if !self.config.show_logs {
            result.logs = workers[last_run_worker_idx].logs.clone();
        }

        for mut worker in workers {
            result.gas_by_case.append(&mut worker.gas_by_case);
            if self.config.show_logs {
                result.logs.append(&mut worker.logs);
            }
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

    fn write_branch_frontiers(&self, workers: &mut [WorkerState<FEN>], func: &Function) {
        let Some(frontier_dir) = &self.config.corpus.frontier_dir else {
            return;
        };
        let limit = self.config.corpus.frontier_limit;
        if limit == 0 {
            return;
        }

        let frontiers = frontier::merge_frontiers(
            limit,
            workers.iter_mut().flat_map(|worker| worker.frontiers.drain(..)),
        );
        if frontiers.is_empty() {
            return;
        }

        let artifact = FuzzBranchFrontierArtifact::new(func, limit, frontiers);
        if let Err(err) = frontier::write_frontier_artifact(frontier_dir, &artifact) {
            warn!(%err, path = ?frontier_dir, "failed to write fuzz branch frontier artifact");
        }
    }

    /// Runs a single fuzz worker
    #[allow(clippy::too_many_arguments)]
    fn run_worker(
        &self,
        worker_id: usize,
        func: &Function,
        fuzz_fixtures: &FuzzFixtures,
        address: Address,
        rd: &RevertDecoder,
        shared_state: &SharedFuzzState,
        progress: Option<&ProgressBar>,
    ) -> Result<WorkerState<FEN>> {
        // Prepare
        let fuzz_state = shared_state.state.fork();
        let dictionary_weight = self.config.dictionary.dictionary_weight.min(100);
        let calldata_strategy = proptest::prop_oneof![
            100 - dictionary_weight => fuzz_calldata(func.clone(), fuzz_fixtures),
            dictionary_weight => fuzz_calldata_from_state(func.clone(), &fuzz_state, fuzz_fixtures),
        ];
        let value_strategy = if func.state_mutability == alloy_json_abi::StateMutability::Payable {
            fuzz_msg_value(self.config.corpus.payable_value_weight).boxed()
        } else {
            Just(None).boxed()
        };
        let sender = self.sender;
        let strategy =
            (calldata_strategy, value_strategy).prop_map(move |(calldata, value)| BasicTxDetails {
                warp: None,
                roll: None,
                sender,
                call_details: CallDetails { target: address, calldata, value },
            });

        let replay_target = ReplayTarget {
            stateless: Some(StatelessReplayTarget { function: func, address }),
            fuzzed_contracts: None,
            dynamic: None,
        };
        let mut corpus = WorkerCorpus::new(
            worker_id,
            self.config.corpus.clone(),
            strategy.boxed(),
            // Master worker replays the persisted corpus using the executor
            (worker_id == 0).then_some(&self.executor_f),
            replay_target,
        )?;
        let mut executor = self.executor_f.clone();
        let frontier_limit = if self.config.corpus.capture_branch_frontiers() {
            self.config.corpus.frontier_limit
        } else {
            0
        };
        let mut frontier_recorder = FuzzFrontierRecorder::new(frontier_limit);

        let mut worker = WorkerState::new(worker_id);
        // We want to collect at least one trace which will be displayed to user.
        let max_traces_to_collect =
            std::cmp::max(1, self.config.gas_report_samples / self.num_workers as u32);

        let worker_runs = self.runs_per_worker(worker_id);
        debug!(worker_runs);

        let mut runner_config = self.runner.config().clone();
        runner_config.cases = worker_runs;

        let mut runner = if let Some(seed) = self.config.seed {
            let worker_seed = Self::fuzz_worker_seed(seed, worker_id);
            trace!(target: "forge::test", ?worker_seed, "deterministic seed for worker {worker_id}");
            let rng = TestRng::from_seed(RngAlgorithm::ChaCha, &worker_seed.to_be_bytes::<32>());
            TestRunner::new_with_rng(runner_config, rng)
        } else {
            TestRunner::new(runner_config)
        };

        if let Some(target_run) = self.config.run {
            for _ in 1..target_run {
                if let Err(err) = corpus.new_input(&mut runner, &fuzz_state, func) {
                    worker.failure = Some(TestCaseError::fail(format!(
                        "failed to generate fuzzed input in worker {}: {err}",
                        worker.id
                    )));
                    shared_state.try_claim_failure(worker_id);
                    return Ok(worker);
                }
            }
        }

        let mut persisted_failure =
            self.persisted_failure.as_ref().filter(|_| worker_id == 0 && self.config.run.is_none());

        // Offset to stagger corpus syncs across workers; so that workers don't sync at the same
        // time.
        let sync_offset = (worker_id as u32).saturating_mul(100);
        let sync_threshold = SYNC_INTERVAL + sync_offset;
        let mut runs_since_sync = sync_threshold; // Always sync at the start.
        let mut last_metrics_report = Instant::now();
        // Continue while:
        // 1. Global state allows (not timed out, not at global limit, no failure found)
        // 2. Worker hasn't reached its specific run limit
        'stop: while shared_state.should_continue() && worker.runs < worker_runs {
            // If counterexample recorded, replay it first, without incrementing runs.
            let (input, fuzz_run) = if worker_id == 0
                && let Some(failure) = persisted_failure.take()
                && failure.calldata.get(..4).is_some_and(|selector| func.selector() == selector)
            {
                let seed = failure.fuzz.seed.or(self.config.seed);
                if let Some(cheats) = executor.inspector_mut().cheatcodes.as_mut()
                    && let Some(seed) = seed
                {
                    let run = failure.fuzz.run.unwrap_or(1);
                    let worker = failure.fuzz.worker.unwrap_or(worker_id as u32) as usize;
                    cheats.set_seed(Self::fuzz_run_seed(seed, worker, run));
                }

                (
                    BasicTxDetails {
                        warp: None,
                        roll: None,
                        sender: self.sender,
                        call_details: CallDetails {
                            target: address,
                            calldata: failure.calldata.clone(),
                            value: failure.value,
                        },
                    },
                    Some(FuzzRunMetadata::new(
                        seed,
                        failure.fuzz.run,
                        Some(failure.fuzz.worker.unwrap_or(worker_id as u32)),
                    )),
                )
            } else {
                runs_since_sync += 1;
                if runs_since_sync >= sync_threshold {
                    let timer = Instant::now();
                    corpus.sync(
                        self.num_workers,
                        &executor,
                        replay_target,
                        &shared_state.global_corpus_metrics,
                    )?;
                    trace!("finished corpus sync in {:?}", timer.elapsed());
                    runs_since_sync = 0;
                }

                let fuzz_run = self.config.run.unwrap_or(worker.runs + 1);
                if let Some(cheats) = executor.inspector_mut().cheatcodes.as_mut()
                    && let Some(seed) = self.config.seed
                {
                    cheats.set_seed(Self::fuzz_run_seed(seed, worker_id, fuzz_run));
                }

                let input = match corpus.new_input(&mut runner, &fuzz_state, func) {
                    Ok(input) => input,
                    Err(err) => {
                        worker.failure = Some(TestCaseError::fail(format!(
                            "failed to generate fuzzed input in worker {}: {err}",
                            worker.id
                        )));
                        shared_state.try_claim_failure(worker_id);
                        break 'stop;
                    }
                };

                (
                    input,
                    Some(FuzzRunMetadata::new(
                        self.config.seed,
                        Some(fuzz_run),
                        Some(worker_id as u32),
                    )),
                )
            };

            let mut inc_runs = || {
                let total_runs = shared_state.increment_runs();
                debug_assert!(
                    shared_state.timer.is_enabled()
                        || total_runs
                            <= if self.config.run.is_some() { 1 } else { self.config.runs },
                    "worker runs were not distributed correctly"
                );
                worker.runs += 1;
                if let Some(progress) = progress {
                    progress.inc(1);
                }
                total_runs
            };

            worker.last_run_timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
            match self.single_fuzz(
                &executor,
                address,
                input,
                &mut corpus,
                &mut frontier_recorder,
                fuzz_run.as_ref(),
            ) {
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
                            worker.debug_bytecodes = case.debug_bytecodes;
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
                        worker.failure_run = fuzz_run;

                        // Only classify magic skip payloads when the revert originates from the
                        // cheatcode address.
                        let reason = if outcome.1.reverter == Some(CHEATCODE_ADDRESS) {
                            SkipReason::decode(&outcome.1.result)
                                .map(|reason| reason.to_string())
                                .or_else(|| rd.maybe_decode(&outcome.1.result, status))
                        } else {
                            rd.maybe_decode(&outcome.1.result, status)
                        };
                        worker.logs.extend(outcome.1.logs.clone());
                        worker.counterexample = Some(outcome);
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
        worker.frontiers = frontier_recorder.into_frontiers();

        // Logs stats
        trace!("worker {worker_id} fuzz stats");
        fuzz_state.log_stats();

        Ok(worker)
    }

    /// Determines the number of runs per worker.
    const fn runs_per_worker(&self, worker_id: usize) -> u32 {
        let worker_id = worker_id as u32;
        let total_runs = if self.config.run.is_some() { 1 } else { self.config.runs };
        let n = self.num_workers as u32;
        let runs = total_runs / n;
        let remainder = total_runs % n;
        // Distribute the remainder evenly among the first `remainder` workers,
        // assuming `worker_id` is in `0..n`.
        if worker_id < remainder { runs + 1 } else { runs }
    }

    /// Returns the worker IDs to execute.
    fn worker_ids(&self) -> Vec<usize> {
        if self.config.run.is_some() {
            vec![self.config.worker.unwrap_or(0) as usize]
        } else {
            (0..self.num_workers).collect()
        }
    }

    /// Derives the deterministic RNG seed for a fuzz worker.
    fn fuzz_worker_seed(seed: U256, worker_id: usize) -> U256 {
        if worker_id == 0 {
            seed
        } else {
            let worker_id = worker_id as u32;
            let seed_data = [&seed.to_be_bytes::<32>()[..], &worker_id.to_be_bytes()[..]].concat();
            U256::from_be_bytes(keccak256(seed_data).0)
        }
    }

    /// Derives the deterministic RNG seed for cheatcode randomness in a worker-local run.
    fn fuzz_run_seed(seed: U256, worker_id: usize, run: u32) -> U256 {
        Self::fuzz_worker_seed(seed, worker_id).wrapping_add(U256::from(run.saturating_sub(1)))
    }
}
