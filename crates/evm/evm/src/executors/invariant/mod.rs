use crate::{
    executors::{
        DURATION_BETWEEN_METRICS_REPORT, EarlyExit, EvmError, Executor, FuzzTestTimer,
        RawCallResult, corpus::WorkerCorpus,
    },
    inspectors::Fuzzer,
};
use alloy_json_abi::Function;
use alloy_primitives::{Address, Bytes, FixedBytes, I256, Selector, U256, map::AddressMap};
use alloy_sol_types::{SolCall, sol};
use eyre::{ContextCompat, Result, eyre};
use foundry_common::{
    TestFunctionExt,
    contracts::{ContractsByAddress, ContractsByArtifact},
    sh_eprintln, sh_println,
};
use foundry_config::InvariantConfig;
use foundry_evm_core::{
    FoundryBlock,
    constants::{
        CALLER, CHEATCODE_ADDRESS, DEFAULT_CREATE2_DEPLOYER, HARDHAT_CONSOLE_ADDRESS, MAGIC_ASSUME,
    },
    evm::FoundryEvmNetwork,
    precompiles::PRECOMPILES,
};
use foundry_evm_fuzz::{
    BasicTxDetails, FuzzCase, FuzzFixtures, FuzzedCases,
    invariant::{
        ArtifactFilters, FuzzRunIdentifiedContracts, InvariantContract, InvariantSettings,
        RandomCallGenerator, SenderFilters, TargetedContract, TargetedContracts,
    },
    strategies::{EvmFuzzState, InvariantFuzzState, invariant_strat, override_call_strat},
};
use foundry_evm_traces::{CallTraceArena, SparsedTraceArena};
use indicatif::ProgressBar;
use parking_lot::RwLock;
use proptest::{strategy::Strategy, test_runner::TestRunner};
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

mod replay;
pub use replay::{replay_error, replay_run};

mod result;
pub use result::InvariantFuzzTestResult;

mod shrink;
pub use shrink::{
    CheckSequenceOptions, HandlerReplayOutcome, check_sequence, check_sequence_value,
    replay_handler_failure_sequence,
};

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
    const fn record_call(&mut self, gas_used: u64) {
        self.total_txs += 1;
        self.total_gas += gas_used;
    }

    fn tx_per_sec(self, elapsed: Duration) -> f64 {
        rate_per_sec(self.total_txs as f64, elapsed)
    }

    fn gas_per_sec(self, elapsed: Duration) -> f64 {
        rate_per_sec(self.total_gas as f64, elapsed)
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

/// Tracks invariant failure counts during a campaign.
#[derive(Debug, Default)]
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
    failure_metrics: &mut InvariantFailureMetrics,
    invariant_contract: &InvariantContract<'_>,
    failures: &InvariantFailures,
) {
    for (f, _) in &invariant_contract.invariant_fns {
        if !failure_metrics.unique_failures.contains(&f.name) && failures.has_failure(f) {
            let reason =
                failures.get_failure(f).and_then(|e| e.revert_reason()).unwrap_or_default();
            failure_metrics.record_failure(&f.name, invariant_contract.name, &reason);
        }
    }
}

/// Builds the machine-readable invariant progress payload emitted during a
/// campaign.
///
/// This keeps the existing corpus progress metrics together with cumulative and
/// derived throughput fields so downstream benchmark tooling can consume a
/// single JSON event shape.
fn build_invariant_progress_json<M: Serialize>(
    timestamp_secs: u64,
    invariant_name: &str,
    corpus_metrics: &M,
    optimization_best: Option<I256>,
    throughput: InvariantThroughputMetrics,
    failure_metrics: &InvariantFailureMetrics,
    elapsed: Duration,
) -> serde_json::Value {
    let mut metrics = serde_json::to_value(corpus_metrics).unwrap_or_default();
    if let Some(obj) = metrics.as_object_mut() {
        obj.insert("failures".to_string(), json!(failure_metrics.failures));
        obj.insert("unique_failures".to_string(), json!(failure_metrics.unique_failures.len()));
        // Surface unique handler-side assertion bugs in live progress, separate from
        // invariant predicate violations counted by `failures`.
        obj.insert("broken_handlers".to_string(), json!(failure_metrics.broken_handlers));
    }

    let mut payload = json!({
        "timestamp": timestamp_secs,
        "event": "pulse",
        "invariant": invariant_name,
        "metrics": metrics,
        "total_txs": throughput.total_txs,
        "total_gas": throughput.total_gas,
        "tx_per_sec": throughput.tx_per_sec(elapsed),
        "gas_per_sec": throughput.gas_per_sec(elapsed),
    });

    if let Some(best) = optimization_best {
        payload["optimization_best"] = json!(best.to_string());
    }

    payload
}

/// Contains data collected during invariant test runs.
struct InvariantTestData {
    // Consumed gas and calldata of every successful fuzz call.
    fuzz_cases: Vec<FuzzedCases>,
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
            fuzz_cases: vec![],
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
        self.test_data.fuzz_cases.push(FuzzedCases::new(run.fuzz_runs));

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
    /// The invariant configuration
    config: InvariantConfig,
    /// Contracts deployed with `setUp()`
    setup_contracts: &'a ContractsByAddress,
    /// Contracts that are part of the project but have not been deployed yet. We need the bytecode
    /// to identify them from the stateset changes.
    project_contracts: &'a ContractsByArtifact,
    /// Filters contracts to be fuzzed through their artifact identifiers.
    artifact_filters: ArtifactFilters,
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
        Self {
            executor,
            runner,
            config,
            setup_contracts,
            project_contracts,
            artifact_filters: ArtifactFilters::default(),
        }
    }

    pub fn config(self) -> InvariantConfig {
        self.config
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
        // Note: invariant function signatures (no inputs) are validated upstream in the
        // suite runner so parameterized `invariant_*` functions are rejected with a per-test
        // failure entry before any campaign runs.

        let (mut invariant_test, mut corpus_manager) = self.prepare_test(
            &invariant_contract,
            fuzz_fixtures,
            fuzz_state,
            initial_handler_failures,
        )?;

        // Start timer for this invariant test.
        let mut runs = 0;
        let timer = FuzzTestTimer::new(self.config.timeout);
        let mut last_metrics_report = Instant::now();
        let campaign_start = Instant::now();
        let mut throughput = InvariantThroughputMetrics::default();
        let mut failure_metrics = InvariantFailureMetrics::default();
        let continue_campaign = |runs: u32| {
            if early_exit.should_stop() {
                return false;
            }

            if timer.is_enabled() { !timer.is_timed_out() } else { runs < self.config.runs }
        };

        // Invariant runs with edge coverage if corpus dir is set or showing edge coverage.
        let edge_coverage_enabled = self.config.corpus.collect_edge_coverage();

        'stop: while continue_campaign(runs) {
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
                self.executor.clone(),
                self.config.depth as usize,
            );

            // We stop the run immediately if we have reverted, and `fail_on_revert` is set.
            if self.config.fail_on_revert && invariant_test.reverts() > 0 {
                return Err(eyre!("call reverted"));
            }

            while current_run.depth < self.config.depth {
                // Check if the timeout has been reached.
                if timer.is_timed_out() {
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
                if self.config.show_metrics {
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
                if corpus_manager.merge_edge_coverage(&mut call_result) {
                    current_run.new_coverage = true;
                }

                if discarded {
                    current_run.inputs.pop();
                    current_run.rejects += 1;
                    if current_run.rejects > self.config.max_assume_rejects {
                        invariant_test.set_error(
                            invariant_contract.anchor(),
                            InvariantFuzzError::MaxAssumeRejects(self.config.max_assume_rejects),
                        );
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
                            self.config.depth,
                            mapping_slots,
                        );
                    }

                    // Collect created contracts and add to fuzz targets only if targeted contracts
                    // are updatable.
                    if let Err(error) =
                        &invariant_test.targeted_contracts.collect_created_contracts(
                            &state_changeset,
                            self.project_contracts,
                            self.setup_contracts,
                            &self.artifact_filters,
                            &mut current_run.created_contracts,
                        )
                    {
                        warn!(target: "forge::test", "{error}");
                    }
                    current_run
                        .fuzz_runs
                        .push(FuzzCase { gas: call_result.gas_used, stipend: call_result.stipend });
                    throughput.record_call(call_result.gas_used);

                    // Determine if test can continue or should exit.
                    // Check invariants based on check_interval to improve deep run performance.
                    // - check_interval=0: only assert on the last call
                    // - check_interval=1 (default): assert after every call
                    // - check_interval=N: assert every N calls AND always on the last call
                    let is_last_call = current_run.depth == self.config.depth - 1;
                    // In optimization mode, always evaluate the invariant to track
                    // the best value at every prefix — check_interval only gates
                    // boolean invariant assertions.
                    let is_optimization = invariant_contract.is_optimization();
                    let should_check_invariant = is_optimization
                        || if self.config.check_interval == 0 {
                            is_last_call
                        } else {
                            self.config.check_interval == 1
                                || (current_run.depth + 1)
                                    .is_multiple_of(self.config.check_interval)
                                || is_last_call
                        };

                    let errors_before_check = invariant_test.test_data.failures.invariant_count();
                    let (continues, broken) = if should_check_invariant {
                        let outcome = can_continue(
                            &invariant_contract,
                            &mut invariant_test,
                            &mut current_run,
                            &self.config,
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
                                &self.config,
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
                        } else if call_result.reverted && self.config.fail_on_revert {
                            // Plain revert under fail_on_revert: attribute to the anchor.
                            let anchor = invariant_contract.anchor();
                            let case_data = error::InvariantRunCtx {
                                contract: &invariant_contract,
                                config: &self.config,
                                targeted_contracts: &invariant_test.targeted_contracts,
                                calldata: &current_run.inputs,
                            }
                            .failed_case(
                                anchor,
                                self.config.fail_on_revert,
                                false,
                                call_result,
                                &[],
                            );
                            invariant_test.set_error(anchor, InvariantFuzzError::Revert(case_data));
                            (false, Some(anchor))
                        } else if call_result.reverted
                            && !invariant_contract.is_optimization()
                            && !self.config.has_delay()
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

                    if !continues || current_run.depth == self.config.depth - 1 {
                        invariant_test.set_last_run_inputs(&current_run.inputs);
                    }
                    // Bridge newly-recorded predicate breaks into `failure_metrics` even when
                    // `continues == true` in multi-predicate campaigns.
                    if invariant_test.test_data.failures.invariant_count() > errors_before_check
                        || broken.is_some()
                    {
                        record_new_invariant_failures(
                            &mut failure_metrics,
                            &invariant_contract,
                            &invariant_test.test_data.failures,
                        );
                    }
                    if !continues {
                        if invariant_contract.invariant_fns.len() > 1 && !self.config.fail_on_revert
                        {
                            break;
                        }
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
            corpus_manager.process_inputs(
                &current_run.inputs,
                &current_run.cmp_seq,
                current_run.new_coverage,
                optimization,
            );

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
                    &self.config,
                )
                .map_err(|_| eyre!("Failed to call afterInvariant"))?;
                if broken.is_some() {
                    // Bridge breaks into pulse metrics, mirroring the in-run path above.
                    record_new_invariant_failures(
                        &mut failure_metrics,
                        &invariant_contract,
                        &invariant_test.test_data.failures,
                    );
                }
            }

            // End current invariant test run.
            invariant_test.end_run(current_run, self.config.gas_report_samples as usize);
            if let Some(progress) = progress {
                // If running with progress then increment completed runs.
                progress.inc(1);
                // Display current best value, corpus metrics, and failure counts.
                let best = invariant_test.test_data.optimization_best_value;
                let broken = invariant_test.test_data.failures.invariant_count();
                // Live count of unique handler-side assertion bugs, separate from the
                // predicate breaks in `broken`. Synced into `failure_metrics` so all
                // campaign-level counters share one struct.
                failure_metrics.broken_handlers = invariant_test.test_data.failures.handler_count();
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
                    progress.set_message(msg);
                }
            } else if edge_coverage_enabled
                && last_metrics_report.elapsed() > DURATION_BETWEEN_METRICS_REPORT
            {
                // Sync handler-bug count snapshot into failure_metrics before emitting.
                failure_metrics.broken_handlers = invariant_test.test_data.failures.handler_count();
                // Display corpus metrics inline as JSON.
                let metrics = build_invariant_progress_json(
                    SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
                    &invariant_contract.anchor().name,
                    &corpus_manager.metrics,
                    invariant_test.test_data.optimization_best_value,
                    throughput,
                    &failure_metrics,
                    campaign_start.elapsed(),
                );
                let _ = sh_println!("{}", serde_json::to_string(&metrics)?);
                last_metrics_report = Instant::now();
            }

            runs += 1;
            if stop_after_run {
                break 'stop;
            }
        }

        trace!(?fuzz_fixtures);
        invariant_test.fuzz_state.log_stats();

        let mut result = invariant_test.test_data;

        // Post-campaign: shrink each handler bug's call sequence to its minimal prefix.
        let total = result.failures.handler_count();
        if total > 0 {
            for (idx, (_site, error)) in result.failures.handler_failures_mut().enumerate() {
                if early_exit.should_stop() {
                    break;
                }
                let Some(failure) = error.as_handler_assertion_mut() else {
                    // Handler-keyed entries always store `HandlerAssertion` by construction.
                    continue;
                };
                shrink::reset_shrink_progress(
                    &self.config,
                    progress,
                    &format!("handler {:#x}::{}", failure.reverter, failure.selector),
                    Some((idx + 1, total)),
                );
                match shrink::shrink_handler_sequence(
                    &self.config,
                    &failure.call_sequence,
                    failure.edge_fingerprint,
                    &self.executor,
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

        let reverts = result.failures.reverts;
        let (errors, handler_errors) = result.failures.partition();
        Ok(InvariantFuzzTestResult {
            errors,
            handler_errors,
            cases: result.fuzz_cases,
            reverts,
            last_run_inputs: result.last_run_inputs,
            gas_report_traces: result.gas_report_traces,
            line_coverage: result.line_coverage,
            metrics: result.metrics,
            failed_corpus_replays: corpus_manager.failed_replays,
            optimization_best_value: result.optimization_best_value,
            optimization_best_sequence: result.optimization_best_sequence,
        })
    }

    /// Prepares certain structures to execute the invariant tests:
    /// * Invariant Fuzz Test.
    /// * Invariant Corpus Manager.
    fn prepare_test(
        &mut self,
        invariant_contract: &InvariantContract<'_>,
        fuzz_fixtures: &FuzzFixtures,
        fuzz_state: EvmFuzzState,
        initial_handler_failures: std::collections::HashMap<
            (Address, Selector),
            InvariantFuzzError,
        >,
    ) -> Result<(InvariantTest, WorkerCorpus)> {
        // Finds out the chosen deployed contracts and/or senders.
        self.select_contract_artifacts(invariant_contract.address)?;
        let (targeted_senders, targeted_contracts) =
            self.select_contracts_and_senders(invariant_contract.address)?;
        let fuzz_state = fuzz_state.into_invariant();

        // Creates the invariant strategy.
        let strategy = invariant_strat(
            fuzz_state.clone(),
            targeted_senders,
            targeted_contracts.clone(),
            self.config.clone(),
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
        self.executor.inspector_mut().set_fuzzer(Fuzzer {
            call_generator: None,
            collected_values: Vec::new(),
            max_collected_values: self.config.dictionary.max_fuzz_dictionary_values,
            mapping_slots,
            collect: true,
        });

        // Let's make sure the invariant is sound before actually starting the run:
        // We'll assert the invariant in its initial state, and if it fails, we'll
        // already know if we can early exit the invariant run.
        // This does not count as a fuzz run. It will just register the revert.
        let mut failures = InvariantFailures::new();
        // Seed disk-recovered handler bugs so live counters reflect them from tick 0.
        for ((addr, sel), err) in initial_handler_failures {
            failures.seed_handler_failure(addr, sel, err);
        }
        invariant_preflight_check(
            invariant_contract,
            &self.config,
            &targeted_contracts,
            &self.executor,
            &[],
            &mut failures,
        )?;
        if let Some(fuzzer) = self.executor.inspector_mut().fuzzer.as_mut() {
            fuzz_state.collect_values(fuzzer.drain_collected_values());
        }
        // NOW enable call_override after the initial invariant check has passed.
        // This allows `override_call_strat` to inject calls during actual fuzz runs
        // for reentrancy vulnerability detection.
        if self.config.call_override {
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
                self.runner.clone(),
                override_call_strat(
                    fuzz_state.snapshot(),
                    override_targets,
                    target_contract_ref.clone(),
                    fuzz_fixtures.clone(),
                ),
                target_contract_ref,
            );

            if let Some(fuzzer) = self.executor.inspector_mut().fuzzer.as_mut() {
                fuzzer.call_generator = Some(call_generator);
            }
        }

        let worker = WorkerCorpus::new(
            0,
            self.config.corpus.clone(),
            strategy.boxed(),
            Some(&self.executor),
            None,
            Some(&targeted_contracts),
        )?;

        let mut invariant_test =
            InvariantTest::new(fuzz_state, targeted_contracts, failures, self.runner.clone());

        // Seed invariant test with previously persisted optimization state,
        // but only if the current invariant is in optimization mode.
        if invariant_contract.is_optimization() {
            let (opt_best_value, opt_best_sequence) = worker.optimization_initial_state();
            invariant_test.test_data.optimization_best_value = opt_best_value;
            invariant_test.test_data.optimization_best_sequence = opt_best_sequence;
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
    use serde_json::json;

    #[test]
    fn invariant_progress_json_includes_throughput_fields() {
        let mut throughput = InvariantThroughputMetrics::default();
        throughput.record_call(20);
        throughput.record_call(30);

        let payload = build_invariant_progress_json(
            123,
            "invariant_balance",
            &json!({ "corpus_count": 7 }),
            Some(I256::try_from(42).unwrap()),
            throughput,
            &InvariantFailureMetrics::default(),
            Duration::from_secs(10),
        );

        assert_eq!(payload["timestamp"], json!(123));
        assert_eq!(payload["invariant"], json!("invariant_balance"));
        assert_eq!(payload["metrics"]["corpus_count"], json!(7));
        assert_eq!(payload["metrics"]["broken_handlers"], json!(0));
        assert_eq!(payload["total_txs"], json!(2));
        assert_eq!(payload["total_gas"], json!(50));
        assert!((payload["tx_per_sec"].as_f64().unwrap() - 0.2).abs() < 1e-12);
        assert!((payload["gas_per_sec"].as_f64().unwrap() - 5.0).abs() < 1e-12);
        assert_eq!(payload["optimization_best"], json!("42"));
    }

    #[test]
    fn invariant_progress_json_zero_elapsed_reports_zero_rates() {
        let mut throughput = InvariantThroughputMetrics::default();
        throughput.record_call(21_000);

        let payload = build_invariant_progress_json(
            456,
            "invariant_zero_elapsed",
            &json!({ "corpus_count": 1 }),
            None,
            throughput,
            &InvariantFailureMetrics::default(),
            Duration::ZERO,
        );

        assert_eq!(payload["tx_per_sec"], json!(0.0));
        assert_eq!(payload["gas_per_sec"], json!(0.0));
        assert!(payload.get("optimization_best").is_none());
    }

    #[test]
    fn invariant_progress_json_includes_failure_counts() {
        let mut failure_metrics = InvariantFailureMetrics::default();
        failure_metrics.record_failure("invariant_a", "TestContract", "revert");
        failure_metrics.record_failure("invariant_a", "TestContract", "revert");
        failure_metrics.record_failure("invariant_b", "TestContract", "assertion failed");
        failure_metrics.broken_handlers = 7;

        let payload = build_invariant_progress_json(
            789,
            "invariant_a",
            &json!({ "corpus_count": 5 }),
            None,
            InvariantThroughputMetrics::default(),
            &failure_metrics,
            Duration::from_secs(1),
        );

        assert_eq!(payload["metrics"]["failures"], json!(3));
        assert_eq!(payload["metrics"]["unique_failures"], json!(2));
        assert_eq!(payload["metrics"]["broken_handlers"], json!(7));
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
