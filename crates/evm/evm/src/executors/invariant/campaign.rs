use super::{InvariantFailures, InvariantFuzzError, InvariantMetrics};
use crate::executors::Executor;
use alloy_json_abi::Function;
use alloy_primitives::{Address, I256, Selector};
use foundry_common::sh_eprintln;
use foundry_evm_core::evm::FoundryEvmNetwork;
use foundry_evm_coverage::HitMaps;
use foundry_evm_fuzz::{
    BasicTxDetails, FuzzCase, FuzzedCases, invariant::FuzzRunIdentifiedContracts,
    strategies::InvariantFuzzState,
};
use foundry_evm_traces::SparsedTraceArena;
use proptest::test_runner::TestRunner;
use revm_inspectors::tracing::CallTraceArena as RevmCallTraceArena;
use serde::Serialize;
use serde_json::json;
use std::{
    collections::{HashMap as Map, HashSet},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// The outcome of an invariant fuzz test.
#[derive(Debug)]
pub struct InvariantFuzzTestResult {
    /// Errors recorded per invariant.
    pub errors: Map<String, InvariantFuzzError>,
    /// Handler-side assertion bugs, keyed by `(reverter, selector)` site (deduped per
    /// handler function). Each entry is [`InvariantFuzzError::HandlerAssertion`].
    pub handler_errors: Map<(Address, Selector), InvariantFuzzError>,
    /// Every successful fuzz test case.
    pub cases: Vec<FuzzedCases>,
    /// Number of reverted fuzz calls.
    pub reverts: usize,
    /// The entire inputs of the last run of the invariant campaign, used for
    /// replaying the run for collecting traces.
    pub last_run_inputs: Vec<BasicTxDetails>,
    /// Additional traces used for gas report construction.
    pub gas_report_traces: Vec<Vec<RevmCallTraceArena>>,
    /// The coverage info collected during the invariant test runs.
    pub line_coverage: Option<HitMaps>,
    /// Fuzzed selectors metrics collected during the invariant test runs.
    pub metrics: Map<String, InvariantMetrics>,
    /// Number of failed replays from persisted corpus.
    pub failed_corpus_replays: usize,
    /// For optimization mode (int256 return): the best (maximum) value achieved.
    /// None means standard invariant check mode.
    pub optimization_best_value: Option<I256>,
    /// For optimization mode: the call sequence that produced the best value.
    pub optimization_best_sequence: Vec<BasicTxDetails>,
}

/// Campaign-level throughput metrics for invariant progress reporting.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct InvariantThroughputMetrics {
    pub(crate) total_txs: u64,
    pub(crate) total_gas: u64,
}

impl InvariantThroughputMetrics {
    pub(crate) const fn record_call(&mut self, gas_used: u64) {
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
pub(crate) struct InvariantFailureMetrics {
    pub(crate) failures: u64,
    pub(crate) unique_failures: HashSet<String>,
    /// Unique handler-side assertion bugs found so far.
    pub(crate) broken_handlers: usize,
}

impl InvariantFailureMetrics {
    /// Records a failure and emits a structured JSON `"failure"` event.
    pub(crate) fn record_failure(&mut self, invariant_name: &str, target: &str, reason: &str) {
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
pub(crate) fn record_new_invariant_failures(
    failure_metrics: &mut InvariantFailureMetrics,
    invariant_contract: &foundry_evm_fuzz::invariant::InvariantContract<'_>,
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
pub(crate) fn build_invariant_progress_json<M: Serialize>(
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

/// Mutable state accumulated while a logical invariant campaign runs.
pub(crate) struct InvariantCampaignState {
    /// Consumed gas and calldata of every successful fuzz call.
    pub(crate) fuzz_cases: Vec<FuzzedCases>,
    /// Data related to reverts or failed assertions of the test.
    pub(crate) failures: InvariantFailures,
    /// Calldata in the last invariant run.
    pub(crate) last_run_inputs: Vec<BasicTxDetails>,
    /// Additional traces for gas report.
    pub(crate) gas_report_traces: Vec<Vec<RevmCallTraceArena>>,
    /// Line coverage information collected from all fuzzed calls.
    pub(crate) line_coverage: Option<HitMaps>,
    /// Metrics for each fuzzed selector.
    pub(crate) metrics: Map<String, InvariantMetrics>,

    /// Proptest runner to query for random values.
    /// The strategy only comes with the first `input`. We fill the rest of the `inputs`
    /// until the desired `depth` so we can use the evolving fuzz dictionary
    /// during the run.
    pub(crate) branch_runner: TestRunner,

    /// Optimization mode state: tracks the best (maximum) value and the sequence that produced it.
    /// Only used when invariant function returns int256.
    pub(crate) optimization_best_value: Option<I256>,
    /// Optimization mode: the call sequence that produced the best value.
    pub(crate) optimization_best_sequence: Vec<BasicTxDetails>,
}

impl InvariantCampaignState {
    fn new(failures: InvariantFailures, branch_runner: TestRunner) -> Self {
        Self {
            fuzz_cases: vec![],
            failures,
            last_run_inputs: vec![],
            gas_report_traces: vec![],
            line_coverage: None,
            metrics: Map::default(),
            branch_runner,
            optimization_best_value: None,
            optimization_best_sequence: vec![],
        }
    }

    pub(crate) fn into_fuzz_result(self, failed_corpus_replays: usize) -> InvariantFuzzTestResult {
        let reverts = self.failures.reverts;
        let (errors, handler_errors) = self.failures.partition();
        InvariantFuzzTestResult {
            errors,
            handler_errors,
            cases: self.fuzz_cases,
            reverts,
            last_run_inputs: self.last_run_inputs,
            gas_report_traces: self.gas_report_traces,
            line_coverage: self.line_coverage,
            metrics: self.metrics,
            failed_corpus_replays,
            optimization_best_value: self.optimization_best_value,
            optimization_best_sequence: self.optimization_best_sequence,
        }
    }
}

/// Runtime state for one logical invariant campaign.
pub(crate) struct InvariantCampaign {
    /// Fuzz state of invariant test.
    pub(crate) fuzz_state: InvariantFuzzState,
    /// Contracts fuzzed by the invariant test.
    pub(crate) targeted_contracts: FuzzRunIdentifiedContracts,
    /// Data collected during invariant runs.
    pub(crate) state: InvariantCampaignState,
}

impl InvariantCampaign {
    /// Instantiates an invariant campaign.
    pub(crate) fn new(
        fuzz_state: InvariantFuzzState,
        targeted_contracts: FuzzRunIdentifiedContracts,
        failures: InvariantFailures,
        branch_runner: TestRunner,
    ) -> Self {
        let state = InvariantCampaignState::new(failures, branch_runner);
        Self { fuzz_state, targeted_contracts, state }
    }

    /// Returns number of invariant test reverts.
    pub(crate) const fn reverts(&self) -> usize {
        self.state.failures.reverts
    }

    /// Set invariant test error.
    pub(crate) fn set_error(&mut self, invariant: &Function, error: InvariantFuzzError) {
        self.state.failures.record_failure(invariant, error);
    }

    /// Set last invariant run call sequence.
    pub(crate) fn set_last_run_inputs(&mut self, inputs: &Vec<BasicTxDetails>) {
        self.state.last_run_inputs.clone_from(inputs);
    }

    /// Merge current collected line coverage with the new coverage from last fuzzed call.
    pub(crate) fn merge_line_coverage(&mut self, new_coverage: Option<HitMaps>) {
        HitMaps::merge_opt(&mut self.state.line_coverage, new_coverage);
    }

    /// Update metrics for a fuzzed selector, extracted from tx details.
    /// Always increments number of calls; discarded runs (through assume cheatcodes) are tracked
    /// separated from reverts.
    pub(crate) fn record_metrics(
        &mut self,
        tx_details: &BasicTxDetails,
        reverted: bool,
        discarded: bool,
    ) {
        if let Some(metric_key) = self.targeted_contracts.targets().fuzzed_metric_key(tx_details) {
            let test_metrics = &mut self.state.metrics;
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
    pub(crate) fn end_run<FEN: FoundryEvmNetwork>(
        &mut self,
        run: InvariantWorkerRun<FEN>,
        gas_samples: usize,
    ) {
        // We clear all the targeted contracts created during this run.
        self.targeted_contracts.clear_created_contracts(run.created_contracts);

        if self.state.gas_report_traces.len() < gas_samples {
            self.state
                .gas_report_traces
                .push(run.run_traces.into_iter().map(|arena| arena.arena).collect());
        }
        self.state.fuzz_cases.push(FuzzedCases::new(run.fuzz_runs));

        // Revert state to not persist values between runs.
        self.fuzz_state.revert();
    }

    /// Updates the optimization state if the new value is better (higher) than the current best.
    pub(crate) fn update_optimization_value(&mut self, value: I256, sequence: &[BasicTxDetails]) {
        if self.state.optimization_best_value.is_none_or(|best| value > best) {
            self.state.optimization_best_value = Some(value);
            self.state.optimization_best_sequence = sequence.to_vec();
        }
    }
}

/// Worker-local state for a single invariant run.
pub(crate) struct InvariantWorkerRun<FEN: FoundryEvmNetwork> {
    /// Invariant run call sequence.
    pub(crate) inputs: Vec<BasicTxDetails>,
    /// Per-call EVM comparison operands (parallel to `inputs`), captured for I2S corpus mutation.
    pub(crate) cmp_seq: Vec<Vec<crate::inspectors::CmpOperands>>,
    /// Current invariant run executor.
    pub(crate) executor: Executor<FEN>,
    /// Invariant run stat reports (eg. gas usage).
    pub(crate) fuzz_runs: Vec<FuzzCase>,
    /// Contracts created during current invariant run.
    pub(crate) created_contracts: Vec<Address>,
    /// Traces of each call of the invariant run call sequence.
    pub(crate) run_traces: Vec<SparsedTraceArena>,
    /// Current depth of invariant run.
    pub(crate) depth: u32,
    /// Current assume rejects of the invariant run.
    pub(crate) rejects: u32,
    /// Whether new coverage was discovered during this run.
    pub(crate) new_coverage: bool,
    /// For optimization mode: the best value found during this run (if any).
    pub(crate) optimization_value: Option<I256>,
    /// For optimization mode: the length of the input prefix that produced the best value.
    pub(crate) optimization_prefix_len: usize,
}

impl<FEN: FoundryEvmNetwork> InvariantWorkerRun<FEN> {
    /// Instantiates an invariant worker run.
    pub(crate) fn new(first_input: BasicTxDetails, executor: Executor<FEN>, depth: usize) -> Self {
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
