use crate::{
    executors::{
        DURATION_BETWEEN_METRICS_REPORT, EarlyExit, EvmError, Executor, RawCallResult,
        corpus::{
            CorpusInsertionMode, DynamicTargetCtx, ReplayTarget, WorkerCorpus, WorkerCorpusSeed,
        },
    },
    inspectors::Fuzzer,
};
use alloy_json_abi::Function;
use alloy_primitives::{
    Address, Bytes, FixedBytes, I256, Selector, U256, keccak256, map::AddressMap,
};
use alloy_sol_types::{SolCall, sol};
use eyre::{ContextCompat, Result, eyre};
use foundry_common::{
    TestFunctionExt,
    contracts::{ContractsByAddress, ContractsByArtifact},
    sh_eprintln, sh_println,
};
use foundry_config::{InvariantConfig, InvariantWorkers};
use foundry_evm_core::{
    FoundryBlock,
    constants::{
        CALLER, CHEATCODE_ADDRESS, DEFAULT_CREATE2_DEPLOYER, HARDHAT_CONSOLE_ADDRESS, MAGIC_ASSUME,
    },
    evm::FoundryEvmNetwork,
    precompiles::PRECOMPILES,
};
use foundry_evm_fuzz::{
    BasicTxDetails, FuzzCase, FuzzFixtures,
    invariant::{
        ArtifactFilters, FuzzRunIdentifiedContracts, InvariantContract, InvariantSettings,
        RandomCallGenerator, SenderFilters, TargetedContract, TargetedContracts,
    },
    strategies::{EvmFuzzState, InvariantFuzzState, invariant_strat, override_call_strat},
};
use foundry_evm_traces::{CallTraceArena, SparsedTraceArena};
use indicatif::ProgressBar;
use parking_lot::RwLock;
use proptest::{
    strategy::Strategy,
    test_runner::{RngAlgorithm, TestRng, TestRunner},
};
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use result::{assert_after_invariant, can_continue, did_fail_on_assert, invariant_preflight_check};
use revm::{context::Block, state::Account};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::{HashMap as Map, HashSet, btree_map::Entry},
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

mod error;
pub use error::{
    FailureKey, HandlerAssertionFailure, InvariantFailures, InvariantFuzzError,
    handler_site_already_minimal,
};
use foundry_evm_coverage::HitMaps;

mod campaign;
use campaign::{
    InvariantCampaignAggregator, InvariantCampaignSpec, InvariantCampaignState,
    InvariantWorkerOutput, InvariantWorkerPlan,
};

mod replay;
pub use replay::{replay_error, replay_run};

mod result;
pub use result::InvariantFuzzTestResult;

mod shrink;
pub use shrink::{
    CheckSequenceOptions, HandlerReplayOutcome, check_sequence, check_sequence_value,
    replay_handler_failure_sequence,
};

/// Minimum number of logical runs assigned to each auto invariant worker at the default invariant
/// depth.
///
/// Keeps short campaigns single-threaded and avoids producing many small rayon jobs.
const MIN_RUNS_PER_INVARIANT_WORKER: u32 = 10_000;
/// Baseline depth used to preserve the previous default-depth worker heuristic.
const DEFAULT_DEPTH_FOR_INVARIANT_WORKER_CAP: u32 = 500;
/// Minimum estimated handler calls assigned to each auto invariant worker.
const MIN_ESTIMATED_CALLS_PER_INVARIANT_WORKER: u64 =
    MIN_RUNS_PER_INVARIANT_WORKER as u64 * DEFAULT_DEPTH_FOR_INVARIANT_WORKER_CAP as u64;

sol! {
    interface IInvariantTest {
        #[derive(Default)]
        struct FuzzSelector {
            address addr;
            bytes4[] selectors;
        }

        #[derive(Default)]
        struct FuzzArtifactSelector {
            string artifact;
            bytes4[] selectors;
        }

        #[derive(Default)]
        struct FuzzInterface {
            address addr;
            string[] artifacts;
        }

        function afterInvariant() external;

        #[derive(Default)]
        function excludeArtifacts() public view returns (string[] memory excludedArtifacts);

        #[derive(Default)]
        function excludeContracts() public view returns (address[] memory excludedContracts);

        #[derive(Default)]
        function excludeSelectors() public view returns (FuzzSelector[] memory excludedSelectors);

        #[derive(Default)]
        function excludeSenders() public view returns (address[] memory excludedSenders);

        #[derive(Default)]
        function targetArtifacts() public view returns (string[] memory targetedArtifacts);

        #[derive(Default)]
        function targetArtifactSelectors() public view returns (FuzzArtifactSelector[] memory targetedArtifactSelectors);

        #[derive(Default)]
        function targetContracts() public view returns (address[] memory targetedContracts);

        #[derive(Default)]
        function targetSelectors() public view returns (FuzzSelector[] memory targetedSelectors);

        #[derive(Default)]
        function targetSenders() public view returns (address[] memory targetedSenders);

        #[derive(Default)]
        function targetInterfaces() public view returns (FuzzInterface[] memory targetedInterfaces);
    }
}

/// Contains invariant metrics for a single fuzzed selector.
#[derive(Default, Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct InvariantMetrics {
    // Count of fuzzed selector calls.
    pub calls: usize,
    // Count of fuzzed selector reverts.
    pub reverts: usize,
    // Count of fuzzed selector discards (through assume cheatcodes).
    pub discards: usize,
}

/// Campaign-level throughput metrics for invariant progress reporting.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct InvariantThroughputMetrics {
    total_txs: u64,
    total_gas: u64,
}

impl InvariantThroughputMetrics {
    fn tps(self, elapsed: Duration) -> f64 {
        round_rate_for_progress(rate_per_sec(self.total_txs as f64, elapsed))
    }

    fn gps(self, elapsed: Duration) -> f64 {
        round_rate_for_progress(rate_per_sec(self.total_gas as f64, elapsed))
    }
}

fn max_invariant_workers_for_campaign(runs: u32, depth: u32) -> usize {
    let estimated_calls = u64::from(runs) * u64::from(depth.max(1));
    usize::try_from((estimated_calls / MIN_ESTIMATED_CALLS_PER_INVARIANT_WORKER).max(1))
        .unwrap_or(usize::MAX)
}

fn auto_invariant_worker_count(
    available_threads: usize,
    invariant_campaign_anchors: usize,
) -> usize {
    (available_threads.max(1) / invariant_campaign_anchors.max(1)).max(1)
}

fn invariant_worker_count_with_threads(
    config: &InvariantConfig,
    available_threads: usize,
    invariant_campaign_anchors: usize,
) -> usize {
    match config.workers {
        InvariantWorkers::Fixed(workers) => workers.get(),
        InvariantWorkers::Auto => {
            let requested =
                auto_invariant_worker_count(available_threads, invariant_campaign_anchors);
            if config.timeout.is_some() {
                requested
            } else {
                requested.min(max_invariant_workers_for_campaign(config.runs, config.depth))
            }
        }
    }
}

fn gas_report_samples_for_worker(total_samples: u32, worker_id: u32, worker_count: usize) -> usize {
    let total_samples = total_samples as usize;
    let worker_count = worker_count.max(1);
    total_samples / worker_count + usize::from((worker_id as usize) < total_samples % worker_count)
}

const fn invariant_worker_collects_evm_cmp_log(
    config: &InvariantConfig,
    worker_id: u32,
    worker_count: usize,
) -> bool {
    config.corpus.collect_evm_cmp_log() && (worker_count <= 1 || worker_id == 0)
}

fn invariant_worker_seed(seed: U256, worker_id: u32) -> U256 {
    if worker_id == 0 {
        seed
    } else {
        let seed_data = [&seed.to_be_bytes::<32>()[..], &worker_id.to_be_bytes()[..]].concat();
        U256::from_be_bytes(keccak256(seed_data).0)
    }
}

fn should_continue_invariant_worker(
    campaign_state: &InvariantCampaignState,
    runs: u32,
    plan: InvariantWorkerPlan,
) -> bool {
    if campaign_state.should_stop() {
        return false;
    }

    campaign_state.is_timed_campaign() || runs < plan.runs
}

fn invariant_worker_runner(
    runner: &mut TestRunner,
    worker_id: u32,
    seed: Option<U256>,
) -> TestRunner {
    if let Some(seed) = seed {
        let worker_seed = invariant_worker_seed(seed, worker_id);
        trace!(target: "forge::test", ?worker_seed, "deterministic seed for invariant worker {worker_id}");
        let rng = TestRng::from_seed(RngAlgorithm::ChaCha, &worker_seed.to_be_bytes::<32>());
        TestRunner::new_with_rng(runner.config().clone(), rng)
    } else if worker_id == 0 {
        runner.clone()
    } else {
        TestRunner::new_with_rng(runner.config().clone(), runner.new_rng())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InvariantCorpusPersistence {
    /// Preserve the legacy single-worker behavior: each interesting input is written immediately.
    Live,
    /// Parallel workers return interesting inputs to the campaign coordinator for merged writes.
    Deferred,
}

impl InvariantCorpusPersistence {
    const fn is_deferred(self) -> bool {
        matches!(self, Self::Deferred)
    }
}

/// Converts a cumulative campaign total into an average per-second rate.
///
/// Returns `0.0` during the initial zero-elapsed startup window to avoid
/// dividing by zero while progress reporting is warming up.
fn rate_per_sec(total: f64, elapsed: Duration) -> f64 {
    let elapsed_secs = elapsed.as_secs_f64();
    if elapsed_secs > 0.0 { total / elapsed_secs } else { 0.0 }
}

fn round_rate_for_progress(rate: f64) -> f64 {
    (rate * 100.0).round() / 100.0
}

/// Tracks invariant failure counts during a campaign.
#[derive(Clone, Debug, Default)]
struct InvariantFailureMetrics {
    failures: u64,
    unique_failures: HashSet<String>,
    /// Unique handler-side assertion bugs found so far.
    broken_handlers: usize,
}

impl InvariantFailureMetrics {
    /// Records a failure and emits a structured JSON `"failure"` event.
    fn record_failure(&mut self, invariant_name: &str, target: &str, reason: &str) {
        self.failures += 1;
        self.unique_failures.insert(invariant_name.to_string());

        let timestamp =
            SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let event = json!({
            "timestamp": timestamp,
            "event": "failure",
            "invariant": invariant_name,
            "target": target,
            "reason": reason,
        });
        let _ = sh_eprintln!("{}", serde_json::to_string(&event).unwrap_or_default());
    }
}

/// Bridges newly-recorded invariant breaks from `failures.errors` into the pulse
/// `failure_metrics` so the live progress stream reflects breaks as they happen.
/// Iterates in declaration order so the emitted "failure" events are deterministic.
fn record_new_invariant_failures(
    campaign_state: &InvariantCampaignState,
    invariant_contract: &InvariantContract<'_>,
    failures: &InvariantFailures,
) {
    for (f, _) in &invariant_contract.invariant_fns {
        if let Some(failure) = failures.get_failure(f) {
            let reason = failure.revert_reason().unwrap_or_default();
            campaign_state.record_invariant_failure(&f.name, invariant_contract.name, &reason);
        }
    }
}

struct InvariantProgressContext<'a> {
    timestamp_secs: u64,
    contract_name: &'a str,
    optimization_best: Option<I256>,
    throughput: InvariantThroughputMetrics,
    elapsed: Duration,
    worker_id: u32,
    worker_count: usize,
}

/// Builds the machine-readable invariant progress payload emitted during a
/// campaign.
///
/// This keeps the existing corpus progress metrics together with cumulative and
/// derived throughput fields so downstream benchmark tooling can consume a
/// single JSON event shape.
fn build_invariant_progress_json<M: Serialize>(
    context: InvariantProgressContext<'_>,
    corpus_metrics: &M,
    failure_metrics: &InvariantFailureMetrics,
) -> serde_json::Value {
    let mut metrics = serde_json::to_value(corpus_metrics).unwrap_or_default();
    if let Some(obj) = metrics.as_object_mut() {
        obj.insert("broken_invariants".to_string(), json!(failure_metrics.unique_failures.len()));
        obj.insert("broken_assertions".to_string(), json!(failure_metrics.broken_handlers));
    }

    let mut payload = json!({
        "timestamp": context.timestamp_secs,
        "event": "pulse",
        "contract": context.contract_name,
        "metrics": metrics,
        "total_txs": context.throughput.total_txs,
        "total_gas": context.throughput.total_gas,
        "tps": context.throughput.tps(context.elapsed),
        "gps": context.throughput.gps(context.elapsed),
        "worker": {
            "id": context.worker_id,
            "count": context.worker_count,
        },
    });

    if let Some(best) = context.optimization_best {
        payload["optimization_best"] = json!(best.to_string());
    }

    payload
}

/// Contains data collected during invariant test runs.
struct InvariantTestData {
    // Number of completed invariant runs.
    runs: usize,
    // Number of completed fuzzed calls across all invariant runs.
    calls: usize,
    // Data related to reverts or failed assertions of the test.
    failures: InvariantFailures,
    // Calldata in the last invariant run.
    last_run_inputs: Vec<BasicTxDetails>,
    // Additional traces for gas report.
    gas_report_traces: Vec<Vec<CallTraceArena>>,
    // Line coverage information collected from all fuzzed calls.
    line_coverage: Option<HitMaps>,
    // Metrics for each fuzzed selector.
    metrics: Map<String, InvariantMetrics>,

    // Proptest runner to query for random values.
    // The strategy only comes with the first `input`. We fill the rest of the `inputs`
    // until the desired `depth` so we can use the evolving fuzz dictionary
    // during the run.
    branch_runner: TestRunner,

    // Optimization mode state: tracks the best (maximum) value and the sequence that produced it.
    // Only used when invariant function returns int256.
    optimization_best_value: Option<I256>,
    optimization_best_sequence: Vec<BasicTxDetails>,
}

/// Contains invariant test data.
struct InvariantTest {
    // Fuzz state of invariant test.
    fuzz_state: InvariantFuzzState,
    // Contracts fuzzed by the invariant test.
    targeted_contracts: FuzzRunIdentifiedContracts,
    // Data collected during invariant runs.
    test_data: InvariantTestData,
}

impl InvariantTest {
    /// Instantiates an invariant test.
    fn new(
        fuzz_state: InvariantFuzzState,
        targeted_contracts: FuzzRunIdentifiedContracts,
        failures: InvariantFailures,
        branch_runner: TestRunner,
    ) -> Self {
        let test_data = InvariantTestData {
            runs: 0,
            calls: 0,
            failures,
            last_run_inputs: vec![],
            gas_report_traces: vec![],
            line_coverage: None,
            metrics: Map::default(),
            branch_runner,
            optimization_best_value: None,
            optimization_best_sequence: vec![],
        };
        Self { fuzz_state, targeted_contracts, test_data }
    }

    /// Returns number of invariant test reverts.
    const fn reverts(&self) -> usize {
        self.test_data.failures.reverts
    }

    /// Set invariant test error.
    fn set_error(&mut self, invariant: &Function, error: InvariantFuzzError) {
        self.test_data.failures.record_failure(invariant, error);
    }

    /// Set last invariant run call sequence.
    fn set_last_run_inputs(&mut self, inputs: &Vec<BasicTxDetails>) {
        self.test_data.last_run_inputs.clone_from(inputs);
    }

    /// Merge current collected line coverage with the new coverage from last fuzzed call.
    fn merge_line_coverage(&mut self, new_coverage: Option<HitMaps>) {
        HitMaps::merge_opt(&mut self.test_data.line_coverage, new_coverage);
    }

    /// Update metrics for a fuzzed selector, extracted from tx details.
    /// Always increments number of calls; discarded runs (through assume cheatcodes) are tracked
    /// separated from reverts.
    fn record_metrics(&mut self, tx_details: &BasicTxDetails, reverted: bool, discarded: bool) {
        if let Some(metric_key) = self.targeted_contracts.targets().fuzzed_metric_key(tx_details) {
            let test_metrics = &mut self.test_data.metrics;
            let invariant_metrics = test_metrics.entry(metric_key).or_default();
            invariant_metrics.calls += 1;
            if discarded {
                invariant_metrics.discards += 1;
            } else if reverted {
                invariant_metrics.reverts += 1;
            }
        }
    }

    /// End invariant test run by collecting results, cleaning collected artifacts and reverting
    /// created fuzz state.
    fn end_run<FEN: FoundryEvmNetwork>(&mut self, run: InvariantTestRun<FEN>, gas_samples: usize) {
        // We clear all the targeted contracts created during this run.
        self.targeted_contracts.clear_created_contracts(run.created_contracts);

        if self.test_data.gas_report_traces.len() < gas_samples {
            self.test_data
                .gas_report_traces
                .push(run.run_traces.into_iter().map(|arena| arena.arena).collect());
        }
        self.test_data.runs += 1;
        self.test_data.calls += run.fuzz_runs.len();

        // Revert state to not persist values between runs.
        self.fuzz_state.revert();
    }

    /// Updates the optimization state if the new value is better (higher) than the current best.
    fn update_optimization_value(&mut self, value: I256, sequence: &[BasicTxDetails]) {
        if self.test_data.optimization_best_value.is_none_or(|best| value > best) {
            self.test_data.optimization_best_value = Some(value);
            self.test_data.optimization_best_sequence = sequence.to_vec();
        }
    }
}

/// Contains data for an invariant test run.
struct InvariantTestRun<FEN: FoundryEvmNetwork> {
    // Invariant run call sequence.
    inputs: Vec<BasicTxDetails>,
    // Per-call EVM comparison operands (parallel to `inputs`), captured for I2S corpus mutation.
    cmp_seq: Vec<Vec<crate::inspectors::CmpOperands>>,
    // Current invariant run executor.
    executor: Executor<FEN>,
    // Invariant run stat reports (eg. gas usage).
    fuzz_runs: Vec<FuzzCase>,
    // Contracts created during current invariant run.
    created_contracts: Vec<Address>,
    // Traces of each call of the invariant run call sequence.
    run_traces: Vec<SparsedTraceArena>,
    // Current depth of invariant run.
    depth: u32,
    // Current assume rejects of the invariant run.
    rejects: u32,
    // Whether new coverage was discovered during this run.
    new_coverage: bool,
    // For optimization mode: the best value found during this run (if any).
    optimization_value: Option<I256>,
    // For optimization mode: the length of the input prefix that produced the best value.
    optimization_prefix_len: usize,
}

/// Immutable state selected once for a logical invariant campaign and cloned into each worker.
#[derive(Clone)]
struct InvariantCampaignSeed {
    artifact_filters: ArtifactFilters,
    sender_filters: SenderFilters,
    targeted_contracts: TargetedContracts,
    targets_are_updatable: bool,
    initial_handler_failures: Map<(Address, Selector), InvariantFuzzError>,
}

impl<FEN: FoundryEvmNetwork> InvariantTestRun<FEN> {
    /// Instantiates an invariant test run.
    fn new(first_input: BasicTxDetails, executor: Executor<FEN>, depth: usize) -> Self {
        Self {
            inputs: vec![first_input],
            cmp_seq: Vec::with_capacity(depth),
            executor,
            fuzz_runs: Vec::with_capacity(depth),
            created_contracts: vec![],
            run_traces: vec![],
            depth: 0,
            rejects: 0,
            new_coverage: false,
            optimization_value: None,
            optimization_prefix_len: 0,
        }
    }

    /// Releases per-run corpus payloads once the worker corpus manager has consumed them.
    ///
    /// Successful runs only need `fuzz_runs`, traces, and created-contract bookkeeping for final
    /// reporting. Counterexample inputs are copied into `InvariantTestData::last_run_inputs`
    /// before this point, so retaining the full per-run input/cmp buffers until `end_run` only
    /// extends peak memory in long invariant campaigns.
    fn drop_corpus_payloads(&mut self) {
        self.inputs.clear();
        self.inputs.shrink_to_fit();
        self.cmp_seq.clear();
        self.cmp_seq.shrink_to_fit();
    }
}

/// Wrapper around any [`Executor`] implementer which provides fuzzing support using [`proptest`].
///
/// After instantiation, calling `invariant_fuzz` will proceed to hammer the deployed smart
/// contracts with inputs, until it finds a counterexample sequence. The provided [`TestRunner`]
/// contains all the configuration which can be overridden via [environment
/// variables](proptest::test_runner::Config)
pub struct InvariantExecutor<'a, FEN: FoundryEvmNetwork> {
    pub executor: Executor<FEN>,
    /// Proptest runner.
    runner: TestRunner,
    /// Configured fuzz seed used to derive deterministic invariant worker runners.
    fuzz_seed: Option<U256>,
    /// The invariant configuration
    config: InvariantConfig,
    /// Contracts deployed with `setUp()`
    setup_contracts: &'a ContractsByAddress,
    /// Contracts that are part of the project but have not been deployed yet. We need the bytecode
    /// to identify them from the stateset changes.
    project_contracts: &'a ContractsByArtifact,
    /// Filters contracts to be fuzzed through their artifact identifiers.
    artifact_filters: ArtifactFilters,
    /// Number of matching invariant campaign anchors in the current test pass.
    invariant_campaign_anchors: usize,
}

impl<'a, FEN: FoundryEvmNetwork> InvariantExecutor<'a, FEN> {
    /// Instantiates a fuzzed executor EVM given a testrunner
    pub fn new(
        executor: Executor<FEN>,
        runner: TestRunner,
        config: InvariantConfig,
        setup_contracts: &'a ContractsByAddress,
        project_contracts: &'a ContractsByArtifact,
    ) -> Self {
        Self::new_with_fuzz_seed(
            executor,
            runner,
            None,
            config,
            setup_contracts,
            project_contracts,
            1,
        )
    }

    /// Instantiates an invariant executor with the configured fuzz seed for deterministic worker
    /// runner derivation.
    pub fn new_with_fuzz_seed(
        executor: Executor<FEN>,
        runner: TestRunner,
        fuzz_seed: Option<U256>,
        config: InvariantConfig,
        setup_contracts: &'a ContractsByAddress,
        project_contracts: &'a ContractsByArtifact,
        invariant_campaign_anchors: usize,
    ) -> Self {
        Self {
            executor,
            runner,
            fuzz_seed,
            config,
            setup_contracts,
            project_contracts,
            artifact_filters: ArtifactFilters::default(),
            invariant_campaign_anchors,
        }
    }

    pub fn config(&self) -> InvariantConfig {
        self.config.clone()
    }

    /// Refs for tracking contracts deployed mid-sequence during corpus replay.
    pub const fn dynamic_target_ctx(&self) -> DynamicTargetCtx<'_> {
        DynamicTargetCtx {
            project_contracts: self.project_contracts,
            setup_contracts: self.setup_contracts,
            artifact_filters: &self.artifact_filters,
        }
    }

    /// Fuzzes any deployed contract and checks any broken invariant at `invariant_address`.
    ///
    /// `initial_handler_failures` pre-seeds the campaign's `broken_handlers` map with bugs
    /// recovered from disk by the runner's persisted-failure replay step, so the live
    /// progress bar and JSON pulse stream surface them from the first emission instead of
    /// jumping at the final report.
    pub fn invariant_fuzz(
        &mut self,
        invariant_contract: InvariantContract<'_>,
        fuzz_fixtures: &FuzzFixtures,
        fuzz_state: EvmFuzzState,
        progress: Option<&ProgressBar>,
        early_exit: &EarlyExit,
        initial_handler_failures: std::collections::HashMap<
            (Address, Selector),
            InvariantFuzzError,
        >,
    ) -> Result<InvariantFuzzTestResult> {
        let campaign_spec = InvariantCampaignSpec::new(self.config.runs);
        let worker_plans = campaign_spec.worker_plans(invariant_worker_count_with_threads(
            &self.config,
            rayon::current_num_threads(),
            self.invariant_campaign_anchors,
        ))?;
        let actual_worker_count = worker_plans.len();
        let campaign_seed =
            self.prepare_campaign_seed(&invariant_contract, initial_handler_failures)?;
        let replay_targets = FuzzRunIdentifiedContracts::new(
            campaign_seed.targeted_contracts.clone(),
            campaign_seed.targets_are_updatable,
        );
        let mut corpus_replay_executor = self.executor.clone();
        corpus_replay_executor.inspector_mut().collect_evm_cmp_log(
            invariant_worker_collects_evm_cmp_log(&self.config, 0, actual_worker_count),
        );
        let corpus_seed = WorkerCorpusSeed::load_from_disk(
            &self.config.corpus,
            Some(&corpus_replay_executor),
            None,
            Some(&replay_targets),
            Some(self.dynamic_target_ctx()),
        )?;
        let corpus_persistence = if actual_worker_count > 1 {
            InvariantCorpusPersistence::Deferred
        } else {
            InvariantCorpusPersistence::Live
        };
        let mut runner = self.runner.clone();
        let config = self.config.clone();
        let setup_contracts = self.setup_contracts;
        let project_contracts = self.project_contracts;
        let base_executor = self.executor.clone();
        let campaign_state =
            Arc::new(InvariantCampaignState::new(early_exit.clone(), self.config.timeout));

        let worker_outputs = if corpus_persistence.is_deferred() {
            let worker_jobs = worker_plans
                .into_iter()
                .map(|worker_plan| {
                    let worker_runner =
                        invariant_worker_runner(&mut runner, worker_plan.worker_id, self.fuzz_seed);
                    let gas_report_samples = gas_report_samples_for_worker(
                        config.gas_report_samples,
                        worker_plan.worker_id,
                        actual_worker_count,
                    );
                    let collect_cmp_log = invariant_worker_collects_evm_cmp_log(
                        &config,
                        worker_plan.worker_id,
                        actual_worker_count,
                    );
                    (worker_plan, worker_runner, gas_report_samples, collect_cmp_log)
                })
                .collect::<Vec<_>>();
            worker_jobs
                .into_par_iter()
                .map(|(worker_plan, worker_runner, gas_report_samples, collect_cmp_log)| {
                    let _guard =
                        info_span!("invariant_worker", id = worker_plan.worker_id).entered();
                    let timer = Instant::now();
                    let output = Self::run_invariant_worker(
                        base_executor.clone(),
                        worker_runner,
                        config.clone(),
                        setup_contracts,
                        project_contracts,
                        worker_plan,
                        invariant_contract.clone(),
                        fuzz_fixtures,
                        fuzz_state.fork(),
                        progress,
                        &campaign_state,
                        campaign_seed.clone(),
                        corpus_seed.clone_for_worker(
                            worker_plan.worker_id as usize,
                            actual_worker_count,
                            collect_cmp_log,
                        ),
                        corpus_persistence,
                        actual_worker_count,
                        gas_report_samples,
                    );
                    debug!("finished in {:?}", timer.elapsed());
                    output
                })
                .collect::<Result<Vec<_>>>()?
        } else {
            let worker_plan = worker_plans[0];
            let runner =
                invariant_worker_runner(&mut runner, worker_plan.worker_id, self.fuzz_seed);
            let gas_report_samples = config.gas_report_samples as usize;
            let collect_cmp_log = invariant_worker_collects_evm_cmp_log(
                &config,
                worker_plan.worker_id,
                actual_worker_count,
            );
            vec![Self::run_invariant_worker(
                base_executor,
                runner,
                config,
                setup_contracts,
                project_contracts,
                worker_plan,
                invariant_contract,
                fuzz_fixtures,
                fuzz_state,
                progress,
                &campaign_state,
                campaign_seed,
                corpus_seed.clone_for_worker(
                    worker_plan.worker_id as usize,
                    actual_worker_count,
                    collect_cmp_log,
                ),
                corpus_persistence,
                actual_worker_count,
                gas_report_samples,
            )?]
        };

        let mut aggregator = InvariantCampaignAggregator::new(campaign_spec);
        for worker_output in worker_outputs {
            aggregator.push(worker_output);
        }
        let (result, corpus_entries) = if campaign_state.is_timed_campaign() {
            aggregator.finish_partial_with_corpus_entries()?
        } else {
            aggregator.finish_with_corpus_entries()?
        };
        if corpus_persistence.is_deferred() {
            let dynamic_target_ctx = self.dynamic_target_ctx();
            corpus_seed.persist_filtered_campaign_outputs(
                &self.config.corpus,
                corpus_entries,
                &self.executor,
                ReplayTarget {
                    fuzzed_function: None,
                    fuzzed_contracts: Some(&replay_targets),
                    dynamic: Some(&dynamic_target_ctx),
                },
                result
                    .optimization_best_value
                    .map(|value| (value, result.optimization_best_sequence.as_slice())),
            )?;
        }
        Ok(result)
    }

    /// Runs one worker-local slice of an invariant campaign.
    #[allow(clippy::too_many_arguments)]
    fn run_invariant_worker(
        mut executor: Executor<FEN>,
        runner: TestRunner,
        config: InvariantConfig,
        setup_contracts: &'a ContractsByAddress,
        project_contracts: &'a ContractsByArtifact,
        plan: InvariantWorkerPlan,
        invariant_contract: InvariantContract<'_>,
        fuzz_fixtures: &FuzzFixtures,
        fuzz_state: EvmFuzzState,
        progress: Option<&ProgressBar>,
        campaign_state: &InvariantCampaignState,
        campaign_seed: InvariantCampaignSeed,
        corpus_seed: WorkerCorpusSeed,
        corpus_persistence: InvariantCorpusPersistence,
        worker_count: usize,
        gas_report_samples: usize,
    ) -> Result<InvariantWorkerOutput> {
        // Note: invariant function signatures (no inputs) are validated upstream in the
        // suite runner so parameterized `invariant_*` functions are rejected with a per-test
        // failure entry before any campaign runs.

        let (mut invariant_test, mut corpus_manager) = Self::prepare_worker(
            &mut executor,
            plan,
            worker_count,
            &invariant_contract,
            fuzz_fixtures,
            fuzz_state,
            &runner,
            &config,
            &campaign_seed,
            corpus_seed,
        )?;
        let mut corpus_entries = Vec::new();

        let mut runs = 0;
        campaign_state.sync_handler_failures(&invariant_test.test_data.failures);

        // Invariant runs with edge coverage if corpus dir is set or showing edge coverage.
        let edge_coverage_enabled = config.corpus.collect_edge_coverage();

        'stop: while should_continue_invariant_worker(campaign_state, runs, plan) {
            // Per-run failure count snapshot used to gate `afterInvariant` below.
            let failures_before_run = invariant_test.test_data.failures.invariant_count();
            let mut stop_after_run = false;

            let initial_seq = corpus_manager.new_inputs(
                &mut invariant_test.test_data.branch_runner,
                &invariant_test.fuzz_state,
                &invariant_test.targeted_contracts,
            )?;

            // Create current invariant run data.
            let mut current_run = InvariantTestRun::new(
                initial_seq[0].clone(),
                // Before each run, we must reset the backend state.
                executor.clone(),
                config.depth as usize,
            );

            // We stop the run immediately if we have reverted, and `fail_on_revert` is set.
            if config.fail_on_revert && invariant_test.reverts() > 0 {
                campaign_state.request_terminal_stop();
                return Err(eyre!("call reverted"));
            }

            while current_run.depth < config.depth {
                // Check if the timeout has been reached.
                if campaign_state.should_stop() {
                    // Since we never record a revert here the test is still considered
                    // successful even though it timed out. We *want*
                    // this behavior for now, so that's ok, but
                    // future developers should be aware of this.
                    break 'stop;
                }

                // Snapshot `(target, selector)` so `can_continue` can borrow `&mut current_run`
                // later without cloning the full `BasicTxDetails`.
                let (handler_target, handler_selector) = {
                    let last = current_run
                        .inputs
                        .last()
                        .ok_or_else(|| eyre!("no input generated to call fuzzed target."))?;
                    let sel_bytes: [u8; 4] = last
                        .call_details
                        .calldata
                        .get(..4)
                        .and_then(|s| s.try_into().ok())
                        .unwrap_or_default();
                    (last.call_details.target, Selector::from(sel_bytes))
                };

                // Execute call from the randomly generated sequence without committing state.
                // State is committed only if call is not a magic assume.
                let mut call_result = execute_tx(
                    &mut current_run.executor,
                    current_run.inputs.last().expect("checked above"),
                )?;
                if let Some(fuzzer) = current_run.executor.inspector_mut().fuzzer.as_mut() {
                    invariant_test.fuzz_state.collect_values(fuzzer.drain_collected_values());
                }
                // Capture per-call EVM cmp operands for I2S corpus mutation. Kept parallel
                // to `current_run.inputs`; populated unconditionally so dropped calls (magic
                // assumes / pops below) get zero-length entries that the corpus side filters out.
                let call_cmp_values = call_result.evm_cmp_values.take().unwrap_or_default();
                let discarded = call_result.result.as_ref() == MAGIC_ASSUME;
                if config.show_metrics {
                    invariant_test.record_metrics(
                        current_run.inputs.last().expect("checked above"),
                        call_result.reverted,
                        discarded,
                    );
                }

                // Collect line coverage from last fuzzed call.
                invariant_test.merge_line_coverage(call_result.line_coverage.clone());
                // Snapshot the edge fingerprint before `merge_edge_coverage` zeroes the
                // buffer. Gate on `assertion_failure` to skip keccak on plain reverts.
                let assertion_failure =
                    !discarded && did_fail_on_assert(&call_result, &call_result.state_changeset);
                let pre_merge_edges_hash = if assertion_failure {
                    error::snapshot_edge_fingerprint(&call_result)
                } else {
                    None
                };
                // Collect edge coverage and set the flag in the current run.
                let new_call_coverage = corpus_manager.merge_edge_coverage(&mut call_result);
                if new_call_coverage {
                    current_run.new_coverage = true;
                }
                let observed_calls = std::mem::take(&mut call_result.observed_calls);
                if new_call_coverage
                    && let Some(entry) = corpus_manager.hoist_observed_calls(
                        &observed_calls,
                        current_run.inputs.last().expect("checked above"),
                        &invariant_test.targeted_contracts,
                        if corpus_persistence.is_deferred() {
                            CorpusInsertionMode::Deferred
                        } else {
                            CorpusInsertionMode::Live
                        },
                    )
                {
                    corpus_entries.push(entry);
                }

                if discarded {
                    current_run.inputs.pop();
                    current_run.rejects += 1;
                    if current_run.rejects > config.max_assume_rejects {
                        invariant_test.set_error(
                            invariant_contract.anchor(),
                            InvariantFuzzError::MaxAssumeRejects(config.max_assume_rejects),
                        );
                        campaign_state.request_terminal_stop();
                        break 'stop;
                    }
                } else {
                    // Commit executed call result.
                    current_run.executor.commit(&mut call_result);

                    // Collect data for fuzzing from the state changeset.
                    // This step updates the state dictionary and therefore invalidates the
                    // ValueTree in use by the current run. This manifestsitself in proptest
                    // observing a different input case than what it was called with, and creates
                    // inconsistencies whenever proptest tries to use the input case after test
                    // execution.
                    // See <https://github.com/foundry-rs/foundry/issues/9764>.
                    let mut state_changeset = std::mem::take(&mut call_result.state_changeset);
                    if !call_result.reverted {
                        let mapping_slots = current_run
                            .executor
                            .inspector()
                            .fuzzer
                            .as_ref()
                            .and_then(|fuzzer| fuzzer.mapping_slots.as_ref());
                        collect_data(
                            &invariant_test,
                            &mut state_changeset,
                            current_run.inputs.last().expect("checked above"),
                            &call_result,
                            config.depth,
                            mapping_slots,
                        );
                    }

                    // Collect created contracts and add to fuzz targets only if targeted contracts
                    // are updatable.
                    if let Err(error) =
                        &invariant_test.targeted_contracts.collect_created_contracts(
                            &state_changeset,
                            project_contracts,
                            setup_contracts,
                            &campaign_seed.artifact_filters,
                            &mut current_run.created_contracts,
                        )
                    {
                        warn!(target: "forge::test", "{error}");
                    }
                    current_run
                        .fuzz_runs
                        .push(FuzzCase { gas: call_result.gas_used, stipend: call_result.stipend });
                    campaign_state.record_call(call_result.gas_used);

                    // Determine if test can continue or should exit.
                    // Check invariants based on check_interval to improve deep run performance.
                    // - check_interval=0: only assert on the last call
                    // - check_interval=1 (default): assert after every call
                    // - check_interval=N: assert every N calls AND always on the last call
                    let is_last_call = current_run.depth == config.depth - 1;
                    // In optimization mode, always evaluate the invariant to track
                    // the best value at every prefix — check_interval only gates
                    // boolean invariant assertions.
                    let is_optimization = invariant_contract.is_optimization();
                    let should_check_invariant = is_optimization
                        || if config.check_interval == 0 {
                            is_last_call
                        } else {
                            config.check_interval == 1
                                || (current_run.depth + 1).is_multiple_of(config.check_interval)
                                || is_last_call
                        };

                    let errors_before_check = invariant_test.test_data.failures.invariant_count();
                    let (continues, broken) = if should_check_invariant {
                        let outcome = can_continue(
                            &invariant_contract,
                            &mut invariant_test,
                            &mut current_run,
                            &config,
                            call_result,
                            &state_changeset,
                            handler_target,
                            handler_selector,
                            pre_merge_edges_hash,
                        )
                        .map_err(|e| eyre!(e.to_string()))?;
                        (outcome.continues, outcome.broken)
                    } else {
                        // Skip invariant check but still track reverts
                        if call_result.reverted {
                            invariant_test.test_data.failures.reverts += 1;
                        }
                        if assertion_failure {
                            // Handler-side assertion: deduped by `(reverter, selector)` site;
                            // campaign keeps running to surface more bugs.
                            let call_reverted = call_result.reverted;
                            error::record_handler_assertion_bug(
                                &invariant_contract,
                                &config,
                                &invariant_test.targeted_contracts,
                                &mut invariant_test.test_data.failures,
                                &mut current_run.inputs,
                                handler_target,
                                handler_selector,
                                pre_merge_edges_hash,
                                call_result,
                                call_reverted,
                                invariant_contract.is_optimization(),
                            );
                            (true, None)
                        } else if call_result.reverted && config.fail_on_revert {
                            // Plain revert under fail_on_revert: attribute to the anchor.
                            let anchor = invariant_contract.anchor();
                            let case_data = error::InvariantRunCtx {
                                contract: &invariant_contract,
                                config: &config,
                                targeted_contracts: &invariant_test.targeted_contracts,
                                calldata: &current_run.inputs,
                            }
                            .failed_case(
                                anchor,
                                config.fail_on_revert,
                                false,
                                call_result,
                                &[],
                            );
                            invariant_test
                                .test_data
                                .failures
                                .record_failure(anchor, InvariantFuzzError::Revert(case_data));
                            (false, Some(anchor))
                        } else if call_result.reverted
                            && !invariant_contract.is_optimization()
                            && !config.has_delay()
                        {
                            // Delay campaigns keep reverted calls so warp/roll survives shrinking.
                            current_run.inputs.pop();
                            (true, None)
                        } else {
                            (true, None)
                        }
                    };

                    // Keep `cmp_seq` parallel to `inputs`: only push when the input survived the
                    // pop branch above.
                    if current_run.cmp_seq.len() < current_run.inputs.len() {
                        current_run.cmp_seq.push(call_cmp_values);
                    }

                    if !continues || current_run.depth == config.depth - 1 {
                        invariant_test.set_last_run_inputs(&current_run.inputs);
                    }
                    // Bridge newly-recorded predicate breaks into `failure_metrics` even when
                    // `continues == true` in multi-predicate campaigns.
                    if invariant_test.test_data.failures.invariant_count() > errors_before_check
                        || broken.is_some()
                    {
                        record_new_invariant_failures(
                            campaign_state,
                            &invariant_contract,
                            &invariant_test.test_data.failures,
                        );
                    }
                    if !continues {
                        if invariant_contract.invariant_fns.len() > 1 && !config.fail_on_revert {
                            break;
                        }
                        campaign_state.request_terminal_stop();
                        stop_after_run = true;
                        break;
                    }
                    current_run.depth += 1;
                }

                current_run.inputs.push(corpus_manager.generate_next_input(
                    &mut invariant_test.test_data.branch_runner,
                    &initial_seq,
                    discarded,
                    current_run.depth as usize,
                )?);
            }

            // Extend corpus with current run data.
            // Materialize the optimization best prefix once at run end (avoids
            // cloning inputs on every new in-run max).
            let optimization = current_run.optimization_value.map(|v| {
                let prefix = current_run.inputs[..current_run.optimization_prefix_len].to_vec();
                (v, prefix)
            });
            if corpus_persistence.is_deferred() {
                if let Some(input) = corpus_manager.process_inputs_for_campaign(
                    &current_run.inputs,
                    &current_run.cmp_seq,
                    current_run.new_coverage,
                    optimization,
                ) {
                    corpus_entries.push(input);
                }
            } else {
                corpus_manager.process_inputs(
                    &current_run.inputs,
                    &current_run.cmp_seq,
                    current_run.new_coverage,
                    optimization,
                );
            }

            // Call `afterInvariant` only if declared and the current run produced no new
            // failure. Multi-predicate campaigns keep running after earlier failures, but the
            // hook must still execute on subsequent runs.
            if invariant_contract.call_after_invariant
                && invariant_test.test_data.failures.invariant_count() == failures_before_run
            {
                let broken = assert_after_invariant(
                    &invariant_contract,
                    &mut invariant_test,
                    &current_run,
                    &config,
                )
                .map_err(|_| eyre!("Failed to call afterInvariant"))?;
                if broken.is_some() {
                    // Bridge breaks into pulse metrics, mirroring the in-run path above.
                    record_new_invariant_failures(
                        campaign_state,
                        &invariant_contract,
                        &invariant_test.test_data.failures,
                    );
                }
            }

            // End current invariant test run.
            current_run.drop_corpus_payloads();
            invariant_test.end_run(current_run, gas_report_samples);
            runs += 1;
            let total_runs = campaign_state.increment_runs();
            debug_assert!(
                campaign_state.is_timed_campaign() || total_runs <= config.runs,
                "worker runs were not distributed correctly"
            );
            if let Some(progress) = progress {
                progress.inc(1);
                campaign_state.sync_handler_failures(&invariant_test.test_data.failures);
                // Display current best value, corpus metrics, and failure counts.
                let best = invariant_test.test_data.optimization_best_value;
                let failure_metrics = campaign_state.failure_metrics();
                let broken = failure_metrics.unique_failures.len();
                let handler_bugs = failure_metrics.broken_handlers;
                let total_invariants = invariant_contract.invariant_fns.len();
                if edge_coverage_enabled || best.is_some() || broken > 0 || handler_bugs > 0 {
                    let mut msg = String::new();
                    if let Some(best) = best {
                        msg.push_str(&format!("best: {best}"));
                    }
                    if edge_coverage_enabled {
                        if !msg.is_empty() {
                            msg.push_str(", ");
                        }
                        msg.push_str(&format!("{}", corpus_manager.metrics));
                    }
                    if broken > 0 {
                        if !msg.is_empty() {
                            msg.push_str(", ");
                        }
                        msg.push_str(&format!("❌ {broken}/{total_invariants} broken"));
                    }
                    if handler_bugs > 0 {
                        if !msg.is_empty() {
                            msg.push_str(", ");
                        }
                        msg.push_str(&format!("⚠ {handler_bugs} handler bug(s)"));
                    }
                    let msg = if corpus_persistence.is_deferred() {
                        format!("[w{}] {msg}", plan.worker_id)
                    } else {
                        msg
                    };
                    progress.set_message(msg);
                }
            } else if edge_coverage_enabled
                && campaign_state.should_emit_metrics_report(DURATION_BETWEEN_METRICS_REPORT)
            {
                campaign_state.sync_handler_failures(&invariant_test.test_data.failures);
                let failure_metrics = campaign_state.failure_metrics();
                let (total_txs, total_gas) = campaign_state.throughput_totals();
                let throughput = InvariantThroughputMetrics { total_txs, total_gas };
                // Display corpus metrics inline as JSON.
                let metrics = build_invariant_progress_json(
                    InvariantProgressContext {
                        timestamp_secs: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
                        contract_name: invariant_contract.name,
                        optimization_best: invariant_test.test_data.optimization_best_value,
                        throughput,
                        elapsed: campaign_state.elapsed(),
                        worker_id: plan.worker_id,
                        worker_count,
                    },
                    &corpus_manager.metrics,
                    &failure_metrics,
                );
                let _ = sh_println!("{}", serde_json::to_string(&metrics)?);
            }

            if stop_after_run {
                break 'stop;
            }
        }

        trace!(?fuzz_fixtures);
        invariant_test.fuzz_state.log_stats();

        Self::shrink_handler_failures(
            &config,
            &executor,
            &mut invariant_test.test_data,
            progress,
            campaign_state.early_exit(),
        );

        // Move out the final test data and drop worker-local fuzz state before returning this
        // worker's aggregate output. Long invariant campaigns can leave large dictionaries and
        // target state behind; once shrinking is complete, only `test_data` is needed.
        let InvariantTest { fuzz_state: _, targeted_contracts: _, test_data: result } =
            invariant_test;
        let reverts = result.failures.reverts;
        let (errors, handler_errors) = result.failures.partition();
        let worker_result = InvariantFuzzTestResult::new(
            errors,
            handler_errors,
            result.runs,
            result.calls,
            reverts,
            result.last_run_inputs,
            result.gas_report_traces,
            result.line_coverage,
            result.metrics,
            if plan.worker_id == 0 { corpus_manager.failed_replays } else { 0 },
            1,
            result.optimization_best_value,
            result.optimization_best_sequence,
        );
        drop(corpus_manager);
        let reported_plan = if campaign_state.is_timed_campaign() {
            InvariantWorkerPlan { runs, ..plan }
        } else {
            // Sharded campaigns must report the original assigned range. Early worker exit changes
            // the number of executed runs, but it must not shrink `plan.runs`: following workers'
            // `first_global_run` offsets were computed from the original partition.
            plan
        };
        Ok(InvariantWorkerOutput { plan: reported_plan, result: worker_result, corpus_entries })
    }

    fn shrink_handler_failures(
        config: &InvariantConfig,
        executor: &Executor<FEN>,
        result: &mut InvariantTestData,
        progress: Option<&ProgressBar>,
        early_exit: &EarlyExit,
    ) {
        let total = result.failures.handler_count();
        if total == 0 {
            return;
        }

        for (idx, error) in result.failures.handler_failures_mut().enumerate() {
            if early_exit.should_stop() {
                break;
            }
            let Some(failure) = error.as_handler_assertion_mut() else {
                continue;
            };
            shrink::reset_shrink_progress(
                config,
                progress,
                &format!("handler {:#x}::{}", failure.reverter, failure.selector),
                Some((idx + 1, total)),
            );
            match shrink::shrink_handler_sequence(
                config,
                &failure.call_sequence,
                failure.edge_fingerprint,
                executor,
                progress,
                early_exit,
            ) {
                Ok(shrunk) if !shrunk.is_empty() => {
                    failure.call_sequence = shrunk;
                }
                Ok(_) => {}
                Err(e) => trace!(target: "forge::test", "handler shrink failed: {e}"),
            }
        }
    }

    fn prepare_campaign_seed(
        &mut self,
        invariant_contract: &InvariantContract<'_>,
        initial_handler_failures: std::collections::HashMap<
            (Address, Selector),
            InvariantFuzzError,
        >,
    ) -> Result<InvariantCampaignSeed> {
        self.select_contract_artifacts(invariant_contract.address)?;
        let (sender_filters, targeted_contracts) =
            self.select_contracts_and_senders(invariant_contract.address)?;
        let targets_are_updatable = targeted_contracts.is_updatable;
        let targeted_contracts = targeted_contracts.targets().clone();

        Ok(InvariantCampaignSeed {
            artifact_filters: self.artifact_filters.clone(),
            sender_filters,
            targeted_contracts,
            targets_are_updatable,
            initial_handler_failures,
        })
    }

    /// Prepares worker-local structures to execute an invariant campaign slice.
    #[allow(clippy::too_many_arguments)]
    fn prepare_worker(
        executor: &mut Executor<FEN>,
        plan: InvariantWorkerPlan,
        worker_count: usize,
        invariant_contract: &InvariantContract<'_>,
        fuzz_fixtures: &FuzzFixtures,
        fuzz_state: EvmFuzzState,
        runner: &TestRunner,
        config: &InvariantConfig,
        campaign_seed: &InvariantCampaignSeed,
        corpus_seed: WorkerCorpusSeed,
    ) -> Result<(InvariantTest, WorkerCorpus)> {
        let fuzz_state = fuzz_state.into_invariant();
        let targeted_contracts = FuzzRunIdentifiedContracts::new(
            campaign_seed.targeted_contracts.clone(),
            campaign_seed.targets_are_updatable,
        );
        executor.inspector_mut().collect_evm_cmp_log(invariant_worker_collects_evm_cmp_log(
            config,
            plan.worker_id,
            worker_count,
        ));

        // Creates the invariant strategy.
        let strategy = invariant_strat(
            fuzz_state.clone(),
            campaign_seed.sender_filters.clone(),
            targeted_contracts.clone(),
            config.clone(),
            fuzz_fixtures.clone(),
        )
        .no_shrink();

        // If any of the targeted contracts have the storage layout enabled then we can sample
        // mapping values. To accomplish, we need to record the mapping storage slots and keys.
        let mapping_slots = targeted_contracts
            .targets()
            .iter()
            .any(|(_, t)| t.storage_layout.is_some())
            .then(AddressMap::default);

        // Set up fuzzer WITHOUT call_generator initially.
        // We defer call_override until after the initial invariant check to avoid
        // injecting random calls during setup which would break the invariant assertion.
        executor.inspector_mut().set_fuzzer(
            Fuzzer::new(config.dictionary.max_fuzz_dictionary_values, mapping_slots)
                .with_call_recording(config.corpus.is_coverage_guided()),
        );

        // Let's make sure the invariant is sound before actually starting the run:
        // We'll assert the invariant in its initial state, and if it fails, we'll
        // already know if we can early exit the invariant run.
        // This does not count as a fuzz run. It will just register the revert.
        let mut failures = InvariantFailures::new();
        // Seed disk-recovered handler bugs so live counters reflect them from tick 0.
        for (&(addr, sel), err) in &campaign_seed.initial_handler_failures {
            failures.seed_handler_failure(addr, sel, err.clone());
        }
        invariant_preflight_check(
            invariant_contract,
            config,
            &targeted_contracts,
            executor,
            &[],
            &mut failures,
        )?;
        if let Some(fuzzer) = executor.inspector_mut().fuzzer.as_mut() {
            fuzz_state.collect_values(fuzzer.drain_collected_values());
            let _ = fuzzer.take_observed_calls();
        }
        let mut worker = WorkerCorpus::from_seed(
            plan.worker_id as usize,
            config.corpus.clone(),
            strategy.boxed(),
            corpus_seed,
        );

        if let Err(err) =
            worker.seed_from_test_traces(invariant_contract, &targeted_contracts, executor)
        {
            debug!(target: "corpus", %err, "failed to seed corpus from test traces");
        }

        // NOW enable call_override after the initial invariant check and corpus trace seeding have
        // passed. This allows `override_call_strat` to inject calls during actual fuzz runs for
        // reentrancy vulnerability detection.
        if config.call_override {
            let target_contract_ref = Arc::new(RwLock::new(Address::ZERO));

            // Collect handler addresses - these are the contracts we want to inject
            // reentrancy into (simulating malicious receive() functions).
            let handler_addresses: std::collections::HashSet<Address> =
                targeted_contracts.targets().keys().copied().collect();
            let override_targets = targeted_contracts
                .targets()
                .iter()
                .filter_map(|(address, contract)| {
                    let functions = contract.abi_fuzzed_functions().cloned().collect::<Vec<_>>();
                    (!functions.is_empty()).then_some((*address, functions))
                })
                .collect::<Vec<_>>();

            let call_generator = RandomCallGenerator::new(
                invariant_contract.address,
                handler_addresses,
                runner.clone(),
                override_call_strat(
                    fuzz_state.snapshot(),
                    override_targets,
                    target_contract_ref.clone(),
                    fuzz_fixtures.clone(),
                ),
                target_contract_ref,
            );

            if let Some(fuzzer) = executor.inspector_mut().fuzzer.as_mut() {
                fuzzer.call_generator = Some(call_generator);
            }
        }

        let mut invariant_test =
            InvariantTest::new(fuzz_state, targeted_contracts, failures, runner.clone());

        // Seed invariant test with previously persisted optimization state,
        // but only if the current invariant is in optimization mode. Persisted optimization state
        // is a master-worker artifact loaded with the initial corpus.
        if invariant_contract.is_optimization() {
            let (opt_best_value, opt_best_sequence) = worker.optimization_initial_state();
            if let Some(value) = opt_best_value {
                invariant_test.update_optimization_value(value, &opt_best_sequence);
            }
        }

        Ok((invariant_test, worker))
    }

    /// Fills the `InvariantExecutor` with the artifact identifier filters (in `path:name` string
    /// format). They will be used to filter contracts after the `setUp`, and more importantly,
    /// during the runs.
    ///
    /// Also excludes any contract without any mutable functions.
    ///
    /// Priority:
    ///
    /// targetArtifactSelectors > excludeArtifacts > targetArtifacts
    pub fn select_contract_artifacts(&mut self, invariant_address: Address) -> Result<()> {
        let targeted_artifact_selectors = self
            .executor
            .call_sol_default(invariant_address, &IInvariantTest::targetArtifactSelectorsCall {});

        // Insert them into the executor `targeted_abi`.
        for IInvariantTest::FuzzArtifactSelector { artifact, selectors } in
            targeted_artifact_selectors
        {
            let identifier = self.validate_selected_contract(artifact, &selectors)?;
            self.artifact_filters.targeted.entry(identifier).or_default().extend(selectors);
        }

        let targeted_artifacts = self
            .executor
            .call_sol_default(invariant_address, &IInvariantTest::targetArtifactsCall {});
        let excluded_artifacts = self
            .executor
            .call_sol_default(invariant_address, &IInvariantTest::excludeArtifactsCall {});

        // Insert `excludeArtifacts` into the executor `excluded_abi`.
        for contract in excluded_artifacts {
            let identifier = self.validate_selected_contract(contract, &[])?;

            if !self.artifact_filters.excluded.contains(&identifier) {
                self.artifact_filters.excluded.push(identifier);
            }
        }

        // Exclude any artifact without mutable functions.
        for (artifact, contract) in self.project_contracts.iter() {
            if contract
                .abi
                .functions()
                .filter(|func| {
                    !matches!(
                        func.state_mutability,
                        alloy_json_abi::StateMutability::Pure
                            | alloy_json_abi::StateMutability::View
                    )
                })
                .count()
                == 0
                && !self.artifact_filters.excluded.contains(&artifact.identifier())
            {
                self.artifact_filters.excluded.push(artifact.identifier());
            }
        }

        // Insert `targetArtifacts` into the executor `targeted_abi`, if they have not been seen
        // before.
        for contract in targeted_artifacts {
            let identifier = self.validate_selected_contract(contract, &[])?;

            if !self.artifact_filters.targeted.contains_key(&identifier)
                && !self.artifact_filters.excluded.contains(&identifier)
            {
                self.artifact_filters.targeted.insert(identifier, vec![]);
            }
        }
        Ok(())
    }

    /// Makes sure that the contract exists in the project. If so, it returns its artifact
    /// identifier.
    fn validate_selected_contract(
        &mut self,
        contract: String,
        selectors: &[FixedBytes<4>],
    ) -> Result<String> {
        if let Some((artifact, contract_data)) =
            self.project_contracts.find_by_name_or_identifier(&contract)?
        {
            // Check that the selectors really exist for this contract.
            for selector in selectors {
                contract_data
                    .abi
                    .functions()
                    .find(|func| func.selector().as_slice() == selector.as_slice())
                    .wrap_err(format!("{contract} does not have the selector {selector:?}"))?;
            }

            return Ok(artifact.identifier());
        }
        eyre::bail!(
            "{contract} not found in the project. Allowed format: `contract_name` or `contract_path:contract_name`."
        );
    }

    /// Selects senders and contracts based on the contract methods `targetSenders() -> address[]`,
    /// `targetContracts() -> address[]` and `excludeContracts() -> address[]`.
    pub fn select_contracts_and_senders(
        &self,
        to: Address,
    ) -> Result<(SenderFilters, FuzzRunIdentifiedContracts)> {
        let targeted_senders =
            self.executor.call_sol_default(to, &IInvariantTest::targetSendersCall {});
        let mut excluded_senders =
            self.executor.call_sol_default(to, &IInvariantTest::excludeSendersCall {});
        // Extend with default excluded addresses - https://github.com/foundry-rs/foundry/issues/4163
        excluded_senders.extend([
            CHEATCODE_ADDRESS,
            HARDHAT_CONSOLE_ADDRESS,
            DEFAULT_CREATE2_DEPLOYER,
        ]);
        // Extend with precompiles - https://github.com/foundry-rs/foundry/issues/4287
        excluded_senders.extend(PRECOMPILES);
        let sender_filters = SenderFilters::new(targeted_senders, excluded_senders);

        let selected = self.executor.call_sol_default(to, &IInvariantTest::targetContractsCall {});
        let excluded = self.executor.call_sol_default(to, &IInvariantTest::excludeContractsCall {});

        let contracts = self
            .setup_contracts
            .iter()
            .filter(|&(addr, (identifier, _))| {
                // Include to address if explicitly set as target.
                if *addr == to && selected.contains(&to) {
                    return true;
                }

                *addr != to
                    && *addr != CHEATCODE_ADDRESS
                    && *addr != HARDHAT_CONSOLE_ADDRESS
                    && (selected.is_empty() || selected.contains(addr))
                    && (excluded.is_empty() || !excluded.contains(addr))
                    && self.artifact_filters.matches(identifier)
            })
            .map(|(addr, (identifier, abi))| {
                (
                    *addr,
                    TargetedContract::new(identifier.clone(), abi.clone())
                        .with_project_contracts(self.project_contracts),
                )
            })
            .collect();
        let mut contracts = TargetedContracts { inner: contracts };

        self.target_interfaces(to, &mut contracts)?;

        self.select_selectors(to, &mut contracts)?;

        // There should be at least one contract identified as target for fuzz runs.
        if contracts.is_empty() {
            eyre::bail!("No contracts to fuzz.");
        }

        Ok((sender_filters, FuzzRunIdentifiedContracts::new(contracts, selected.is_empty())))
    }

    /// Extends the contracts and selectors to fuzz with the addresses and ABIs specified in
    /// `targetInterfaces() -> (address, string[])[]`. Enables targeting of addresses that are
    /// not deployed during `setUp` such as when fuzzing in a forked environment. Also enables
    /// targeting of delegate proxies and contracts deployed with `create` or `create2`.
    pub fn target_interfaces(
        &self,
        invariant_address: Address,
        targeted_contracts: &mut TargetedContracts,
    ) -> Result<()> {
        let interfaces = self
            .executor
            .call_sol_default(invariant_address, &IInvariantTest::targetInterfacesCall {});

        // Since `targetInterfaces` returns a tuple array there is no guarantee
        // that the addresses are unique this map is used to merge functions of
        // the specified interfaces for the same address. For example:
        // `[(addr1, ["IERC20", "IOwnable"])]` and `[(addr1, ["IERC20"]), (addr1, ("IOwnable"))]`
        // should be equivalent.
        let mut combined = TargetedContracts::new();

        // Loop through each address and its associated artifact identifiers.
        // We're borrowing here to avoid taking full ownership.
        for IInvariantTest::FuzzInterface { addr, artifacts } in &interfaces {
            // Identifiers are specified as an array, so we loop through them.
            for identifier in artifacts {
                // Try to find the contract by name or identifier in the project's contracts.
                if let Some((_, contract_data)) =
                    self.project_contracts.iter().find(|(artifact, _)| {
                        &artifact.name == identifier || &artifact.identifier() == identifier
                    })
                {
                    let abi = &contract_data.abi;
                    combined
                        // Check if there's an entry for the given key in the 'combined' map.
                        .entry(*addr)
                        // If the entry exists, extends its ABI with the function list.
                        .and_modify(|entry| {
                            // Extend the ABI's function list with the new functions.
                            entry.abi.functions.extend(abi.functions.clone());
                        })
                        // Otherwise insert it into the map.
                        .or_insert_with(|| {
                            let mut contract =
                                TargetedContract::new(identifier.clone(), abi.clone());
                            contract.storage_layout =
                                contract_data.storage_layout.as_ref().map(Arc::clone);
                            contract
                        });
                }
            }
        }

        targeted_contracts.extend(combined.inner);

        Ok(())
    }

    /// Selects the functions to fuzz based on the contract method `targetSelectors()` and
    /// `targetArtifactSelectors()`.
    pub fn select_selectors(
        &self,
        address: Address,
        targeted_contracts: &mut TargetedContracts,
    ) -> Result<()> {
        for (address, (identifier, _)) in self.setup_contracts {
            if let Some(selectors) = self.artifact_filters.targeted.get(identifier) {
                self.add_address_with_functions(*address, selectors, false, targeted_contracts)?;
            }
        }

        let mut target_test_selectors = vec![];
        let mut excluded_test_selectors = vec![];

        // Collect contract functions marked as target for fuzzing campaign.
        let selectors =
            self.executor.call_sol_default(address, &IInvariantTest::targetSelectorsCall {});
        for IInvariantTest::FuzzSelector { addr, selectors } in selectors {
            if addr == address {
                target_test_selectors = selectors.clone();
            }
            self.add_address_with_functions(addr, &selectors, false, targeted_contracts)?;
        }

        // Collect contract functions excluded from fuzzing campaign.
        let excluded_selectors =
            self.executor.call_sol_default(address, &IInvariantTest::excludeSelectorsCall {});
        for IInvariantTest::FuzzSelector { addr, selectors } in excluded_selectors {
            if addr == address {
                // If fuzz selector address is the test contract, then record selectors to be
                // later excluded if needed.
                excluded_test_selectors = selectors.clone();
            }
            self.add_address_with_functions(addr, &selectors, true, targeted_contracts)?;
        }

        if target_test_selectors.is_empty()
            && let Some(target) = targeted_contracts.get(&address)
        {
            // If test contract is marked as a target and no target selector explicitly set, then
            // include only state-changing functions that are not reserved and selectors that are
            // not explicitly excluded.
            let selectors: Vec<_> = target
                .abi
                .functions()
                .filter_map(|func| {
                    if matches!(
                        func.state_mutability,
                        alloy_json_abi::StateMutability::Pure
                            | alloy_json_abi::StateMutability::View
                    ) || func.is_reserved()
                        || excluded_test_selectors.contains(&func.selector())
                    {
                        None
                    } else {
                        Some(func.selector())
                    }
                })
                .collect();
            self.add_address_with_functions(address, &selectors, false, targeted_contracts)?;
        }

        Ok(())
    }

    /// Adds the address and fuzzed or excluded functions to `TargetedContracts`.
    fn add_address_with_functions(
        &self,
        address: Address,
        selectors: &[Selector],
        should_exclude: bool,
        targeted_contracts: &mut TargetedContracts,
    ) -> eyre::Result<()> {
        // Do not add address in target contracts if no function selected.
        if selectors.is_empty() {
            return Ok(());
        }

        let contract = match targeted_contracts.entry(address) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let (identifier, abi) = self.setup_contracts.get(&address).ok_or_else(|| {
                    eyre::eyre!(
                        "[{}] address does not have an associated contract: {}",
                        if should_exclude { "excludeSelectors" } else { "targetSelectors" },
                        address
                    )
                })?;
                entry.insert(
                    TargetedContract::new(identifier.clone(), abi.clone())
                        .with_project_contracts(self.project_contracts),
                )
            }
        };
        contract.add_selectors(selectors.iter().copied(), should_exclude)?;
        Ok(())
    }

    /// Computes the current invariant settings for the given invariant contract address.
    ///
    /// This extracts the target contracts, selectors, senders, and failure settings
    /// that are used to determine if a persisted counterexample is still valid.
    pub fn compute_settings(&mut self, invariant_address: Address) -> Result<InvariantSettings> {
        self.select_contract_artifacts(invariant_address)?;
        let (sender_filters, targeted_contracts) =
            self.select_contracts_and_senders(invariant_address)?;
        let targets = targeted_contracts.targets();
        Ok(InvariantSettings::new(&targets, &sender_filters, self.config.fail_on_revert))
    }
}

/// Collects data from call for fuzzing. However, it first verifies that the sender is not an EOA
/// before inserting it into the dictionary. Otherwise, we flood the dictionary with
/// randomly generated addresses.
fn collect_data<FEN: FoundryEvmNetwork>(
    invariant_test: &InvariantTest,
    state_changeset: &mut AddressMap<Account>,
    tx: &BasicTxDetails,
    call_result: &RawCallResult<FEN>,
    run_depth: u32,
    mapping_slots: Option<&AddressMap<foundry_common::mapping_slots::MappingSlots>>,
) {
    // Verify it has no code.
    let has_code = if let Some(Some(code)) =
        state_changeset.get(&tx.sender).map(|account| account.info.code.as_ref())
    {
        !code.is_empty()
    } else {
        false
    };

    // We keep the nonce changes to apply later.
    let sender_changeset = if has_code { None } else { state_changeset.remove(&tx.sender) };

    // Collect values from fuzzed call result and add them to fuzz dictionary.
    invariant_test.fuzz_state.collect_values_from_call(
        &invariant_test.targeted_contracts,
        tx,
        &call_result.result,
        &call_result.logs,
        &*state_changeset,
        run_depth,
        mapping_slots,
    );

    // Inject typed sancov trace-cmp operands into the fuzz dictionary.
    if let Some(cmp_values) = &call_result.sancov_cmp_values {
        invariant_test.fuzz_state.collect_typed_cmp_values(
            cmp_values.iter().map(|s| (s.width, alloy_primitives::B256::from(s.value))),
        );
    }
    // Re-add changes
    if let Some(changed) = sender_changeset {
        state_changeset.insert(tx.sender, changed);
    }
}

/// Calls the `afterInvariant()` function on a contract.
/// Returns call result and if call succeeded.
/// The state after the call is not persisted.
///
/// Uses the handler-gate success check so a stale committed `GLOBAL_FAIL_SLOT` from a
/// previously-recorded handler bug doesn't false-positive this call (the slot is `1` from
/// the prior bug, but `afterInvariant` itself didn't write it in this changeset).
pub(crate) fn call_after_invariant_function<FEN: FoundryEvmNetwork>(
    executor: &Executor<FEN>,
    to: Address,
) -> Result<(RawCallResult<FEN>, bool), EvmError<FEN>> {
    let calldata = Bytes::from_static(&IInvariantTest::afterInvariantCall::SELECTOR);
    let mut call_result = executor.call_raw(CALLER, to, calldata, U256::ZERO)?;
    let success = executor.is_raw_call_mut_success_handler_gate(to, &mut call_result);
    Ok((call_result, success))
}

/// Calls the invariant function and returns call result and if succeeded.
///
/// Uses the handler-gate success check (same rationale as `call_after_invariant_function`):
/// the predicate is broken iff this call's own changeset writes `GLOBAL_FAIL_SLOT` (via `t()` /
/// `vm.assert*`) or the call reverts; a stale committed slot from a prior handler bug must not
/// poison every later predicate evaluation in the run.
pub(crate) fn call_invariant_function<FEN: FoundryEvmNetwork>(
    executor: &Executor<FEN>,
    address: Address,
    calldata: Bytes,
) -> Result<(RawCallResult<FEN>, bool)> {
    let mut call_result = executor.call_raw(CALLER, address, calldata, U256::ZERO)?;
    let success = executor.is_raw_call_mut_success_handler_gate(address, &mut call_result);
    Ok((call_result, success))
}

/// Executes a fuzz call and returns the result.
/// Applies any block timestamp (warp) and block number (roll) adjustments before the call.
pub(crate) fn execute_tx<FEN: FoundryEvmNetwork>(
    executor: &mut Executor<FEN>,
    tx: &BasicTxDetails,
) -> Result<RawCallResult<FEN>> {
    let warp = tx.warp.unwrap_or_default();
    let roll = tx.roll.unwrap_or_default();

    if warp > 0 || roll > 0 {
        // Apply pre-call block adjustments to the executor's env.
        let ts = executor.evm_env().block_env.timestamp();
        let num = executor.evm_env().block_env.number();
        executor.evm_env_mut().block_env.set_timestamp(ts + warp);
        executor.evm_env_mut().block_env.set_number(num + roll);

        // Also update the inspector's cheatcodes.block if set.
        // The inspector's block may override the env during interpreter initialization,
        // so we need to add our warp/roll on top of any existing cheatcode-set values.
        let block_env = executor.evm_env().block_env.clone();
        if let Some(cheatcodes) = executor.inspector_mut().cheatcodes.as_mut() {
            if let Some(block) = cheatcodes.block.as_mut() {
                let bts = block.timestamp();
                let bnum = block.number();
                block.set_timestamp(bts + warp);
                block.set_number(bnum + roll);
            } else {
                cheatcodes.block = Some(block_env);
            }
        }
    }

    // Bound requested value by sender's available balance so payable paths still get
    // exercised when the requested value exceeds balance, instead of collapsing to zero.
    let requested_value = tx.call_details.value.unwrap_or(U256::ZERO);
    let sender_balance = executor.get_balance(tx.sender)?;
    let value = requested_value.min(sender_balance);
    executor
        .call_raw(tx.sender, tx.call_details.target, tx.call_details.calldata.clone(), value)
        .map_err(|e| eyre!(format!("Could not make raw evm call: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::{prelude::any, strategy::ValueTree, test_runner::Config};
    use serde_json::json;

    fn first_generated_u64(runner: &mut TestRunner) -> u64 {
        any::<u64>().new_tree(runner).unwrap().current()
    }

    fn test_runner() -> TestRunner {
        TestRunner::new(Config { failure_persistence: None, ..Default::default() })
    }

    fn seeded_test_runner(seed: U256) -> TestRunner {
        let config = Config { failure_persistence: None, ..Default::default() };
        let rng = TestRng::from_seed(RngAlgorithm::ChaCha, &seed.to_be_bytes::<32>());
        TestRunner::new_with_rng(config, rng)
    }

    #[test]
    fn invariant_worker_seed_preserves_master_seed_and_derives_workers() {
        let seed = U256::from(0x1234);

        assert_eq!(invariant_worker_seed(seed, 0), seed);
        assert_ne!(invariant_worker_seed(seed, 1), seed);
        assert_ne!(invariant_worker_seed(seed, 1), invariant_worker_seed(seed, 2));
        assert_ne!(invariant_worker_seed(seed, 1), invariant_worker_seed(U256::from(0x5678), 1));
    }

    #[test]
    fn invariant_worker_runner_preserves_seed_for_master_worker() {
        let seed = U256::from(0x1234);
        let mut seeded_runner = seeded_test_runner(seed);
        let mut parent = test_runner();
        let mut worker = invariant_worker_runner(&mut parent, 0, Some(seed));

        assert_eq!(first_generated_u64(&mut worker), first_generated_u64(&mut seeded_runner));
    }

    #[test]
    fn invariant_worker_runner_uses_seed_independent_of_parent_rng_state() {
        let seed = U256::from(0x1234);
        let mut parent = test_runner();
        let mut advanced_parent = test_runner();
        let _ = first_generated_u64(&mut advanced_parent);

        let mut worker = invariant_worker_runner(&mut parent, 1, Some(seed));
        let mut worker_from_advanced_parent =
            invariant_worker_runner(&mut advanced_parent, 1, Some(seed));

        assert_eq!(
            first_generated_u64(&mut worker),
            first_generated_u64(&mut worker_from_advanced_parent)
        );
    }

    #[test]
    fn invariant_progress_json_includes_throughput_fields() {
        let throughput = InvariantThroughputMetrics { total_txs: 2, total_gas: 50 };

        let payload = build_invariant_progress_json(
            InvariantProgressContext {
                timestamp_secs: 123,
                contract_name: "InvariantContract",
                optimization_best: Some(I256::try_from(42).unwrap()),
                throughput,
                elapsed: Duration::from_secs(10),
                worker_id: 1,
                worker_count: 4,
            },
            &json!({ "corpus_count": 7 }),
            &InvariantFailureMetrics::default(),
        );

        assert_eq!(payload["timestamp"], json!(123));
        assert_eq!(payload["contract"], json!("InvariantContract"));
        assert!(payload.get("invariant").is_none());
        assert_eq!(payload["metrics"]["corpus_count"], json!(7));
        assert_eq!(payload["metrics"]["broken_assertions"], json!(0));
        assert!(payload["metrics"].get("broken_handlers").is_none());
        assert_eq!(payload["total_txs"], json!(2));
        assert_eq!(payload["total_gas"], json!(50));
        assert_eq!(payload["tps"], json!(0.2));
        assert_eq!(payload["gps"], json!(5.0));
        assert!(payload.get("tx_per_sec").is_none());
        assert!(payload.get("gas_per_sec").is_none());
        assert_eq!(payload["worker"]["id"], json!(1));
        assert_eq!(payload["worker"]["count"], json!(4));
        assert_eq!(payload["optimization_best"], json!("42"));
    }

    #[test]
    fn invariant_worker_count_keeps_short_campaigns_single_worker() {
        assert_eq!(
            max_invariant_workers_for_campaign(0, DEFAULT_DEPTH_FOR_INVARIANT_WORKER_CAP),
            1
        );
        assert_eq!(
            max_invariant_workers_for_campaign(
                MIN_RUNS_PER_INVARIANT_WORKER - 1,
                DEFAULT_DEPTH_FOR_INVARIANT_WORKER_CAP
            ),
            1
        );
        assert_eq!(
            max_invariant_workers_for_campaign(
                MIN_RUNS_PER_INVARIANT_WORKER,
                DEFAULT_DEPTH_FOR_INVARIANT_WORKER_CAP
            ),
            1
        );
        assert_eq!(
            max_invariant_workers_for_campaign(
                MIN_RUNS_PER_INVARIANT_WORKER * 2,
                DEFAULT_DEPTH_FOR_INVARIANT_WORKER_CAP
            ),
            2
        );
        assert_eq!(max_invariant_workers_for_campaign(256, 100_000), 5);
    }

    #[test]
    fn invariant_worker_count_preserves_fixed_workers() {
        let mut config = InvariantConfig {
            runs: MIN_RUNS_PER_INVARIANT_WORKER * 4,
            workers: foundry_config::InvariantWorkers::Fixed(
                std::num::NonZeroUsize::new(4).unwrap(),
            ),
            ..Default::default()
        };
        assert_eq!(invariant_worker_count_with_threads(&config, 8, 1), 4);

        config.corpus.show_edge_coverage = true;
        assert_eq!(invariant_worker_count_with_threads(&config, 8, 1), 4);

        config.corpus.show_edge_coverage = false;
        config.corpus.corpus_dir = Some(std::path::PathBuf::from("corpus"));
        assert_eq!(invariant_worker_count_with_threads(&config, 8, 1), 4);

        config.runs = MIN_RUNS_PER_INVARIANT_WORKER - 1;
        config.timeout = None;
        assert_eq!(invariant_worker_count_with_threads(&config, 8, 1), 4);

        config.timeout = Some(1);
        assert_eq!(invariant_worker_count_with_threads(&config, 8, 4), 4);
    }

    #[test]
    fn invariant_worker_count_does_not_cap_configured_workers_by_available_threads() {
        let config = InvariantConfig {
            runs: MIN_RUNS_PER_INVARIANT_WORKER * 8,
            workers: foundry_config::InvariantWorkers::Fixed(
                std::num::NonZeroUsize::new(8).unwrap(),
            ),
            ..Default::default()
        };

        assert_eq!(invariant_worker_count_with_threads(&config, 4, 1), 8);
    }

    #[test]
    fn invariant_worker_count_splits_available_threads_for_auto_workers() {
        let mut config = InvariantConfig {
            runs: MIN_RUNS_PER_INVARIANT_WORKER * 4,
            depth: DEFAULT_DEPTH_FOR_INVARIANT_WORKER_CAP,
            workers: foundry_config::InvariantWorkers::Auto,
            ..Default::default()
        };

        assert_eq!(invariant_worker_count_with_threads(&config, 4, 1), 4);
        assert_eq!(invariant_worker_count_with_threads(&config, 8, 2), 4);
        assert_eq!(invariant_worker_count_with_threads(&config, 8, 3), 2);
        assert_eq!(invariant_worker_count_with_threads(&config, 3, 8), 1);
        assert_eq!(invariant_worker_count_with_threads(&config, 0, 0), 1);

        config.runs = MIN_RUNS_PER_INVARIANT_WORKER - 1;
        assert_eq!(invariant_worker_count_with_threads(&config, 8, 2), 1);

        config.depth = 100_000;
        assert_eq!(invariant_worker_count_with_threads(&config, 8, 2), 4);

        config.timeout = Some(1);
        assert_eq!(invariant_worker_count_with_threads(&config, 8, 2), 4);
    }

    #[test]
    fn invariant_worker_cmp_log_selection_uses_one_worker_per_campaign() {
        let mut config = InvariantConfig::default();
        assert!(!invariant_worker_collects_evm_cmp_log(&config, 0, 1));

        config.corpus.corpus_dir = Some("corpus".into());
        assert!(invariant_worker_collects_evm_cmp_log(&config, 0, 1));
        assert!(invariant_worker_collects_evm_cmp_log(&config, 0, 4));
        assert!(!invariant_worker_collects_evm_cmp_log(&config, 1, 4));
        assert!(!invariant_worker_collects_evm_cmp_log(&config, 3, 4));

        config.corpus.sancov_edges = true;
        assert!(!invariant_worker_collects_evm_cmp_log(&config, 0, 1));
        assert!(!invariant_worker_collects_evm_cmp_log(&config, 0, 4));
    }

    #[test]
    fn timed_invariant_workers_are_not_bounded_by_assigned_runs() {
        let plan = InvariantWorkerPlan { worker_id: 0, first_global_run: 0, runs: 1 };

        let untimed = InvariantCampaignState::new(EarlyExit::new(false), None);
        assert!(should_continue_invariant_worker(&untimed, 0, plan));
        assert!(!should_continue_invariant_worker(&untimed, 1, plan));

        let timed = InvariantCampaignState::new(EarlyExit::new(false), Some(60));
        assert!(should_continue_invariant_worker(&timed, 0, plan));
        assert!(should_continue_invariant_worker(&timed, 1, plan));
        assert!(should_continue_invariant_worker(&timed, 10_000, plan));
    }

    #[test]
    fn gas_report_samples_are_split_across_workers() {
        assert_eq!(gas_report_samples_for_worker(0, 0, 4), 0);
        assert_eq!(gas_report_samples_for_worker(8, 0, 4), 2);
        assert_eq!(gas_report_samples_for_worker(8, 3, 4), 2);
        assert_eq!(gas_report_samples_for_worker(10, 0, 4), 3);
        assert_eq!(gas_report_samples_for_worker(10, 1, 4), 3);
        assert_eq!(gas_report_samples_for_worker(10, 2, 4), 2);
        assert_eq!(gas_report_samples_for_worker(10, 3, 4), 2);
        assert_eq!(gas_report_samples_for_worker(3, 3, 4), 0);
    }

    #[test]
    fn invariant_progress_json_zero_elapsed_reports_zero_rates() {
        let throughput = InvariantThroughputMetrics { total_txs: 1, total_gas: 21_000 };

        let payload = build_invariant_progress_json(
            InvariantProgressContext {
                timestamp_secs: 456,
                contract_name: "invariant_zero_elapsed",
                optimization_best: None,
                throughput,
                elapsed: Duration::ZERO,
                worker_id: 0,
                worker_count: 1,
            },
            &json!({ "corpus_count": 1 }),
            &InvariantFailureMetrics::default(),
        );

        assert_eq!(payload["tps"], json!(0.0));
        assert_eq!(payload["gps"], json!(0.0));
        assert!(payload.get("optimization_best").is_none());
    }

    #[test]
    fn invariant_progress_json_rounds_fractional_rates() {
        let payload = build_invariant_progress_json(
            InvariantProgressContext {
                timestamp_secs: 456,
                contract_name: "TestContract",
                optimization_best: None,
                throughput: InvariantThroughputMetrics { total_txs: 1, total_gas: 1 },
                elapsed: Duration::from_secs(3),
                worker_id: 0,
                worker_count: 1,
            },
            &json!({ "corpus_count": 1 }),
            &InvariantFailureMetrics::default(),
        );

        assert_eq!(payload["tps"], json!(0.33));
        assert_eq!(payload["gps"], json!(0.33));
    }

    #[test]
    fn invariant_progress_json_includes_broken_counts() {
        let mut failure_metrics = InvariantFailureMetrics::default();
        failure_metrics.record_failure("invariant_a", "TestContract", "revert");
        failure_metrics.record_failure("invariant_a", "TestContract", "revert");
        failure_metrics.record_failure("invariant_b", "TestContract", "assertion failed");
        failure_metrics.broken_handlers = 7;

        let payload = build_invariant_progress_json(
            InvariantProgressContext {
                timestamp_secs: 789,
                contract_name: "TestContract",
                optimization_best: None,
                throughput: InvariantThroughputMetrics::default(),
                elapsed: Duration::from_secs(1),
                worker_id: 0,
                worker_count: 1,
            },
            &json!({ "corpus_count": 5 }),
            &failure_metrics,
        );

        assert!(payload["metrics"].get("failures").is_none());
        assert!(payload["metrics"].get("unique_failures").is_none());
        assert_eq!(payload["metrics"]["broken_invariants"], json!(2));
        assert_eq!(payload["metrics"]["broken_assertions"], json!(7));
        assert!(payload["metrics"].get("broken_handlers").is_none());
    }

    #[test]
    fn failure_metrics_tracks_total_and_unique_failures() {
        let mut metrics = InvariantFailureMetrics::default();
        metrics.record_failure("invariant_a", "TestContract", "revert");
        metrics.record_failure("invariant_a", "TestContract", "revert");
        metrics.record_failure("invariant_b", "TestContract", "assertion failed");

        assert_eq!(metrics.failures, 3);
        assert_eq!(metrics.unique_failures.len(), 2);
        assert!(metrics.unique_failures.contains("invariant_a"));
        assert!(metrics.unique_failures.contains("invariant_b"));
    }

    #[test]
    fn failure_metrics_default_is_zero() {
        let metrics = InvariantFailureMetrics::default();
        assert_eq!(metrics.failures, 0);
        assert!(metrics.unique_failures.is_empty());
        assert_eq!(metrics.broken_handlers, 0);
    }
}
