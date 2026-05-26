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

    /// Current PR1 execution shape: the whole logical campaign is assigned to worker 0.
    pub const fn single_worker_plan(self) -> InvariantWorkerPlan {
        InvariantWorkerPlan { worker_id: 0, first_global_run: 0, runs: self.total_runs }
    }
}

/// Static assignment of a contiguous logical run range to one worker.
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
/// PR1 keeps the execution model single-worker while making the campaign/result boundary explicit.
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
            output.plan == self.spec.single_worker_plan(),
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

    #[test]
    fn single_worker_plan_covers_logical_campaign() {
        let spec = InvariantCampaignSpec::new(3);
        let plan = spec.single_worker_plan();

        assert_eq!(plan.worker_id, 0);
        assert_eq!(plan.first_global_run, 0);
        assert_eq!(plan.runs, 3);
    }

    #[test]
    fn aggregator_returns_single_worker_result_without_rewriting() {
        let spec = InvariantCampaignSpec::new(1);
        let worker = InvariantWorkerOutput::new(spec.single_worker_plan(), empty_result(2, 3));

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
            .push(InvariantWorkerOutput::new(spec.single_worker_plan(), empty_result(0, 0)))
            .unwrap();
        let err = aggregator
            .push(InvariantWorkerOutput::new(spec.single_worker_plan(), empty_result(0, 0)))
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
