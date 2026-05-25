use super::InvariantFuzzTestResult;
use alloy_primitives::U256;
#[cfg(debug_assertions)]
use alloy_primitives::{Address, I256, Selector};

/// Stable identity for one logical invariant run inside a contract-level campaign.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvariantRunId {
    /// Zero-based run index in the logical campaign.
    pub global_run: u32,
    /// Worker that executed the run.
    pub worker_id: u32,
    /// Zero-based run index inside the worker.
    pub worker_run: u32,
    /// Base seed used for the campaign's input stream.
    pub seed: Option<U256>,
}

impl InvariantRunId {
    pub const fn new(global_run: u32, worker_id: u32, worker_run: u32, seed: Option<U256>) -> Self {
        Self { global_run, worker_id, worker_run, seed }
    }
}

/// Immutable plan-level description for an invariant campaign.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvariantCampaignSpec {
    /// Total logical runs configured for the campaign.
    pub total_runs: u32,
    /// Base seed used to derive worker/run provenance.
    pub seed: Option<U256>,
}

impl InvariantCampaignSpec {
    pub const fn new(total_runs: u32, seed: Option<U256>) -> Self {
        Self { total_runs, seed }
    }

    /// Current PR1 execution shape: the whole logical campaign is assigned to worker 0.
    pub const fn single_worker_plan(&self) -> InvariantWorkerPlan {
        InvariantWorkerPlan {
            worker_id: 0,
            first_global_run: 0,
            runs: self.total_runs,
            seed: self.seed,
        }
    }
}

/// Static assignment of a contiguous logical run range to one worker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvariantWorkerPlan {
    pub worker_id: u32,
    pub first_global_run: u32,
    pub runs: u32,
    pub seed: Option<U256>,
}

impl InvariantWorkerPlan {
    pub const fn run_id(&self, worker_run: u32) -> InvariantRunId {
        InvariantRunId::new(
            self.first_global_run + worker_run,
            self.worker_id,
            worker_run,
            self.seed,
        )
    }

    #[cfg(debug_assertions)]
    pub fn contains(&self, run_id: InvariantRunId) -> bool {
        let same_worker = run_id.worker_id == self.worker_id;
        let same_seed = run_id.seed == self.seed;
        let worker_run_in_range = run_id.worker_run < self.runs;
        let global_run_matches = run_id.global_run == self.first_global_run + run_id.worker_run;
        same_worker && same_seed && worker_run_in_range && global_run_matches
    }
}

/// Output produced by one invariant worker.
#[derive(Debug)]
pub struct InvariantWorkerOutput {
    pub plan: InvariantWorkerPlan,
    #[cfg(debug_assertions)]
    pub runs: Vec<InvariantRunOutput>,
    #[cfg(debug_assertions)]
    pub failures: Vec<InvariantFailureOutput>,
    pub result: InvariantFuzzTestResult,
}

impl InvariantWorkerOutput {
    #[cfg(debug_assertions)]
    pub const fn new(
        plan: InvariantWorkerPlan,
        runs: Vec<InvariantRunOutput>,
        failures: Vec<InvariantFailureOutput>,
        result: InvariantFuzzTestResult,
    ) -> Self {
        Self { plan, runs, failures, result }
    }

    #[cfg(not(debug_assertions))]
    pub const fn new(plan: InvariantWorkerPlan, result: InvariantFuzzTestResult) -> Self {
        Self { plan, result }
    }
}

/// Worker-local summary for one completed logical run.
#[cfg(debug_assertions)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvariantRunOutput {
    pub id: InvariantRunId,
    pub calls: usize,
    pub reverts: usize,
    pub new_coverage: bool,
    pub optimization_value: Option<I256>,
}

#[cfg(debug_assertions)]
impl InvariantRunOutput {
    pub const fn new(
        id: InvariantRunId,
        calls: usize,
        reverts: usize,
        new_coverage: bool,
        optimization_value: Option<I256>,
    ) -> Self {
        Self { id, calls, reverts, new_coverage, optimization_value }
    }
}

/// Stable source for one failure candidate emitted by a worker.
#[cfg(debug_assertions)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvariantFailureOutput {
    pub id: InvariantRunId,
    pub kind: InvariantFailureKind,
}

#[cfg(debug_assertions)]
impl InvariantFailureOutput {
    pub const fn new(id: InvariantRunId, kind: InvariantFailureKind) -> Self {
        Self { id, kind }
    }
}

/// Logical failure key kept independent from the renderer-facing error payload.
#[cfg(debug_assertions)]
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum InvariantFailureKind {
    Predicate(String),
    Handler { reverter: Address, selector: Selector },
}

/// Merges worker outputs back into one logical invariant campaign result.
#[derive(Debug)]
pub struct InvariantCampaignAggregator {
    spec: InvariantCampaignSpec,
    output: Option<InvariantWorkerOutput>,
}

impl InvariantCampaignAggregator {
    pub const fn new(spec: InvariantCampaignSpec) -> Self {
        Self { spec, output: None }
    }

    pub fn push(&mut self, output: InvariantWorkerOutput) {
        debug_assert!(self.output.is_none(), "PR1 only wires the single-worker identity path");
        self.output = Some(output);
    }

    pub fn finish(self) -> InvariantFuzzTestResult {
        let output = self.output.expect("at least one invariant worker output");
        debug_assert_eq!(output.plan.runs, self.spec.total_runs);
        debug_assert_eq!(output.plan.seed, self.spec.seed);
        #[cfg(debug_assertions)]
        {
            debug_assert!(output.runs.iter().enumerate().all(|(expected, run)| {
                run.id.global_run == expected as u32
                    && run.id.worker_id == output.plan.worker_id
                    && run.id.seed == output.plan.seed
                    && run.id.global_run == output.plan.first_global_run + run.id.worker_run
            }));
            debug_assert!(output.failures.iter().all(|failure| output.plan.contains(failure.id)));
        }
        output.result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn single_worker_plan_preserves_logical_run_identity() {
        let seed = Some(U256::from(0x1234));
        let spec = InvariantCampaignSpec::new(3, seed);
        let plan = spec.single_worker_plan();

        assert_eq!(plan.worker_id, 0);
        assert_eq!(plan.first_global_run, 0);
        assert_eq!(plan.runs, 3);
        assert_eq!(plan.seed, seed);
        assert_eq!(plan.run_id(2), InvariantRunId::new(2, 0, 2, seed));
    }

    #[test]
    fn aggregator_returns_single_worker_result_without_rewriting() {
        let seed = Some(U256::from(7));
        let spec = InvariantCampaignSpec::new(1, seed);
        let plan = spec.single_worker_plan();
        let run_id = plan.run_id(0);
        let result = InvariantFuzzTestResult::new(
            HashMap::default(),
            HashMap::default(),
            Vec::new(),
            2,
            Vec::new(),
            Vec::new(),
            None,
            HashMap::default(),
            3,
            None,
            Vec::new(),
        );
        let worker = InvariantWorkerOutput::new(
            plan,
            vec![InvariantRunOutput::new(run_id, 4, 2, true, None)],
            vec![InvariantFailureOutput::new(
                run_id,
                InvariantFailureKind::Predicate("invariant_ok".to_string()),
            )],
            result,
        );

        let mut aggregator = InvariantCampaignAggregator::new(spec);
        aggregator.push(worker);
        let result = aggregator.finish();

        assert_eq!(result.reverts, 2);
        assert_eq!(result.failed_corpus_replays, 3);
    }

    #[test]
    fn aggregator_allows_failure_from_aborted_run() {
        let seed = Some(U256::from(9));
        let spec = InvariantCampaignSpec::new(2, seed);
        let plan = spec.single_worker_plan();
        let completed = plan.run_id(0);
        let aborted = plan.run_id(1);
        let result = InvariantFuzzTestResult::new(
            HashMap::default(),
            HashMap::default(),
            Vec::new(),
            1,
            Vec::new(),
            Vec::new(),
            None,
            HashMap::default(),
            0,
            None,
            Vec::new(),
        );
        let worker = InvariantWorkerOutput::new(
            plan,
            vec![InvariantRunOutput::new(completed, 1, 0, false, None)],
            vec![InvariantFailureOutput::new(
                aborted,
                InvariantFailureKind::Predicate("invariant_aborted".to_string()),
            )],
            result,
        );

        let mut aggregator = InvariantCampaignAggregator::new(spec);
        aggregator.push(worker);
        let result = aggregator.finish();

        assert_eq!(result.reverts, 1);
    }
}
