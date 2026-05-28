use super::{InvariantFuzzError, InvariantFuzzTestResult, InvariantMetrics};
use alloy_primitives::{Address, I256, Selector};
use eyre::{Result, ensure};
use foundry_evm_coverage::HitMaps;
use foundry_evm_fuzz::BasicTxDetails;
use proptest::test_runner::TestError;
use std::collections::HashMap;

/// Immutable plan-level description for an invariant campaign.
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

        let worker_count = workers.min(self.total_runs as usize);
        let base_runs = self.total_runs / worker_count as u32;
        let extra_runs = self.total_runs % worker_count as u32;

        let mut first_global_run = 0;
        let mut plans = Vec::with_capacity(worker_count);
        for worker_id in 0..worker_count {
            let runs = base_runs + u32::from((worker_id as u32) < extra_runs);
            plans.push(InvariantWorkerPlan { worker_id: worker_id as u32, first_global_run, runs });
            first_global_run += runs;
        }

        debug_assert_eq!(first_global_run, self.total_runs);
        Ok(plans)
    }
}

/// Static assignment of a contiguous logical run range to one worker.
///
/// The assigned range is `[first_global_run, first_global_run + runs)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvariantWorkerPlan {
    pub worker_id: u32,
    pub first_global_run: u32,
    pub runs: u32,
}

/// Output produced by one invariant worker.
#[derive(Debug)]
pub struct InvariantWorkerOutput {
    pub plan: InvariantWorkerPlan,
    pub result: InvariantFuzzTestResult,
}

impl InvariantWorkerOutput {
    pub const fn new(plan: InvariantWorkerPlan, result: InvariantFuzzTestResult) -> Self {
        Self { plan, result }
    }
}

/// Merges worker outputs back into one logical invariant campaign result.
#[derive(Debug)]
pub struct InvariantCampaignAggregator {
    spec: InvariantCampaignSpec,
    outputs: Vec<InvariantWorkerOutput>,
}

impl InvariantCampaignAggregator {
    pub const fn new(spec: InvariantCampaignSpec) -> Self {
        Self { spec, outputs: Vec::new() }
    }

    pub fn push(&mut self, output: InvariantWorkerOutput) -> Result<()> {
        let end = worker_range_end(output.plan)?;
        ensure!(
            end <= self.spec.total_runs,
            "invariant worker output exceeds the logical campaign"
        );
        self.outputs.push(output);
        Ok(())
    }

    pub fn finish(mut self) -> Result<InvariantFuzzTestResult> {
        ensure!(!self.outputs.is_empty(), "missing invariant worker output");

        self.outputs.sort_by_key(|output| (output.plan.first_global_run, output.plan.worker_id));
        ensure_outputs_cover_campaign(self.spec, &self.outputs)?;

        let mut errors = HashMap::default();
        let mut error_choices = HashMap::default();
        let mut handler_errors = HashMap::default();
        let mut cases = Vec::new();
        let mut reverts = 0;
        let mut last_run_inputs = Vec::new();
        let mut gas_report_traces = Vec::new();
        let mut line_coverage = None;
        let mut metrics = HashMap::default();
        let failed_corpus_replays = self.outputs[0].result.failed_corpus_replays;
        let mut optimization_best_value = None;
        let mut optimization_best_sequence = Vec::new();
        let mut optimization_best_key = None;

        for output in self.outputs {
            let plan = output.plan;
            let run_key =
                RunChoice { first_global_run: plan.first_global_run, worker_id: plan.worker_id };
            let result = output.result;

            merge_predicate_errors(&mut errors, &mut error_choices, result.errors, run_key);
            merge_handler_errors(&mut handler_errors, result.handler_errors);
            cases.extend(result.cases);
            reverts += result.reverts;
            last_run_inputs = result.last_run_inputs;
            gas_report_traces.extend(result.gas_report_traces);
            HitMaps::merge_opt(&mut line_coverage, result.line_coverage);
            merge_metrics(&mut metrics, result.metrics);
            merge_optimization(
                &mut optimization_best_value,
                &mut optimization_best_sequence,
                &mut optimization_best_key,
                result.optimization_best_value,
                result.optimization_best_sequence,
                run_key,
            );
        }

        Ok(InvariantFuzzTestResult::new(
            errors,
            handler_errors,
            cases,
            reverts,
            last_run_inputs,
            gas_report_traces,
            line_coverage,
            metrics,
            failed_corpus_replays,
            optimization_best_value,
            optimization_best_sequence,
        ))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct RunChoice {
    first_global_run: u32,
    worker_id: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct PredicateFailureChoice {
    run: RunChoice,
    sequence_len: usize,
}

fn worker_range_end(plan: InvariantWorkerPlan) -> Result<u32> {
    plan.first_global_run
        .checked_add(plan.runs)
        .ok_or_else(|| eyre::eyre!("invariant worker output range overflows"))
}

fn ensure_outputs_cover_campaign(
    spec: InvariantCampaignSpec,
    outputs: &[InvariantWorkerOutput],
) -> Result<()> {
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
        next_global_run = worker_range_end(output.plan)?;
    }

    ensure!(
        next_global_run == spec.total_runs,
        "invariant worker outputs do not cover the logical campaign"
    );
    Ok(())
}

fn merge_predicate_errors(
    merged: &mut HashMap<String, InvariantFuzzError>,
    choices: &mut HashMap<String, PredicateFailureChoice>,
    worker_errors: HashMap<String, InvariantFuzzError>,
    run: RunChoice,
) {
    for (invariant, error) in worker_errors {
        let candidate = PredicateFailureChoice { run, sequence_len: error_sequence_len(&error) };
        if choices.get(&invariant).is_none_or(|existing| candidate < *existing) {
            choices.insert(invariant.clone(), candidate);
            merged.insert(invariant, error);
        }
    }
}

fn merge_handler_errors(
    merged: &mut HashMap<(Address, Selector), InvariantFuzzError>,
    worker_errors: HashMap<(Address, Selector), InvariantFuzzError>,
) {
    for (site, error) in worker_errors {
        let candidate_len = error_sequence_len(&error);
        match merged.get_mut(&site) {
            Some(existing) if error_sequence_len(existing) <= candidate_len => {}
            Some(existing) => *existing = error,
            None => {
                merged.insert(site, error);
            }
        }
    }
}

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

fn merge_optimization(
    best_value: &mut Option<I256>,
    best_sequence: &mut Vec<BasicTxDetails>,
    best_key: &mut Option<RunChoice>,
    candidate_value: Option<I256>,
    candidate_sequence: Vec<BasicTxDetails>,
    candidate_key: RunChoice,
) {
    let Some(candidate_value) = candidate_value else {
        return;
    };

    let should_replace = best_value.is_none_or(|best_value| candidate_value > best_value)
        || best_value.is_some_and(|best_value| {
            candidate_value == best_value
                && best_key.is_none_or(|best_key| candidate_key < best_key)
        });

    if should_replace {
        *best_value = Some(candidate_value);
        *best_sequence = candidate_sequence;
        *best_key = Some(candidate_key);
    }
}

const fn error_sequence_len(error: &InvariantFuzzError) -> usize {
    match error {
        InvariantFuzzError::BrokenInvariant(case_data) | InvariantFuzzError::Revert(case_data) => {
            match &case_data.test_error {
                TestError::Fail(_, sequence) => sequence.len(),
                TestError::Abort(_) => usize::MAX,
            }
        }
        InvariantFuzzError::HandlerAssertion(failure) => failure.call_sequence.len(),
        InvariantFuzzError::MaxAssumeRejects(_) => usize::MAX,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        super::error::{FailedInvariantCaseData, HandlerAssertionFailure},
        *,
    };
    use alloy_primitives::{B256, Bytes};
    use foundry_evm_coverage::HitMap;
    use foundry_evm_fuzz::{CallDetails, FuzzCase, FuzzedCases};
    use revm_inspectors::tracing::CallTraceArena;

    fn empty_result(reverts: usize, failed_corpus_replays: usize) -> InvariantFuzzTestResult {
        InvariantFuzzTestResult::new(
            HashMap::default(),
            HashMap::default(),
            Vec::new(),
            reverts,
            Vec::new(),
            Vec::new(),
            None,
            HashMap::default(),
            failed_corpus_replays,
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

    fn worker_result(
        case_gas: u64,
        reverts: usize,
        last_input_sender: u8,
        metric_name: &str,
        metrics: InvariantMetrics,
        coverage_hits: u32,
        failed_corpus_replays: usize,
    ) -> InvariantFuzzTestResult {
        let mut result = empty_result(reverts, failed_corpus_replays);
        result.cases.push(FuzzedCases::new(vec![FuzzCase { gas: case_gas, stipend: 0 }]));
        result.last_run_inputs = vec![basic_tx(last_input_sender)];
        result.gas_report_traces.push(vec![CallTraceArena::default()]);
        result.line_coverage = Some(hit_maps(7, coverage_hits));
        result.metrics.insert(metric_name.to_string(), metrics);
        result
    }

    fn sequence(len: usize, first_sender: u8) -> Vec<BasicTxDetails> {
        (0..len).map(|idx| basic_tx(first_sender.wrapping_add(idx as u8))).collect()
    }

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

    fn one_worker_plan(spec: InvariantCampaignSpec) -> InvariantWorkerPlan {
        spec.worker_plans(1).unwrap().pop().unwrap()
    }

    #[test]
    fn worker_plans_cover_logical_campaign_with_one_worker() {
        let plan = one_worker_plan(InvariantCampaignSpec::new(3));

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
    }

    #[test]
    fn aggregator_returns_single_worker_result_without_rewriting() {
        let spec = InvariantCampaignSpec::new(1);
        let worker = InvariantWorkerOutput::new(one_worker_plan(spec), empty_result(2, 3));

        let mut aggregator = InvariantCampaignAggregator::new(spec);
        aggregator.push(worker).unwrap();
        let result = aggregator.finish().unwrap();

        assert_eq!(result.reverts, 2);
        assert_eq!(result.failed_corpus_replays, 3);
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
        aggregator
            .push(InvariantWorkerOutput::new(
                plans[2],
                worker_result(
                    30,
                    3,
                    0x30,
                    "transfer(address)",
                    InvariantMetrics { calls: 3, reverts: 1, discards: 0 },
                    3,
                    0,
                ),
            ))
            .unwrap();
        aggregator
            .push(InvariantWorkerOutput::new(
                plans[0],
                worker_result(
                    10,
                    1,
                    0x10,
                    "transfer(address)",
                    InvariantMetrics { calls: 1, reverts: 0, discards: 2 },
                    1,
                    4,
                ),
            ))
            .unwrap();
        aggregator
            .push(InvariantWorkerOutput::new(
                plans[1],
                worker_result(
                    20,
                    2,
                    0x20,
                    "approve(address)",
                    InvariantMetrics { calls: 2, reverts: 1, discards: 1 },
                    2,
                    0,
                ),
            ))
            .unwrap();

        let result = aggregator.finish().unwrap();

        let merged_case_gas =
            result.cases.iter().map(|cases| cases.last().unwrap().gas).collect::<Vec<_>>();
        assert_eq!(merged_case_gas, vec![10, 20, 30]);
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
        aggregator.push(InvariantWorkerOutput::new(plans[1], later)).unwrap();
        aggregator.push(InvariantWorkerOutput::new(plans[0], earlier)).unwrap();
        let result = aggregator.finish().unwrap();

        assert_eq!(result.errors.len(), 1);
        assert_eq!(result.errors["invariant_balance"].revert_reason().as_deref(), Some("earlier"));
    }

    #[test]
    fn predicate_failure_choice_uses_worker_id_then_sequence_length_after_run() {
        let mut merged = HashMap::default();
        let mut choices = HashMap::default();

        let mut worker_errors = HashMap::default();
        worker_errors.insert("invariant_balance".to_string(), predicate_error("worker-2", 1));
        merge_predicate_errors(
            &mut merged,
            &mut choices,
            worker_errors,
            RunChoice { first_global_run: 5, worker_id: 2 },
        );

        let mut worker_errors = HashMap::default();
        worker_errors.insert("invariant_balance".to_string(), predicate_error("worker-1", 4));
        merge_predicate_errors(
            &mut merged,
            &mut choices,
            worker_errors,
            RunChoice { first_global_run: 5, worker_id: 1 },
        );
        assert_eq!(merged["invariant_balance"].revert_reason().as_deref(), Some("worker-1"));

        let mut worker_errors = HashMap::default();
        worker_errors.insert("invariant_balance".to_string(), predicate_error("shorter", 2));
        merge_predicate_errors(
            &mut merged,
            &mut choices,
            worker_errors,
            RunChoice { first_global_run: 5, worker_id: 1 },
        );
        assert_eq!(merged["invariant_balance"].revert_reason().as_deref(), Some("shorter"));
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
        aggregator.push(InvariantWorkerOutput::new(plans[1], shorter)).unwrap();
        aggregator.push(InvariantWorkerOutput::new(plans[0], longer)).unwrap();
        let result = aggregator.finish().unwrap();

        let failure = result.handler_errors[&site].as_handler_assertion().unwrap();
        assert_eq!(result.handler_errors.len(), 1);
        assert_eq!(failure.call_sequence.len(), 2);
        assert_eq!(failure.revert_reason, "shorter");
    }

    #[test]
    fn aggregator_keeps_max_optimization_value_and_earlier_tie() {
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
        aggregator.push(InvariantWorkerOutput::new(plans[2], later_tie)).unwrap();
        aggregator.push(InvariantWorkerOutput::new(plans[0], first)).unwrap();
        aggregator.push(InvariantWorkerOutput::new(plans[1], earlier_best)).unwrap();
        let result = aggregator.finish().unwrap();

        assert_eq!(result.optimization_best_value, Some(I256::try_from(9).unwrap()));
        assert_eq!(result.optimization_best_sequence[0].sender, Address::repeat_byte(0x20));
    }

    #[test]
    fn aggregator_rejects_overlapping_outputs() {
        let spec = InvariantCampaignSpec::new(1);
        let mut aggregator = InvariantCampaignAggregator::new(spec);

        aggregator
            .push(InvariantWorkerOutput::new(one_worker_plan(spec), empty_result(0, 0)))
            .unwrap();
        aggregator
            .push(InvariantWorkerOutput::new(one_worker_plan(spec), empty_result(0, 0)))
            .unwrap();
        let err = aggregator.finish().unwrap_err();

        assert!(err.to_string().contains("do not cover the logical campaign"));
    }

    #[test]
    fn aggregator_rejects_plan_that_does_not_cover_campaign() {
        let spec = InvariantCampaignSpec::new(2);
        let plan = InvariantWorkerPlan { worker_id: 0, first_global_run: 0, runs: 1 };
        let worker = InvariantWorkerOutput::new(plan, empty_result(0, 0));

        let mut aggregator = InvariantCampaignAggregator::new(spec);
        aggregator.push(worker).unwrap();
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
