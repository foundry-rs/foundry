use super::{
    FailureKey, InvariantFailureMetrics, InvariantFailures, InvariantFuzzError,
    InvariantFuzzTestResult, InvariantMetrics,
};
use crate::executors::{EarlyExit, EvmExecutionCancellation};
use alloy_primitives::{Address, I256, Selector};
use eyre::{Result, ensure};
use foundry_evm_coverage::HitMaps;
use foundry_evm_fuzz::BasicTxDetails;
use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

/// Immutable plan-level description for an invariant campaign.
///
/// This is only a planning contract for splitting one logical campaign into worker ranges. It does
/// not start workers, choose worker counts, or decide corpus/failure persistence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvariantCampaignSpec {
    /// Total logical runs configured for the campaign.
    pub total_runs: u32,
}

impl InvariantCampaignSpec {
    pub const fn new(total_runs: u32) -> Self {
        Self { total_runs }
    }

    /// Partitions the logical campaign into contiguous worker run ranges.
    ///
    /// This only describes work assignment. It does not start worker execution and does not
    /// attribute failures to worker/run origins.
    pub fn worker_plans(self, workers: usize) -> Result<Vec<InvariantWorkerPlan>> {
        ensure!(workers > 0, "invariant campaign requires at least one worker");

        if self.total_runs == 0 {
            return Ok(vec![InvariantWorkerPlan { worker_id: 0, first_global_run: 0, runs: 0 }]);
        }

        let worker_count = workers.min(self.total_runs as usize) as u32;
        let base_runs = self.total_runs / worker_count;
        let extra_runs = self.total_runs % worker_count;

        let mut first_global_run = 0;
        let mut plans = Vec::with_capacity(worker_count as usize);
        for worker_id in 0..worker_count {
            let runs = base_runs + u32::from(worker_id < extra_runs);
            plans.push(InvariantWorkerPlan { worker_id, first_global_run, runs });
            first_global_run += runs;
        }

        debug_assert_eq!(first_global_run, self.total_runs);
        Ok(plans)
    }
}

/// Static assignment of a contiguous logical run range to one worker.
///
/// The assigned range is `[first_global_run, first_global_run + runs)`.
/// Worker `0` is the master worker for master-only artifacts such as persisted corpus replay
/// counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvariantWorkerPlan {
    pub worker_id: u32,
    pub first_global_run: u32,
    pub runs: u32,
}

/// Shared state used only to coordinate invariant worker execution.
pub struct InvariantCampaignState {
    started_at: Instant,
    timed: bool,
    total_runs: AtomicU32,
    total_txs: AtomicU64,
    total_gas: AtomicU64,
    cancellation: EvmExecutionCancellation,
    last_metrics_report: Mutex<Instant>,
    failure_metrics: Mutex<CampaignFailureMetrics>,
}

#[derive(Default)]
struct CampaignFailureMetrics {
    metrics: InvariantFailureMetrics,
    handler_sites: HashSet<(Address, Selector)>,
}

impl InvariantCampaignState {
    pub fn new(early_exit: EarlyExit, timeout: Option<u32>) -> Self {
        let started_at = Instant::now();
        let deadline = timeout
            .map(|timeout| Duration::from_secs(timeout.into()))
            .and_then(|timeout| started_at.checked_add(timeout));
        Self {
            started_at,
            timed: timeout.is_some(),
            total_runs: AtomicU32::new(0),
            total_txs: AtomicU64::new(0),
            total_gas: AtomicU64::new(0),
            cancellation: EvmExecutionCancellation::campaign(
                early_exit,
                Arc::new(AtomicBool::new(false)),
                deadline,
            ),
            last_metrics_report: Mutex::new(started_at),
            failure_metrics: Mutex::new(CampaignFailureMetrics::default()),
        }
    }

    pub fn increment_runs(&self) -> u32 {
        self.total_runs.fetch_add(1, Ordering::Relaxed) + 1
    }

    #[cfg(test)]
    pub fn total_runs(&self) -> u32 {
        self.total_runs.load(Ordering::Relaxed)
    }

    pub fn record_call(&self, gas_used: u64) {
        self.total_txs.fetch_add(1, Ordering::Relaxed);
        self.total_gas.fetch_add(gas_used, Ordering::Relaxed);
    }

    pub fn throughput_totals(&self) -> (u64, u64) {
        (self.total_txs.load(Ordering::Relaxed), self.total_gas.load(Ordering::Relaxed))
    }

    pub fn elapsed(&self) -> Duration {
        self.started_at.elapsed()
    }

    pub const fn is_timed_campaign(&self) -> bool {
        self.timed
    }

    pub fn should_stop(&self) -> bool {
        self.cancellation.should_stop(true)
    }

    pub fn request_terminal_stop(&self) {
        self.cancellation.request_stop();
    }

    pub fn should_emit_metrics_report(&self, interval: Duration) -> bool {
        let mut last_report =
            self.last_metrics_report.lock().expect("metrics report lock poisoned");
        if last_report.elapsed() <= interval {
            return false;
        }

        *last_report = Instant::now();
        true
    }

    pub(super) fn record_invariant_failure(
        &self,
        invariant_name: &str,
        target: &str,
        reason: &str,
    ) {
        let mut failure_metrics =
            self.failure_metrics.lock().expect("failure metrics lock poisoned");
        if !failure_metrics.metrics.unique_failures.contains(invariant_name) {
            failure_metrics.metrics.record_failure(invariant_name, target, reason);
        }
    }

    pub(super) fn sync_handler_failures(&self, failures: &InvariantFailures) {
        let mut failure_metrics =
            self.failure_metrics.lock().expect("failure metrics lock poisoned");
        for (key, error) in &failures.failures {
            let FailureKey::Handler(addr, selector) = key else { continue };
            if failure_metrics.handler_sites.insert((*addr, *selector)) {
                let reason = error.revert_reason().unwrap_or_default();
                failure_metrics.metrics.record_handler_failure(*addr, *selector, &reason);
            }
        }
        debug_assert_eq!(
            failure_metrics.metrics.broken_handlers,
            failure_metrics.handler_sites.len()
        );
    }

    pub(super) fn failure_metrics(&self) -> InvariantFailureMetrics {
        self.failure_metrics.lock().expect("failure metrics lock poisoned").metrics.clone()
    }

    pub const fn early_exit(&self) -> &EarlyExit {
        self.cancellation.early_exit_ref()
    }

    pub const fn cancellation(&self) -> &EvmExecutionCancellation {
        &self.cancellation
    }
}

/// Output produced by one invariant worker.
///
/// This is a data envelope for aggregation only. It does not imply that this module executed the
/// worker, shrank failures, or wrote any persisted corpus/failure files.
#[derive(Debug)]
pub struct InvariantWorkerOutput {
    pub plan: InvariantWorkerPlan,
    pub result: InvariantFuzzTestResult,
}

impl InvariantWorkerOutput {
    #[cfg(test)]
    pub const fn new(plan: InvariantWorkerPlan, result: InvariantFuzzTestResult) -> Self {
        Self { plan, result }
    }
}

/// Merges worker outputs back into one logical invariant campaign result.
///
/// Merge policy:
/// - outputs are folded in `first_global_run` order;
/// - predicate failures keep the first failure in logical run order;
/// - handler assertion failures keep the shorter reproducer, with equal lengths preserving the
///   earlier logical worker;
/// - optimization mode keeps the maximum value, with ties preserving the earlier logical worker;
/// - `failed_corpus_replays` is a master-worker-only value from worker `0`;
/// - run/call counts, reverts, gas traces, selector metrics, and line coverage accumulate into the
///   logical campaign result.
#[derive(Debug)]
pub struct InvariantCampaignAggregator {
    spec: InvariantCampaignSpec,
    outputs: Vec<InvariantWorkerOutput>,
}

impl InvariantCampaignAggregator {
    pub const fn new(spec: InvariantCampaignSpec) -> Self {
        Self { spec, outputs: Vec::new() }
    }

    pub fn push(&mut self, output: InvariantWorkerOutput) {
        self.outputs.push(output);
    }

    /// Validates the collected worker ranges and folds them into one logical campaign result.
    #[cfg(test)]
    pub fn finish(self) -> Result<InvariantFuzzTestResult> {
        self.finish_campaign()
    }

    pub fn finish_campaign(mut self) -> Result<InvariantFuzzTestResult> {
        ensure!(!self.outputs.is_empty(), "missing invariant worker output");

        self.outputs.sort_by_key(|output| output.plan.first_global_run);
        ensure_outputs_cover_campaign(self.spec, &self.outputs)?;
        fold_outputs(self.outputs)
    }

    /// Folds timeout worker outputs without requiring full logical campaign coverage.
    ///
    /// Timeout campaigns share a wall-clock deadline across workers. When the deadline hits, any
    /// worker may have completed fewer than its assigned runs, so the original static ranges can
    /// contain gaps. The merge still validates worker identity and preserves deterministic worker
    /// order, but final run count is derived from the completed worker counters.
    pub fn finish_partial(mut self) -> Result<InvariantFuzzTestResult> {
        ensure!(!self.outputs.is_empty(), "missing invariant worker output");

        self.outputs.sort_by_key(|output| output.plan.first_global_run);
        ensure_worker_ids_are_valid(&self.outputs)?;
        fold_outputs(self.outputs)
    }
}

fn fold_outputs(outputs: Vec<InvariantWorkerOutput>) -> Result<InvariantFuzzTestResult> {
    let workers = outputs.len();
    let mut errors = HashMap::default();
    let mut handler_errors = HashMap::default();
    let mut runs = 0;
    let mut calls = 0;
    let mut reverts = 0;
    let mut last_run_inputs = Vec::new();
    let mut gas_report_traces = Vec::new();
    let mut line_coverage = None;
    let mut metrics = HashMap::default();
    let mut failed_corpus_replays = 0;
    let mut optimization_best = None;

    for InvariantWorkerOutput { plan, result } in outputs {
        if plan.worker_id == 0 {
            failed_corpus_replays = result.failed_corpus_replays;
        }
        for (invariant, error) in result.errors {
            errors.entry(invariant).or_insert(error);
        }
        merge_handler_errors(&mut handler_errors, result.handler_errors);
        runs += result.runs;
        calls += result.calls;
        reverts += result.reverts;
        if !result.last_run_inputs.is_empty() {
            last_run_inputs = result.last_run_inputs;
        }
        gas_report_traces.extend(result.gas_report_traces);
        HitMaps::merge_opt(&mut line_coverage, result.line_coverage);
        merge_metrics(&mut metrics, result.metrics);
        merge_optimization(
            &mut optimization_best,
            result.optimization_best_value,
            result.optimization_best_sequence,
        );
    }
    let (optimization_best_value, optimization_best_sequence) =
        optimization_best.map(|(value, sequence)| (Some(value), sequence)).unwrap_or_default();
    Ok(InvariantFuzzTestResult::new(
        errors,
        handler_errors,
        runs,
        calls,
        reverts,
        last_run_inputs,
        gas_report_traces,
        line_coverage,
        metrics,
        failed_corpus_replays,
        workers,
        optimization_best_value,
        optimization_best_sequence,
    ))
}

fn ensure_outputs_cover_campaign(
    spec: InvariantCampaignSpec,
    outputs: &[InvariantWorkerOutput],
) -> Result<()> {
    ensure_worker_ids_are_valid(outputs)?;

    if spec.total_runs == 0 {
        ensure!(
            outputs.len() == 1
                && outputs[0].plan.first_global_run == 0
                && outputs[0].plan.runs == 0,
            "invariant worker outputs do not cover the logical campaign"
        );
        return Ok(());
    }

    let mut next_global_run = 0;
    for output in outputs {
        ensure!(output.plan.runs > 0, "invariant worker outputs do not cover the logical campaign");
        ensure!(
            output.plan.first_global_run == next_global_run,
            "invariant worker outputs do not cover the logical campaign"
        );
        next_global_run = next_global_run
            .checked_add(output.plan.runs)
            .ok_or_else(|| eyre::eyre!("invariant worker output range overflows"))?;
    }

    ensure!(
        next_global_run == spec.total_runs,
        "invariant worker outputs do not cover the logical campaign"
    );
    Ok(())
}

fn ensure_worker_ids_are_valid(outputs: &[InvariantWorkerOutput]) -> Result<()> {
    let mut seen = HashSet::with_capacity(outputs.len());
    for output in outputs {
        ensure!(
            seen.insert(output.plan.worker_id),
            "duplicate invariant worker output for worker {}",
            output.plan.worker_id
        );
    }

    ensure!(seen.contains(&0), "missing invariant master worker output");
    Ok(())
}

/// Deduplicates handler assertion failures by site, keeping the shorter reproducer.
/// Equal-length reproducers keep the one already inserted, which is the earlier logical worker
/// because the caller folds worker outputs in `first_global_run` order.
fn merge_handler_errors(
    merged: &mut HashMap<(Address, Selector), InvariantFuzzError>,
    worker_errors: HashMap<(Address, Selector), InvariantFuzzError>,
) {
    for (site, error) in worker_errors {
        let candidate_len = handler_error_sequence_len(&error);
        if merged
            .get(&site)
            .is_none_or(|existing| handler_error_sequence_len(existing) > candidate_len)
        {
            merged.insert(site, error);
        }
    }
}

/// Adds worker-local selector metrics into the logical campaign totals.
fn merge_metrics(
    merged: &mut HashMap<String, InvariantMetrics>,
    worker_metrics: HashMap<String, InvariantMetrics>,
) {
    for (selector, metrics) in worker_metrics {
        let entry = merged.entry(selector).or_default();
        entry.calls += metrics.calls;
        entry.reverts += metrics.reverts;
        entry.discards += metrics.discards;
    }
}

/// Keeps the best optimization value, using logical run order to break ties.
fn merge_optimization(
    best: &mut Option<(I256, Vec<BasicTxDetails>)>,
    candidate_value: Option<I256>,
    candidate_sequence: Vec<BasicTxDetails>,
) {
    let Some(candidate_value) = candidate_value else {
        return;
    };

    if best.as_ref().is_none_or(|(best, _)| candidate_value > *best) {
        *best = Some((candidate_value, candidate_sequence));
    }
}

fn handler_error_sequence_len(error: &InvariantFuzzError) -> usize {
    error.as_handler_assertion().map_or(usize::MAX, |failure| failure.call_sequence.len())
}

#[cfg(test)]
mod tests {
    use super::{
        super::error::{FailedInvariantCaseData, HandlerAssertionFailure},
        *,
    };
    use alloy_primitives::{B256, Bytes};
    use foundry_evm_coverage::HitMap;
    use foundry_evm_fuzz::CallDetails;
    use proptest::test_runner::TestError;
    use revm_inspectors::tracing::CallTraceArena;

    fn empty_result(reverts: usize, failed_corpus_replays: usize) -> InvariantFuzzTestResult {
        InvariantFuzzTestResult::new(
            HashMap::default(),
            HashMap::default(),
            0,
            0,
            reverts,
            Vec::new(),
            Vec::new(),
            None,
            HashMap::default(),
            failed_corpus_replays,
            1,
            None,
            Vec::new(),
        )
    }

    fn basic_tx(sender: u8) -> BasicTxDetails {
        BasicTxDetails {
            warp: None,
            roll: None,
            sender: Address::repeat_byte(sender),
            call_details: CallDetails {
                target: Address::repeat_byte(sender.wrapping_add(1)),
                calldata: Bytes::from(vec![0, 0, 0, sender]),
                value: None,
            },
        }
    }

    fn hit_maps(pc: u32, hits: u32) -> HitMaps {
        let mut hit_map = HitMap::new(Bytes::from_static(&[0]));
        hit_map.hits(pc, hits);

        let mut maps = HitMaps::default();
        maps.insert(B256::ZERO, hit_map);
        maps
    }

    /// Builds a worker-local result fixture with the fields merged by the aggregator.
    fn worker_result(
        reverts: usize,
        last_input_sender: u8,
        metric_name: &str,
        metrics: InvariantMetrics,
        coverage_hits: u32,
        failed_corpus_replays: usize,
    ) -> InvariantFuzzTestResult {
        let mut result = empty_result(reverts, failed_corpus_replays);
        result.runs = 1;
        result.calls = metrics.calls;
        result.last_run_inputs = vec![basic_tx(last_input_sender)];
        result.gas_report_traces.push(vec![CallTraceArena::default()]);
        result.line_coverage = Some(hit_maps(7, coverage_hits));
        result.metrics.insert(metric_name.to_string(), metrics);
        result
    }

    fn sequence(len: usize, first_sender: u8) -> Vec<BasicTxDetails> {
        (0..len).map(|idx| basic_tx(first_sender.wrapping_add(idx as u8))).collect()
    }

    /// Builds a predicate failure fixture with a reproducible call sequence.
    fn predicate_error(reason: &str, sequence_len: usize) -> InvariantFuzzError {
        InvariantFuzzError::BrokenInvariant(FailedInvariantCaseData {
            test_error: TestError::Fail(reason.to_string().into(), sequence(sequence_len, 0x80)),
            return_reason: reason.to_string().into(),
            revert_reason: reason.to_string(),
            addr: Address::repeat_byte(0x70),
            calldata: Bytes::new(),
            inner_sequence: Vec::new(),
            shrink_run_limit: 0,
            fail_on_revert: false,
            assertion_failure: false,
        })
    }

    /// Builds a handler assertion fixture with a reproducible call sequence.
    fn handler_error(
        reverter: Address,
        selector: Selector,
        sequence_len: usize,
        reason: &str,
    ) -> InvariantFuzzError {
        InvariantFuzzError::HandlerAssertion(HandlerAssertionFailure {
            reverter,
            selector,
            call_sequence: sequence(sequence_len, 0x90),
            original_sequence_len: sequence_len,
            revert_reason: reason.to_string(),
            edge_fingerprint: B256::ZERO,
        })
    }

    fn one_worker_plan(total_runs: u32) -> InvariantWorkerPlan {
        let mut plans = InvariantCampaignSpec::new(total_runs).worker_plans(1).unwrap();
        assert_eq!(plans.len(), 1);
        plans.pop().unwrap()
    }

    #[test]
    fn worker_plans_cover_logical_campaign_with_one_worker() {
        let plan = one_worker_plan(3);

        assert_eq!(plan.worker_id, 0);
        assert_eq!(plan.first_global_run, 0);
        assert_eq!(plan.runs, 3);
    }

    #[test]
    fn worker_plans_split_runs_evenly() {
        let plans = InvariantCampaignSpec::new(100).worker_plans(4).unwrap();

        assert_eq!(
            plans,
            vec![
                InvariantWorkerPlan { worker_id: 0, first_global_run: 0, runs: 25 },
                InvariantWorkerPlan { worker_id: 1, first_global_run: 25, runs: 25 },
                InvariantWorkerPlan { worker_id: 2, first_global_run: 50, runs: 25 },
                InvariantWorkerPlan { worker_id: 3, first_global_run: 75, runs: 25 },
            ]
        );
    }

    #[test]
    fn worker_plans_distribute_remainder_to_earlier_workers() {
        let plans = InvariantCampaignSpec::new(10).worker_plans(3).unwrap();

        assert_eq!(
            plans,
            vec![
                InvariantWorkerPlan { worker_id: 0, first_global_run: 0, runs: 4 },
                InvariantWorkerPlan { worker_id: 1, first_global_run: 4, runs: 3 },
                InvariantWorkerPlan { worker_id: 2, first_global_run: 7, runs: 3 },
            ]
        );
    }

    #[test]
    fn worker_plans_do_not_create_empty_workers_when_runs_are_available() {
        let plans = InvariantCampaignSpec::new(2).worker_plans(8).unwrap();

        assert_eq!(
            plans,
            vec![
                InvariantWorkerPlan { worker_id: 0, first_global_run: 0, runs: 1 },
                InvariantWorkerPlan { worker_id: 1, first_global_run: 1, runs: 1 },
            ]
        );
    }

    #[test]
    fn worker_plans_keep_zero_run_campaign_as_single_empty_plan() {
        let plans = InvariantCampaignSpec::new(0).worker_plans(4).unwrap();

        assert_eq!(plans, vec![InvariantWorkerPlan { worker_id: 0, first_global_run: 0, runs: 0 }]);
    }

    #[test]
    fn worker_plans_reject_zero_workers() {
        let err = InvariantCampaignSpec::new(1).worker_plans(0).unwrap_err();
        assert!(err.to_string().contains("requires at least one worker"));

        let err = InvariantCampaignSpec::new(0).worker_plans(0).unwrap_err();
        assert!(err.to_string().contains("requires at least one worker"));
    }

    #[test]
    fn campaign_state_stops_after_terminal_request() {
        let state = InvariantCampaignState::new(EarlyExit::new(false), None);
        assert!(!state.should_stop());

        state.request_terminal_stop();

        assert!(state.should_stop());
    }

    #[test]
    fn campaign_state_uses_shared_timeout_and_global_throughput() {
        let state = InvariantCampaignState::new(EarlyExit::new(false), Some(0));
        std::thread::sleep(Duration::from_millis(1));

        assert!(state.is_timed_campaign());
        assert!(state.should_stop());

        state.record_call(20);
        state.record_call(30);
        assert_eq!(state.throughput_totals(), (2, 50));
        assert_eq!(state.increment_runs(), 1);
        assert_eq!(state.total_runs(), 1);
    }

    #[test]
    fn campaign_state_deduplicates_handler_failure_events_across_workers() {
        let state = InvariantCampaignState::new(EarlyExit::new(false), None);
        let target = Address::repeat_byte(0x11);
        let selector = Selector::from([0xde, 0xad, 0xbe, 0xef]);
        let mut first_worker = InvariantFailures::new();
        first_worker.seed_handler_failure(
            target,
            selector,
            handler_error(target, selector, 2, "assertion failed"),
        );
        let mut second_worker = InvariantFailures::new();
        second_worker.seed_handler_failure(
            target,
            selector,
            handler_error(target, selector, 1, "assertion failed"),
        );

        state.sync_handler_failures(&first_worker);
        state.sync_handler_failures(&second_worker);

        assert_eq!(state.failure_metrics().broken_handlers, 1);
    }

    #[test]
    fn aggregator_returns_single_worker_result_without_rewriting() {
        let spec = InvariantCampaignSpec::new(1);
        let worker = InvariantWorkerOutput::new(one_worker_plan(1), empty_result(2, 3));

        let mut aggregator = InvariantCampaignAggregator::new(spec);
        aggregator.push(worker);
        let result = aggregator.finish().unwrap();

        assert_eq!(result.reverts, 2);
        assert_eq!(result.failed_corpus_replays, 3);
    }

    #[test]
    fn aggregator_accepts_single_worker_output_for_zero_run_campaign() {
        let spec = InvariantCampaignSpec::new(0);
        let worker = InvariantWorkerOutput::new(
            InvariantWorkerPlan { worker_id: 0, first_global_run: 0, runs: 0 },
            empty_result(0, 0),
        );

        let mut aggregator = InvariantCampaignAggregator::new(spec);
        aggregator.push(worker);
        let result = aggregator.finish().unwrap();

        assert_eq!(result.reverts, 0);
    }

    #[test]
    fn aggregator_merges_multiple_worker_outputs_in_logical_run_order() {
        let spec = InvariantCampaignSpec::new(3);
        let plans = [
            InvariantWorkerPlan { worker_id: 0, first_global_run: 0, runs: 1 },
            InvariantWorkerPlan { worker_id: 1, first_global_run: 1, runs: 1 },
            InvariantWorkerPlan { worker_id: 2, first_global_run: 2, runs: 1 },
        ];

        let mut aggregator = InvariantCampaignAggregator::new(spec);
        aggregator.push(InvariantWorkerOutput::new(
            plans[2],
            worker_result(
                3,
                0x30,
                "transfer(address)",
                InvariantMetrics { calls: 3, reverts: 1, discards: 0 },
                3,
                0,
            ),
        ));
        aggregator.push(InvariantWorkerOutput::new(
            plans[0],
            worker_result(
                1,
                0x10,
                "transfer(address)",
                InvariantMetrics { calls: 1, reverts: 0, discards: 2 },
                1,
                4,
            ),
        ));
        aggregator.push(InvariantWorkerOutput::new(
            plans[1],
            worker_result(
                2,
                0x20,
                "approve(address)",
                InvariantMetrics { calls: 2, reverts: 1, discards: 1 },
                2,
                0,
            ),
        ));

        let result = aggregator.finish().unwrap();

        assert_eq!(result.runs, 3);
        assert_eq!(result.calls, 6);
        assert_eq!(result.reverts, 6);
        assert_eq!(result.gas_report_traces.len(), 3);
        assert_eq!(result.last_run_inputs[0].sender, Address::repeat_byte(0x30));

        let transfer_metrics = result.metrics.get("transfer(address)").unwrap();
        assert_eq!(transfer_metrics, &InvariantMetrics { calls: 4, reverts: 1, discards: 2 });
        let approve_metrics = result.metrics.get("approve(address)").unwrap();
        assert_eq!(approve_metrics, &InvariantMetrics { calls: 2, reverts: 1, discards: 1 });

        let coverage = result.line_coverage.unwrap();
        assert_eq!(coverage.get(&B256::ZERO).unwrap().get(7).unwrap().get(), 6);
        assert_eq!(result.failed_corpus_replays, 4);
    }

    #[test]
    fn aggregator_preserves_run_and_call_counts() {
        let spec = InvariantCampaignSpec::new(3);
        let plans = [
            InvariantWorkerPlan { worker_id: 0, first_global_run: 0, runs: 1 },
            InvariantWorkerPlan { worker_id: 1, first_global_run: 1, runs: 2 },
        ];
        let mut first = empty_result(0, 0);
        first.runs = 1;
        first.calls = 1000;
        let mut second = empty_result(0, 0);
        second.runs = 2;
        second.calls = 2000;

        let mut aggregator = InvariantCampaignAggregator::new(spec);
        aggregator.push(InvariantWorkerOutput::new(plans[1], second));
        aggregator.push(InvariantWorkerOutput::new(plans[0], first));
        let result = aggregator.finish().unwrap();

        assert_eq!(result.runs, 3);
        assert_eq!(result.calls, 3000);
    }

    #[test]
    fn timeout_aggregator_accepts_partial_outputs_with_range_gaps() {
        fn result_with_counts(
            runs: usize,
            calls: usize,
            has_last_run: bool,
            failed_corpus_replays: usize,
        ) -> InvariantFuzzTestResult {
            let mut result = empty_result(0, failed_corpus_replays);
            result.runs = runs;
            result.calls = calls;
            result.last_run_inputs = if has_last_run { vec![basic_tx(0x44)] } else { Vec::new() };
            result
        }

        let spec = InvariantCampaignSpec::new(10);
        let outputs = [
            InvariantWorkerOutput::new(
                InvariantWorkerPlan { worker_id: 0, first_global_run: 0, runs: 2 },
                result_with_counts(2, 20, true, 5),
            ),
            InvariantWorkerOutput::new(
                InvariantWorkerPlan { worker_id: 1, first_global_run: 4, runs: 0 },
                result_with_counts(0, 0, false, 0),
            ),
            InvariantWorkerOutput::new(
                InvariantWorkerPlan { worker_id: 2, first_global_run: 7, runs: 1 },
                result_with_counts(1, 10, true, 0),
            ),
        ];

        let mut strict = InvariantCampaignAggregator::new(spec);
        for output in outputs {
            strict.push(output);
        }
        let err = strict.finish().unwrap_err();
        assert!(err.to_string().contains("do not cover the logical campaign"));

        let mut partial = InvariantCampaignAggregator::new(spec);
        partial.push(InvariantWorkerOutput::new(
            InvariantWorkerPlan { worker_id: 0, first_global_run: 0, runs: 2 },
            result_with_counts(2, 20, true, 5),
        ));
        partial.push(InvariantWorkerOutput::new(
            InvariantWorkerPlan { worker_id: 1, first_global_run: 4, runs: 0 },
            result_with_counts(0, 0, false, 0),
        ));
        partial.push(InvariantWorkerOutput::new(
            InvariantWorkerPlan { worker_id: 2, first_global_run: 7, runs: 1 },
            result_with_counts(1, 10, true, 0),
        ));

        let result = partial.finish_partial().unwrap();

        assert_eq!(result.runs, 3);
        assert_eq!(result.calls, 30);
        assert_eq!(result.failed_corpus_replays, 5);
    }

    #[test]
    fn aggregator_keeps_earlier_predicate_failure_for_each_invariant() {
        let spec = InvariantCampaignSpec::new(2);
        let plans = [
            InvariantWorkerPlan { worker_id: 0, first_global_run: 0, runs: 1 },
            InvariantWorkerPlan { worker_id: 1, first_global_run: 1, runs: 1 },
        ];
        let mut earlier = empty_result(0, 0);
        earlier.errors.insert("invariant_balance".to_string(), predicate_error("earlier", 3));
        let mut later = empty_result(0, 0);
        later.errors.insert("invariant_balance".to_string(), predicate_error("later", 1));

        let mut aggregator = InvariantCampaignAggregator::new(spec);
        aggregator.push(InvariantWorkerOutput::new(plans[1], later));
        aggregator.push(InvariantWorkerOutput::new(plans[0], earlier));
        let result = aggregator.finish().unwrap();

        assert_eq!(result.errors.len(), 1);
        assert_eq!(result.errors["invariant_balance"].revert_reason().as_deref(), Some("earlier"));
    }

    #[test]
    fn aggregator_dedupes_handler_assertions_by_site_and_keeps_shorter_sequence() {
        let spec = InvariantCampaignSpec::new(2);
        let plans = [
            InvariantWorkerPlan { worker_id: 0, first_global_run: 0, runs: 1 },
            InvariantWorkerPlan { worker_id: 1, first_global_run: 1, runs: 1 },
        ];
        let site = (Address::repeat_byte(0xaa), Selector::from([1, 2, 3, 4]));
        let mut longer = empty_result(0, 0);
        longer.handler_errors.insert(site, handler_error(site.0, site.1, 4, "longer"));
        let mut shorter = empty_result(0, 0);
        shorter.handler_errors.insert(site, handler_error(site.0, site.1, 2, "shorter"));

        let mut aggregator = InvariantCampaignAggregator::new(spec);
        aggregator.push(InvariantWorkerOutput::new(plans[1], shorter));
        aggregator.push(InvariantWorkerOutput::new(plans[0], longer));
        let result = aggregator.finish().unwrap();

        let failure = result.handler_errors[&site].as_handler_assertion().unwrap();
        assert_eq!(result.handler_errors.len(), 1);
        assert_eq!(failure.call_sequence.len(), 2);
        assert_eq!(failure.revert_reason, "shorter");
    }

    #[test]
    fn aggregator_keeps_earlier_handler_assertion_when_lengths_tie() {
        let spec = InvariantCampaignSpec::new(2);
        let plans = [
            InvariantWorkerPlan { worker_id: 0, first_global_run: 0, runs: 1 },
            InvariantWorkerPlan { worker_id: 1, first_global_run: 1, runs: 1 },
        ];
        let site = (Address::repeat_byte(0xaa), Selector::from([1, 2, 3, 4]));
        let mut earlier = empty_result(0, 0);
        earlier.handler_errors.insert(site, handler_error(site.0, site.1, 2, "earlier"));
        let mut later = empty_result(0, 0);
        later.handler_errors.insert(site, handler_error(site.0, site.1, 2, "later"));

        let mut aggregator = InvariantCampaignAggregator::new(spec);
        aggregator.push(InvariantWorkerOutput::new(plans[1], later));
        aggregator.push(InvariantWorkerOutput::new(plans[0], earlier));
        let result = aggregator.finish().unwrap();

        let failure = result.handler_errors[&site].as_handler_assertion().unwrap();
        assert_eq!(result.handler_errors.len(), 1);
        assert_eq!(failure.call_sequence.len(), 2);
        assert_eq!(failure.revert_reason, "earlier");
    }

    #[test]
    fn aggregator_keeps_distinct_predicate_failures() {
        let spec = InvariantCampaignSpec::new(2);
        let plans = [
            InvariantWorkerPlan { worker_id: 0, first_global_run: 0, runs: 1 },
            InvariantWorkerPlan { worker_id: 1, first_global_run: 1, runs: 1 },
        ];
        let mut earlier = empty_result(0, 0);
        earlier.errors.insert("invariant_a".to_string(), predicate_error("a", 3));
        let mut later = empty_result(0, 0);
        later.errors.insert("invariant_b".to_string(), predicate_error("b", 2));

        let mut aggregator = InvariantCampaignAggregator::new(spec);
        aggregator.push(InvariantWorkerOutput::new(plans[1], later));
        aggregator.push(InvariantWorkerOutput::new(plans[0], earlier));
        let result = aggregator.finish().unwrap();

        assert_eq!(result.errors.len(), 2);
        assert_eq!(result.errors["invariant_a"].revert_reason().as_deref(), Some("a"));
        assert_eq!(result.errors["invariant_b"].revert_reason().as_deref(), Some("b"));
    }

    #[test]
    fn aggregator_keeps_first_max_optimization_value_on_tie() {
        let spec = InvariantCampaignSpec::new(3);
        let plans = [
            InvariantWorkerPlan { worker_id: 0, first_global_run: 0, runs: 1 },
            InvariantWorkerPlan { worker_id: 1, first_global_run: 1, runs: 1 },
            InvariantWorkerPlan { worker_id: 2, first_global_run: 2, runs: 1 },
        ];
        let mut first = empty_result(0, 0);
        first.optimization_best_value = Some(I256::try_from(7).unwrap());
        first.optimization_best_sequence = sequence(1, 0x10);
        let mut earlier_best = empty_result(0, 0);
        earlier_best.optimization_best_value = Some(I256::try_from(9).unwrap());
        earlier_best.optimization_best_sequence = sequence(1, 0x20);
        let mut later_tie = empty_result(0, 0);
        later_tie.optimization_best_value = Some(I256::try_from(9).unwrap());
        later_tie.optimization_best_sequence = sequence(1, 0x30);

        let mut aggregator = InvariantCampaignAggregator::new(spec);
        aggregator.push(InvariantWorkerOutput::new(plans[2], later_tie));
        aggregator.push(InvariantWorkerOutput::new(plans[0], first));
        aggregator.push(InvariantWorkerOutput::new(plans[1], earlier_best));
        let result = aggregator.finish().unwrap();

        assert_eq!(result.optimization_best_value, Some(I256::try_from(9).unwrap()));
        assert_eq!(result.optimization_best_sequence[0].sender, Address::repeat_byte(0x20));
    }

    #[test]
    fn aggregator_rejects_overlapping_outputs() {
        let spec = InvariantCampaignSpec::new(1);
        let mut aggregator = InvariantCampaignAggregator::new(spec);

        aggregator.push(InvariantWorkerOutput::new(
            InvariantWorkerPlan { worker_id: 0, first_global_run: 0, runs: 1 },
            empty_result(0, 0),
        ));
        aggregator.push(InvariantWorkerOutput::new(
            InvariantWorkerPlan { worker_id: 1, first_global_run: 0, runs: 1 },
            empty_result(0, 0),
        ));
        let err = aggregator.finish().unwrap_err();

        assert!(err.to_string().contains("do not cover the logical campaign"));
    }

    #[test]
    fn aggregator_rejects_duplicate_worker_ids() {
        let spec = InvariantCampaignSpec::new(2);
        let mut aggregator = InvariantCampaignAggregator::new(spec);

        aggregator.push(InvariantWorkerOutput::new(
            InvariantWorkerPlan { worker_id: 0, first_global_run: 0, runs: 1 },
            empty_result(0, 0),
        ));
        aggregator.push(InvariantWorkerOutput::new(
            InvariantWorkerPlan { worker_id: 0, first_global_run: 1, runs: 1 },
            empty_result(0, 0),
        ));
        let err = aggregator.finish().unwrap_err();

        assert!(err.to_string().contains("duplicate invariant worker output"));
    }

    #[test]
    fn aggregator_allows_non_dense_worker_ids_with_contiguous_ranges() {
        let spec = InvariantCampaignSpec::new(2);
        let mut aggregator = InvariantCampaignAggregator::new(spec);

        aggregator.push(InvariantWorkerOutput::new(
            InvariantWorkerPlan { worker_id: 0, first_global_run: 0, runs: 1 },
            empty_result(0, 0),
        ));
        aggregator.push(InvariantWorkerOutput::new(
            InvariantWorkerPlan { worker_id: 2, first_global_run: 1, runs: 1 },
            empty_result(2, 0),
        ));
        let result = aggregator.finish().unwrap();

        assert_eq!(result.reverts, 2);
    }

    #[test]
    fn aggregator_rejects_missing_master_worker() {
        let spec = InvariantCampaignSpec::new(2);
        let mut aggregator = InvariantCampaignAggregator::new(spec);

        aggregator.push(InvariantWorkerOutput::new(
            InvariantWorkerPlan { worker_id: 1, first_global_run: 0, runs: 1 },
            empty_result(0, 0),
        ));
        aggregator.push(InvariantWorkerOutput::new(
            InvariantWorkerPlan { worker_id: 2, first_global_run: 1, runs: 1 },
            empty_result(0, 0),
        ));
        let err = aggregator.finish().unwrap_err();

        assert!(err.to_string().contains("missing invariant master worker output"));
    }

    #[test]
    fn aggregator_uses_master_failed_corpus_replays() {
        let spec = InvariantCampaignSpec::new(2);
        let plans = [
            InvariantWorkerPlan { worker_id: 0, first_global_run: 0, runs: 1 },
            InvariantWorkerPlan { worker_id: 1, first_global_run: 1, runs: 1 },
        ];

        let mut aggregator = InvariantCampaignAggregator::new(spec);
        aggregator.push(InvariantWorkerOutput::new(plans[0], empty_result(0, 7)));
        aggregator.push(InvariantWorkerOutput::new(plans[1], empty_result(0, 1)));
        let result = aggregator.finish().unwrap();

        assert_eq!(result.failed_corpus_replays, 7);
    }

    #[test]
    fn aggregator_uses_master_failed_corpus_replays_independent_of_output_order() {
        let spec = InvariantCampaignSpec::new(2);
        let plans = [
            InvariantWorkerPlan { worker_id: 1, first_global_run: 0, runs: 1 },
            InvariantWorkerPlan { worker_id: 0, first_global_run: 1, runs: 1 },
        ];

        let mut aggregator = InvariantCampaignAggregator::new(spec);
        aggregator.push(InvariantWorkerOutput::new(plans[0], empty_result(0, 0)));
        aggregator.push(InvariantWorkerOutput::new(plans[1], empty_result(0, 7)));
        let result = aggregator.finish().unwrap();

        assert_eq!(result.failed_corpus_replays, 7);
    }

    #[test]
    fn aggregator_rejects_plan_that_does_not_cover_campaign() {
        let spec = InvariantCampaignSpec::new(2);
        let plan = InvariantWorkerPlan { worker_id: 0, first_global_run: 0, runs: 1 };
        let worker = InvariantWorkerOutput::new(plan, empty_result(0, 0));

        let mut aggregator = InvariantCampaignAggregator::new(spec);
        aggregator.push(worker);
        let err = aggregator.finish().unwrap_err();

        assert!(err.to_string().contains("do not cover the logical campaign"));
    }

    #[test]
    fn aggregator_rejects_missing_output() {
        let aggregator = InvariantCampaignAggregator::new(InvariantCampaignSpec::new(1));
        let err = aggregator.finish().unwrap_err();

        assert!(err.to_string().contains("missing invariant worker output"));
    }
}
