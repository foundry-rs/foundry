//! Serialized evaluation results and their per-split aggregation.

use super::{
    forge_json::TestRecord,
    lcov::CoverageStats,
    manifest::{Budget, Source, Split},
    stats::Estimate,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Version of the `results.json` layout.
pub const SCHEMA_VERSION: u32 = 1;

/// Full output of one evaluation, written as `results.json`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EvalResults {
    pub schema_version: u32,
    /// Optional human-readable label, such as `baseline` or a treatment name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub created_at: String,
    pub manifest: String,
    pub forge_bin: String,
    pub forge_version: String,
    pub extra_forge_args: Vec<String>,
    pub seeds: Vec<u64>,
    pub coverage: bool,
    pub targets: Vec<TargetResult>,
    /// Per-split aggregates, derived from `targets`.
    pub splits: BTreeMap<Split, SplitSummary>,
}

/// All seeds of one target.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TargetResult {
    pub name: String,
    pub split: Split,
    pub source: Source,
    pub expected_failures: Vec<String>,
    pub failures_are_bugs: bool,
    /// Effective budget after CLI and manifest defaults.
    pub budget: Budget,
    pub runs: Vec<SeedRun>,
}

/// Outcome category of a forge invocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    /// Forge produced parseable results (tests may have failed).
    Ok,
    /// Forge exceeded the per-invocation timeout and was killed.
    TimedOut,
    /// Forge produced no usable output, or the project could not be prepared.
    Error,
}

impl RunStatus {
    /// Human-readable status.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::TimedOut => "timed out",
            Self::Error => "error",
        }
    }
}

/// One `forge test` run, and optionally one `forge coverage` run, for a seed.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SeedRun {
    pub seed: u64,
    pub status: RunStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    pub wall_secs: f64,
    pub tests: Vec<TestRecord>,
    /// Expected failures (or, with `failures_are_bugs`, failing tests) that failed in this run.
    pub bugs_found: Vec<String>,
    /// Failing tests that are not expected failures.
    pub unexpected_failures: Vec<String>,
    /// Expected failures that matched no test in the output.
    pub missing_expected: Vec<String>,
    /// Smallest reported duration among the tests that found a bug. Forge reports per-test
    /// durations, so this is an upper bound on time-to-first-failure that includes setup and any
    /// shrinking.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_failure_secs: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coverage: Option<CoverageRun>,
}

/// One `forge coverage` run.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CoverageRun {
    pub status: RunStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub wall_secs: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stats: Option<CoverageStats>,
}

impl SeedRun {
    /// Branch coverage percentage, if coverage was collected.
    pub fn branch_pct(&self) -> Option<f64> {
        self.coverage_stats().and_then(|stats| stats.branch_pct())
    }

    /// Line coverage percentage, if coverage was collected.
    pub fn line_pct(&self) -> Option<f64> {
        self.coverage_stats().and_then(|stats| stats.line_pct())
    }

    /// Function coverage percentage, if coverage was collected.
    pub fn function_pct(&self) -> Option<f64> {
        self.coverage_stats().and_then(|stats| stats.function_pct())
    }

    fn coverage_stats(&self) -> Option<&CoverageStats> {
        self.coverage.as_ref().and_then(|coverage| coverage.stats.as_ref())
    }
}

impl TargetResult {
    /// Number of enumerated bugs, or `None` for `failures_are_bugs` targets.
    pub fn expected_bugs(&self) -> Option<usize> {
        (!self.failures_are_bugs).then_some(self.expected_failures.len())
    }

    /// Bugs found per seed. Timed-out and errored runs count as finding nothing.
    pub fn bugs_found(&self) -> Vec<f64> {
        self.runs.iter().map(|run| run.bugs_found.len() as f64).collect()
    }

    /// Branch coverage per seed, skipping seeds without coverage data.
    pub fn branch_pcts(&self) -> Vec<f64> {
        self.runs.iter().filter_map(SeedRun::branch_pct).collect()
    }

    /// Line coverage per seed, skipping seeds without coverage data.
    pub fn line_pcts(&self) -> Vec<f64> {
        self.runs.iter().filter_map(SeedRun::line_pct).collect()
    }

    /// `forge test` wall time per seed.
    pub fn wall_secs(&self) -> Vec<f64> {
        self.runs.iter().map(|run| run.wall_secs).collect()
    }

    /// Time-to-first-failure per seed, for seeds that found a bug.
    pub fn first_failure_secs(&self) -> Vec<f64> {
        self.runs.iter().filter_map(|run| run.first_failure_secs).collect()
    }

    /// Number of seeds that found each expected bug.
    pub fn found_counts(&self) -> Vec<(String, usize)> {
        self.expected_failures
            .iter()
            .map(|bug| {
                (bug.clone(), self.runs.iter().filter(|run| run.bugs_found.contains(bug)).count())
            })
            .collect()
    }

    /// Function coverage per seed, skipping seeds without coverage data.
    pub fn function_pcts(&self) -> Vec<f64> {
        self.runs.iter().filter_map(SeedRun::function_pct).collect()
    }
}

/// Split-level metrics for one seed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SplitSeedMetrics {
    pub seed: u64,
    /// Bugs found summed over the split's targets.
    pub bugs_found: f64,
    /// `bugs_found` over expected bugs, for targets with enumerated bugs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bug_fraction: Option<f64>,
    /// Branch coverage averaged over targets (macro average, in percent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_coverage: Option<f64>,
    /// Line coverage averaged over targets (macro average, in percent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_coverage: Option<f64>,
    /// `forge test` wall time summed over the split's targets.
    pub wall_secs: f64,
}

/// Aggregates across seeds for one split.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SplitSummary {
    pub targets: usize,
    pub expected_bugs: usize,
    pub per_seed: Vec<SplitSeedMetrics>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bugs_found: Option<Estimate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bug_fraction: Option<Estimate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_coverage: Option<Estimate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_coverage: Option<Estimate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wall_secs: Option<Estimate>,
}

/// Selects one per-seed metric of a split.
#[derive(Clone, Copy, Debug)]
pub enum Metric {
    BugsFound,
    BugFraction,
    BranchCoverage,
    LineCoverage,
    WallSecs,
}

impl Metric {
    /// Value of this metric for one seed.
    pub const fn value(self, metrics: &SplitSeedMetrics) -> Option<f64> {
        match self {
            Self::BugsFound => Some(metrics.bugs_found),
            Self::BugFraction => metrics.bug_fraction,
            Self::BranchCoverage => metrics.branch_coverage,
            Self::LineCoverage => metrics.line_coverage,
            Self::WallSecs => Some(metrics.wall_secs),
        }
    }
}

impl SplitSummary {
    /// Aggregates the targets of one split. Seeds are aligned by position in `seeds`.
    pub fn compute(targets: &[&TargetResult], seeds: &[u64]) -> Self {
        let expected_bugs = targets.iter().filter_map(|target| target.expected_bugs()).sum();
        let per_seed = seeds
            .iter()
            .map(|&seed| {
                let runs = targets
                    .iter()
                    .filter_map(|target| {
                        target.runs.iter().find(|run| run.seed == seed).map(|run| (*target, run))
                    })
                    .collect::<Vec<_>>();
                let enumerated_found = runs
                    .iter()
                    .filter(|(target, _)| !target.failures_are_bugs)
                    .map(|(_, run)| run.bugs_found.len())
                    .sum::<usize>();
                SplitSeedMetrics {
                    seed,
                    bugs_found: runs.iter().map(|(_, run)| run.bugs_found.len() as f64).sum(),
                    bug_fraction: (expected_bugs > 0)
                        .then(|| enumerated_found as f64 / expected_bugs as f64),
                    branch_coverage: mean(runs.iter().filter_map(|(_, run)| run.branch_pct())),
                    line_coverage: mean(runs.iter().filter_map(|(_, run)| run.line_pct())),
                    wall_secs: runs.iter().map(|(_, run)| run.wall_secs).sum(),
                }
            })
            .collect::<Vec<_>>();
        let estimate = |metric: Metric| {
            Estimate::from_samples(
                &per_seed.iter().filter_map(|m| metric.value(m)).collect::<Vec<_>>(),
            )
        };
        Self {
            targets: targets.len(),
            expected_bugs,
            bugs_found: estimate(Metric::BugsFound),
            bug_fraction: estimate(Metric::BugFraction),
            branch_coverage: estimate(Metric::BranchCoverage),
            line_coverage: estimate(Metric::LineCoverage),
            wall_secs: estimate(Metric::WallSecs),
            per_seed,
        }
    }

    /// Per-seed values of `metric`, with their seeds.
    pub fn samples(&self, metric: Metric) -> Vec<(u64, f64)> {
        self.per_seed.iter().filter_map(|m| metric.value(m).map(|value| (m.seed, value))).collect()
    }
}

impl EvalResults {
    /// Recomputes [`Self::splits`] from [`Self::targets`].
    pub fn summarize(&mut self) {
        self.splits = Split::ALL
            .into_iter()
            .filter_map(|split| {
                let targets =
                    self.targets.iter().filter(|target| target.split == split).collect::<Vec<_>>();
                (!targets.is_empty()).then(|| (split, SplitSummary::compute(&targets, &self.seeds)))
            })
            .collect();
    }
}

fn mean(values: impl Iterator<Item = f64>) -> Option<f64> {
    let (sum, count) = values.fold((0.0, 0usize), |(sum, count), value| (sum + value, count + 1));
    (count > 0).then(|| sum / count as f64)
}
