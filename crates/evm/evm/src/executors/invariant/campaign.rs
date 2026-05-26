use super::InvariantFuzzTestResult;
use eyre::{Result, ensure};

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
///
/// TODO: extend this to merge multiple worker outputs once invariant execution is parallelized.
#[derive(Debug)]
pub struct InvariantCampaignAggregator {
    spec: InvariantCampaignSpec,
    output: Option<InvariantWorkerOutput>,
}

impl InvariantCampaignAggregator {
    pub const fn new(spec: InvariantCampaignSpec) -> Self {
        Self { spec, output: None }
    }

    pub fn push(&mut self, output: InvariantWorkerOutput) -> Result<()> {
        ensure!(
            self.output.is_none(),
            "single-worker invariant aggregator received multiple outputs"
        );
        ensure!(
            self.spec.worker_plans(1)?.first().is_some_and(|plan| *plan == output.plan),
            "single-worker invariant output does not cover the logical campaign"
        );
        self.output = Some(output);
        Ok(())
    }

    pub fn finish(self) -> Result<InvariantFuzzTestResult> {
        let output = self.output.ok_or_else(|| eyre::eyre!("missing invariant worker output"))?;
        Ok(output.result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

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
    fn aggregator_rejects_multiple_outputs() {
        let spec = InvariantCampaignSpec::new(1);
        let mut aggregator = InvariantCampaignAggregator::new(spec);

        aggregator
            .push(InvariantWorkerOutput::new(one_worker_plan(spec), empty_result(0, 0)))
            .unwrap();
        let err = aggregator
            .push(InvariantWorkerOutput::new(one_worker_plan(spec), empty_result(0, 0)))
            .unwrap_err();

        assert!(err.to_string().contains("received multiple outputs"));
    }

    #[test]
    fn aggregator_rejects_plan_that_does_not_cover_campaign() {
        let spec = InvariantCampaignSpec::new(2);
        let plan = InvariantWorkerPlan { worker_id: 0, first_global_run: 0, runs: 1 };
        let worker = InvariantWorkerOutput::new(plan, empty_result(0, 0));

        let mut aggregator = InvariantCampaignAggregator::new(spec);
        let err = aggregator.push(worker).unwrap_err();

        assert!(err.to_string().contains("does not cover the logical campaign"));
    }

    #[test]
    fn aggregator_rejects_missing_output() {
        let aggregator = InvariantCampaignAggregator::new(InvariantCampaignSpec::new(1));
        let err = aggregator.finish().unwrap_err();

        assert!(err.to_string().contains("missing invariant worker output"));
    }
}
