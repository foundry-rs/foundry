//! Markdown summaries and baseline comparison.

use super::{
    manifest::Split,
    results::{EvalResults, Metric, RunStatus, SplitSummary, TargetResult},
    stats::{Difference, Estimate},
};
use std::{collections::BTreeSet, fmt, fmt::Write};

/// Bugs-found standard deviation across seeds above which a target is flagged as noisy.
const HIGH_BUG_STD_DEV: f64 = 0.4;
/// Branch coverage standard deviation (percentage points) above which a target is flagged.
const HIGH_BRANCH_STD_DEV: f64 = 5.0;
/// Maximum length of an error message in the summary.
const MAX_ERROR_LEN: usize = 240;

/// Renders `SUMMARY.md`.
pub fn render_summary(results: &EvalResults) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# Fuzz eval summary\n");
    write_run_info(&mut out, results);
    let _ = writeln!(
        out,
        "- Intervals: two-sided 95% Student t-intervals across seeds, shown as `mean [low, high]`. \
         Split coverage is the per-seed mean over the split's targets.\n"
    );

    let _ = writeln!(out, "## Splits\n");
    let _ = writeln!(
        out,
        "| Split | Targets | Expected bugs | Bugs found | Fraction of expected | Branch coverage | Line coverage | Wall time (s) |"
    );
    let _ = writeln!(out, "| --- | ---: | ---: | --- | --- | --- | --- | --- |");
    for (split, summary) in &results.splits {
        let _ = writeln!(
            out,
            "| {split} | {} | {} | {} | {} | {} | {} | {} |",
            summary.targets,
            summary.expected_bugs,
            fmt_estimate(summary.bugs_found.as_ref(), 2, ""),
            fmt_estimate(summary.bug_fraction.as_ref(), 2, ""),
            fmt_estimate(summary.branch_coverage.as_ref(), 1, "%"),
            fmt_estimate(summary.line_coverage.as_ref(), 1, "%"),
            fmt_estimate(summary.wall_secs.as_ref(), 3, ""),
        );
    }

    for split in Split::ALL {
        let targets = targets_in(results, split);
        if targets.is_empty() {
            continue;
        }
        let _ = writeln!(out, "\n## Targets: {split}\n");
        let _ = writeln!(
            out,
            "| Target | Bugs found | Seeds finding each bug | Branch coverage | Line coverage | Function coverage | Wall time (s) | Time to first failure (ms) |"
        );
        let _ = writeln!(out, "| --- | --- | --- | --- | --- | --- | --- | --- |");
        for target in targets {
            let found_by = if target.failures_are_bugs {
                "any failure counts".to_string()
            } else {
                target
                    .found_counts()
                    .iter()
                    .map(|(bug, count)| format!("`{bug}` {count}/{}", target.runs.len()))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let _ = writeln!(
                out,
                "| `{}` | {} | {} | {} | {} | {} | {} | {} |",
                target.name,
                fmt_estimate(Estimate::from_samples(&target.bugs_found()).as_ref(), 2, ""),
                found_by,
                fmt_estimate(Estimate::from_samples(&target.branch_pcts()).as_ref(), 1, "%"),
                fmt_estimate(Estimate::from_samples(&target.line_pcts()).as_ref(), 1, "%"),
                fmt_estimate(Estimate::from_samples(&target.function_pcts()).as_ref(), 1, "%"),
                fmt_estimate(Estimate::from_samples(&target.wall_secs()).as_ref(), 3, ""),
                fmt_estimate(
                    Estimate::from_samples(
                        &target
                            .first_failure_secs()
                            .iter()
                            .map(|secs| secs * 1e3)
                            .collect::<Vec<_>>()
                    )
                    .as_ref(),
                    1,
                    ""
                ),
            );
        }
    }

    let _ = writeln!(out, "\n## Diagnostics\n");
    write_variance(&mut out, results);
    write_headroom(&mut out, results);
    write_problems(&mut out, results);
    out
}

fn write_run_info(out: &mut String, results: &EvalResults) {
    if let Some(label) = &results.label {
        let _ = writeln!(out, "- Label: `{label}`");
    }
    let _ = writeln!(out, "- Manifest: `{}`", results.manifest);
    let _ = writeln!(out, "- Forge: `{}` (`{}`)", results.forge_version, results.forge_bin);
    let _ = writeln!(
        out,
        "- Seeds: {}",
        results.seeds.iter().map(u64::to_string).collect::<Vec<_>>().join(", ")
    );
    let _ = writeln!(out, "- Extra forge args: {}", fmt_args(&results.extra_forge_args));
    let _ = writeln!(out, "- Coverage: {}", if results.coverage { "enabled" } else { "disabled" });
}

fn write_variance(out: &mut String, results: &EvalResults) {
    let _ = writeln!(out, "### Variance\n");
    let _ = writeln!(
        out,
        "Flagged when the bugs-found std dev exceeds {HIGH_BUG_STD_DEV} or the branch coverage \
         std dev exceeds {HIGH_BRANCH_STD_DEV} percentage points; flagged targets need more seeds \
         for a tight interval.\n"
    );
    let _ = writeln!(
        out,
        "| Target | Split | Bugs found std dev | Branch coverage std dev (pp) | Wall time CV | Flag |"
    );
    let _ = writeln!(out, "| --- | --- | ---: | ---: | ---: | --- |");
    for target in &results.targets {
        let bugs = Estimate::from_samples(&target.bugs_found());
        let branch = Estimate::from_samples(&target.branch_pcts());
        let wall = Estimate::from_samples(&target.wall_secs());
        let high = bugs.is_some_and(|e| e.std_dev > HIGH_BUG_STD_DEV)
            || branch.is_some_and(|e| e.std_dev > HIGH_BRANCH_STD_DEV);
        let _ = writeln!(
            out,
            "| `{}` | {} | {} | {} | {} | {} |",
            target.name,
            target.split,
            bugs.map_or("n/a".to_string(), |e| format!("{:.2}", e.std_dev)),
            branch.map_or("n/a".to_string(), |e| format!("{:.2}", e.std_dev)),
            wall.filter(|e| e.mean > 0.0)
                .map_or("n/a".to_string(), |e| format!("{:.2}", e.std_dev / e.mean)),
            if high { "high variance" } else { "" },
        );
    }
}

fn write_headroom(out: &mut String, results: &EvalResults) {
    let _ = writeln!(out, "\n### Headroom\n");
    let mut notes = Vec::new();
    for target in &results.targets {
        let runs = target.runs.len();
        if runs == 0 {
            continue;
        }
        if let Some(expected) = target.expected_bugs().filter(|&n| n > 0) {
            if target.runs.iter().all(|run| run.bugs_found.len() == expected) {
                notes.push(format!(
                    "- `{}` ({}): no headroom, every seed found every expected bug",
                    target.name, target.split
                ));
            } else if target.runs.iter().all(|run| run.bugs_found.is_empty()) {
                notes.push(format!(
                    "- `{}` ({}): no signal, no seed found an expected bug",
                    target.name, target.split
                ));
            }
        }
        let branch = target.branch_pcts();
        if branch.len() == runs && branch.iter().all(|&pct| pct >= 100.0) {
            notes.push(format!(
                "- `{}` ({}): no coverage headroom, 100% branch coverage on every seed",
                target.name, target.split
            ));
        }
    }
    if notes.is_empty() {
        let _ = writeln!(out, "Every target has headroom on bugs found and branch coverage.");
    } else {
        let _ = writeln!(out, "{}", notes.join("\n"));
    }
}

fn write_problems(out: &mut String, results: &EvalResults) {
    let _ = writeln!(out, "\n### Harness and target problems\n");
    let mut notes = Vec::new();
    for target in &results.targets {
        for run in &target.runs {
            let seed = run.seed;
            if run.status != RunStatus::Ok {
                notes.push(format!(
                    "- `{}` seed {seed}: forge test run {}: {}",
                    target.name,
                    run.status.as_str(),
                    fmt_error(run.error.as_deref())
                ));
            }
            if !run.unexpected_failures.is_empty() {
                notes.push(format!(
                    "- `{}` seed {seed}: unexpected failing tests: {}",
                    target.name,
                    fmt_names(&run.unexpected_failures)
                ));
            }
            if !run.missing_expected.is_empty() {
                notes.push(format!(
                    "- `{}` seed {seed}: expected failures matched no test: {}",
                    target.name,
                    fmt_names(&run.missing_expected)
                ));
            }
            if let Some(coverage) = &run.coverage
                && coverage.status != RunStatus::Ok
            {
                notes.push(format!(
                    "- `{}` seed {seed}: forge coverage run {}: {}",
                    target.name,
                    coverage.status.as_str(),
                    fmt_error(coverage.error.as_deref())
                ));
            }
        }
    }
    if notes.is_empty() {
        let _ = writeln!(out, "None: every run produced results and only expected tests failed.");
    } else {
        let _ = writeln!(out, "{}", notes.join("\n"));
    }
}

/// Decision for a candidate change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The held-out test split improved.
    Keep,
    /// Only the train split improved, or the test split regressed.
    Revert,
    /// No significant change on the test split.
    Neutral,
}

impl fmt::Display for Verdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Keep => "KEEP",
            Self::Revert => "REVERT",
            Self::Neutral => "NEUTRAL",
        })
    }
}

/// Primary and secondary metric changes for one split.
#[derive(Clone, Copy, Debug, Default)]
pub struct SplitChange {
    pub bugs_found: Option<Difference>,
    pub branch_coverage: Option<Difference>,
}

impl SplitChange {
    /// Bugs found increased, or branch coverage increased without bugs found decreasing.
    pub fn improved(&self) -> bool {
        let bugs_up = self.bugs_found.is_some_and(|d| d.is_increase());
        let bugs_down = self.bugs_found.is_some_and(|d| d.is_decrease());
        bugs_up || (!bugs_down && self.branch_coverage.is_some_and(|d| d.is_increase()))
    }

    /// Bugs found decreased, or branch coverage decreased without bugs found increasing.
    pub fn regressed(&self) -> bool {
        let bugs_up = self.bugs_found.is_some_and(|d| d.is_increase());
        let bugs_down = self.bugs_found.is_some_and(|d| d.is_decrease());
        bugs_down || (!bugs_up && self.branch_coverage.is_some_and(|d| d.is_decrease()))
    }
}

/// Applies the keep/revert rule to the train and test split changes.
///
/// A metric changes only when the 95% interval of its difference excludes zero. The test split
/// decides: `KEEP` if it improved, `REVERT` if it regressed or if only the train split improved
/// (suspected overfitting), and `NEUTRAL` otherwise.
pub fn verdict(train: Option<&SplitChange>, test: Option<&SplitChange>) -> (Verdict, String) {
    let train_improved = train.is_some_and(SplitChange::improved);
    let Some(test) = test else {
        return (Verdict::Neutral, "no test split in both runs".to_string());
    };
    if test.improved() {
        let train_note = if train_improved { "; train also improved" } else { "" };
        (Verdict::Keep, format!("held-out test split improved{train_note}"))
    } else if test.regressed() {
        (Verdict::Revert, "held-out test split regressed".to_string())
    } else if train_improved {
        (Verdict::Revert, "only the train split improved (suspected overfitting)".to_string())
    } else {
        (Verdict::Neutral, "no significant change on the held-out test split".to_string())
    }
}

/// Renders the comparison of `current` against `baseline` and returns it with the verdict.
pub fn render_comparison(baseline: &EvalResults, current: &EvalResults) -> (String, Verdict) {
    let mut out = String::new();
    let _ = writeln!(out, "# Fuzz eval comparison\n");
    for (name, results) in [("Baseline", baseline), ("Current", current)] {
        let _ = writeln!(
            out,
            "- {name}: {}forge `{}`, extra args {}, seeds {}",
            results.label.as_ref().map(|label| format!("`{label}`, ")).unwrap_or_default(),
            results.forge_version,
            fmt_args(&results.extra_forge_args),
            results.seeds.iter().map(u64::to_string).collect::<Vec<_>>().join(", "),
        );
    }
    let warnings = comparison_warnings(baseline, current);
    for warning in &warnings {
        let _ = writeln!(out, "- Warning: {warning}");
    }
    let _ = writeln!(
        out,
        "\nDeltas are `current - baseline` with 95% t-intervals: paired over seeds when both runs \
         used the same seeds, Welch otherwise. A metric changed only if its delta interval \
         excludes zero.\n"
    );
    let _ = writeln!(
        out,
        "| Split | Metric | Baseline | Current | Delta [95% CI] | CIs overlap | Change |"
    );
    let _ = writeln!(out, "| --- | --- | --- | --- | --- | --- | --- |");

    let mut changes = [None, None];
    for (index, split) in Split::ALL.into_iter().enumerate() {
        let (Some(base), Some(cur)) = (baseline.splits.get(&split), current.splits.get(&split))
        else {
            continue;
        };
        let mut change = SplitChange::default();
        for (metric, name, decimals, unit, higher_is_better) in [
            (Metric::BugsFound, "bugs found", 2, "", true),
            (Metric::BugFraction, "fraction of expected", 2, "", true),
            (Metric::BranchCoverage, "branch coverage", 1, "%", true),
            (Metric::LineCoverage, "line coverage", 1, "%", true),
            (Metric::WallSecs, "wall time (s)", 3, "", false),
        ] {
            let Some(diff) = difference(base, cur, metric) else { continue };
            match metric {
                Metric::BugsFound => change.bugs_found = Some(diff),
                Metric::BranchCoverage => change.branch_coverage = Some(diff),
                _ => {}
            }
            let base_estimate = estimate(base, metric);
            let cur_estimate = estimate(cur, metric);
            let overlap =
                match (&base_estimate, &cur_estimate) {
                    (Some(a), Some(b)) if a.ci_low.is_some() && b.ci_low.is_some() => {
                        if a.overlaps(b) { "yes" } else { "no" }
                    }
                    _ => "n/a",
                };
            let label = match (diff.is_increase(), diff.is_decrease(), higher_is_better) {
                (true, _, true) | (_, true, false) => "better",
                (true, _, false) | (_, true, true) => "worse",
                _ => "no significant change",
            };
            let _ = writeln!(
                out,
                "| {split} | {name} | {} | {} | {} | {overlap} | {label} |",
                fmt_estimate(base_estimate.as_ref(), decimals, unit),
                fmt_estimate(cur_estimate.as_ref(), decimals, unit),
                fmt_difference(&diff, decimals),
            );
        }
        changes[index] = Some(change);
    }

    write_target_deltas(&mut out, baseline, current);

    let (verdict, reason) = verdict(changes[0].as_ref(), changes[1].as_ref());
    let _ = writeln!(out, "\nVerdict: {verdict} ({reason})");
    (out, verdict)
}

fn write_target_deltas(out: &mut String, baseline: &EvalResults, current: &EvalResults) {
    let _ = writeln!(out, "\n## Per-target bugs found\n");
    let _ = writeln!(out, "| Target | Split | Baseline | Current | Delta |");
    let _ = writeln!(out, "| --- | --- | ---: | ---: | ---: |");
    for target in &current.targets {
        let Some(base) = baseline.targets.iter().find(|base| base.name == target.name) else {
            continue;
        };
        let base_mean = Estimate::from_samples(&base.bugs_found()).map_or(0.0, |e| e.mean);
        let cur_mean = Estimate::from_samples(&target.bugs_found()).map_or(0.0, |e| e.mean);
        let _ = writeln!(
            out,
            "| `{}` | {} | {base_mean:.2} | {cur_mean:.2} | {:+.2} |",
            target.name,
            target.split,
            cur_mean - base_mean
        );
    }
}

fn comparison_warnings(baseline: &EvalResults, current: &EvalResults) -> Vec<String> {
    let mut warnings = Vec::new();
    let names = |results: &EvalResults| {
        results
            .targets
            .iter()
            .map(|target| (target.split, target.name.clone()))
            .collect::<BTreeSet<_>>()
    };
    if names(baseline) != names(current) {
        warnings.push("the runs cover different targets or splits".to_string());
    }
    if baseline.seeds != current.seeds {
        warnings.push("the runs used different seeds; intervals are unpaired".to_string());
    }
    for target in &current.targets {
        if let Some(base) = baseline.targets.iter().find(|base| base.name == target.name)
            && base.budget != target.budget
        {
            warnings.push(format!("`{}` used a different budget", target.name));
        }
    }
    if baseline.coverage != current.coverage {
        warnings.push("only one run collected coverage".to_string());
    }
    warnings
}

fn difference(base: &SplitSummary, cur: &SplitSummary, metric: Metric) -> Option<Difference> {
    let base_samples = base.samples(metric);
    let cur_samples = cur.samples(metric);
    let paired = base_samples.len() == cur_samples.len()
        && base_samples.iter().zip(&cur_samples).all(|(a, b)| a.0 == b.0);
    let values =
        |samples: &[(u64, f64)]| samples.iter().map(|(_, value)| *value).collect::<Vec<_>>();
    Difference::between(&values(&base_samples), &values(&cur_samples), paired)
}

fn estimate(summary: &SplitSummary, metric: Metric) -> Option<Estimate> {
    Estimate::from_samples(&summary.samples(metric).into_iter().map(|(_, v)| v).collect::<Vec<_>>())
}

fn targets_in(results: &EvalResults, split: Split) -> Vec<&TargetResult> {
    results.targets.iter().filter(|target| target.split == split).collect()
}

fn fmt_estimate(estimate: Option<&Estimate>, decimals: usize, unit: &str) -> String {
    let Some(estimate) = estimate else { return "n/a".to_string() };
    match (estimate.ci_low, estimate.ci_high) {
        (Some(low), Some(high)) => {
            format!("{:.decimals$}{unit} [{low:.decimals$}, {high:.decimals$}]", estimate.mean)
        }
        _ => format!("{:.decimals$}{unit} (n={})", estimate.mean, estimate.n),
    }
}

fn fmt_difference(diff: &Difference, decimals: usize) -> String {
    match (diff.ci_low, diff.ci_high) {
        (Some(low), Some(high)) => {
            format!("{:+.decimals$} [{low:+.decimals$}, {high:+.decimals$}]", diff.delta)
        }
        _ => format!("{:+.decimals$} (no interval)", diff.delta),
    }
}

fn fmt_args(args: &[String]) -> String {
    if args.is_empty() { "none".to_string() } else { format!("`{}`", args.join(" ")) }
}

fn fmt_names(names: &[String]) -> String {
    names.iter().map(|name| format!("`{name}`")).collect::<Vec<_>>().join(", ")
}

fn fmt_error(error: Option<&str>) -> String {
    let error = error.unwrap_or("unknown error").replace('\n', " ").replace('|', "\\|");
    if error.chars().count() > MAX_ERROR_LEN {
        format!("{}...", error.chars().take(MAX_ERROR_LEN).collect::<String>())
    } else {
        error
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        lcov::CoverageStats,
        manifest::{Budget, Source},
        results::{CoverageRun, SeedRun},
    };
    use snapbox::{assert_data_eq, str};
    use std::{collections::BTreeMap, path::PathBuf};

    fn diff(low: f64, high: f64) -> Option<Difference> {
        Some(Difference {
            delta: (low + high) / 2.0,
            ci_low: Some(low),
            ci_high: Some(high),
            paired: true,
        })
    }

    fn change(bugs: Option<Difference>, branch: Option<Difference>) -> SplitChange {
        SplitChange { bugs_found: bugs, branch_coverage: branch }
    }

    #[test]
    fn verdict_rule() {
        let up = diff(0.5, 1.5);
        let down = diff(-1.5, -0.5);
        let flat = diff(-0.5, 0.5);
        let cases = [
            // (train, test, verdict)
            (change(flat, flat), change(up, flat), Verdict::Keep),
            (change(up, up), change(up, flat), Verdict::Keep),
            (change(flat, flat), change(flat, up), Verdict::Keep),
            (change(flat, flat), change(down, up), Verdict::Revert),
            (change(up, flat), change(flat, flat), Verdict::Revert),
            (change(flat, up), change(flat, flat), Verdict::Revert),
            (change(flat, flat), change(flat, down), Verdict::Revert),
            (change(flat, flat), change(flat, flat), Verdict::Neutral),
            (change(down, down), change(flat, flat), Verdict::Neutral),
            (change(None, None), change(None, None), Verdict::Neutral),
        ];
        for (train, test, expected) in cases {
            assert_eq!(verdict(Some(&train), Some(&test)).0, expected, "{train:?} / {test:?}");
        }
        assert_eq!(
            verdict(Some(&change(up, up)), None),
            (Verdict::Neutral, "no test split in both runs".to_string())
        );
        assert_eq!(
            verdict(Some(&change(up, flat)), Some(&change(flat, flat))).1,
            "only the train split improved (suspected overfitting)"
        );
    }

    fn run(seed: u64, bugs: &[&str], branches_hit: Option<u64>, wall_secs: f64) -> SeedRun {
        SeedRun {
            seed,
            status: RunStatus::Ok,
            error: None,
            exit_code: Some(1),
            wall_secs,
            tests: Vec::new(),
            bugs_found: bugs.iter().map(|bug| bug.to_string()).collect(),
            unexpected_failures: Vec::new(),
            missing_expected: Vec::new(),
            first_failure_secs: (!bugs.is_empty()).then_some(wall_secs / 2.0),
            coverage: branches_hit.map(|hit| CoverageRun {
                status: RunStatus::Ok,
                error: None,
                wall_secs,
                stats: Some(CoverageStats {
                    files: 1,
                    lines_found: 10,
                    lines_hit: 5 + hit,
                    branches_found: 4,
                    branches_hit: hit,
                    functions_found: 2,
                    functions_hit: 2,
                }),
            }),
        }
    }

    fn target(name: &str, split: Split, runs: Vec<SeedRun>) -> TargetResult {
        TargetResult {
            name: name.to_string(),
            split,
            source: Source::Local { path: PathBuf::from("/fixtures/project") },
            expected_failures: vec!["invariant_bug".to_string()],
            failures_are_bugs: false,
            budget: Budget::default(),
            runs,
        }
    }

    fn results(label: &str, targets: Vec<TargetResult>) -> EvalResults {
        let mut results = EvalResults {
            schema_version: crate::results::SCHEMA_VERSION,
            label: Some(label.to_string()),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            manifest: "manifest.toml".to_string(),
            forge_bin: "/bin/forge".to_string(),
            forge_version: "forge 1.0.0".to_string(),
            extra_forge_args: Vec::new(),
            seeds: vec![1, 2, 3],
            coverage: true,
            targets,
            splits: BTreeMap::new(),
        };
        results.summarize();
        results
    }

    fn baseline() -> EvalResults {
        let bug = ["invariant_bug"];
        let mut flaky = target(
            "flaky",
            Split::Train,
            vec![run(1, &bug, Some(4), 1.0), run(2, &[], Some(3), 2.0), run(3, &bug, Some(4), 1.0)],
        );
        flaky.runs[1].unexpected_failures = vec!["FlakyTest::test_setup".to_string()];
        let saturated = target(
            "saturated",
            Split::Train,
            vec![
                run(1, &bug, Some(4), 0.5),
                run(2, &bug, Some(4), 0.5),
                run(3, &bug, Some(4), 0.5),
            ],
        );
        let mut hard = target(
            "hard",
            Split::Test,
            vec![run(1, &[], Some(2), 3.0), run(2, &[], Some(2), 3.0), run(3, &[], Some(1), 3.0)],
        );
        hard.runs[2].status = RunStatus::TimedOut;
        hard.runs[2].error = Some("forge test timed out after 3s".to_string());
        results("baseline", vec![flaky, saturated, hard])
    }

    #[test]
    fn renders_summary() {
        assert_data_eq!(
            render_summary(&baseline()),
            str![[r#"
# Fuzz eval summary

- Label: `baseline`
- Manifest: `manifest.toml`
- Forge: `forge 1.0.0` (`/bin/forge`)
- Seeds: 1, 2, 3
- Extra forge args: none
- Coverage: enabled
- Intervals: two-sided 95% Student t-intervals across seeds, shown as `mean [low, high]`. Split coverage is the per-seed mean over the split's targets.

## Splits

| Split | Targets | Expected bugs | Bugs found | Fraction of expected | Branch coverage | Line coverage | Wall time (s) |
| --- | ---: | ---: | --- | --- | --- | --- | --- |
| train | 2 | 2 | 1.67 [0.23, 3.10] | 0.83 [0.12, 1.55] | 95.8% [77.9, 113.8] | 88.3% [81.2, 95.5] | 1.833 [0.399, 3.268] |
| test | 1 | 1 | 0.00 [0.00, 0.00] | 0.00 [0.00, 0.00] | 41.7% [5.8, 77.5] | 66.7% [52.3, 81.0] | 3.000 [3.000, 3.000] |

## Targets: train

| Target | Bugs found | Seeds finding each bug | Branch coverage | Line coverage | Function coverage | Wall time (s) | Time to first failure (ms) |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `flaky` | 0.67 [-0.77, 2.10] | `invariant_bug` 2/3 | 91.7% [55.8, 127.5] | 86.7% [72.3, 101.0] | 100.0% [100.0, 100.0] | 1.333 [-0.101, 2.768] | 500.0 [500.0, 500.0] |
| `saturated` | 1.00 [1.00, 1.00] | `invariant_bug` 3/3 | 100.0% [100.0, 100.0] | 90.0% [90.0, 90.0] | 100.0% [100.0, 100.0] | 0.500 [0.500, 0.500] | 250.0 [250.0, 250.0] |

## Targets: test

| Target | Bugs found | Seeds finding each bug | Branch coverage | Line coverage | Function coverage | Wall time (s) | Time to first failure (ms) |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `hard` | 0.00 [0.00, 0.00] | `invariant_bug` 0/3 | 41.7% [5.8, 77.5] | 66.7% [52.3, 81.0] | 100.0% [100.0, 100.0] | 3.000 [3.000, 3.000] | n/a |

## Diagnostics

### Variance

Flagged when the bugs-found std dev exceeds 0.4 or the branch coverage std dev exceeds 5 percentage points; flagged targets need more seeds for a tight interval.

| Target | Split | Bugs found std dev | Branch coverage std dev (pp) | Wall time CV | Flag |
| --- | --- | ---: | ---: | ---: | --- |
| `flaky` | train | 0.58 | 14.43 | 0.43 | high variance |
| `saturated` | train | 0.00 | 0.00 | 0.00 |  |
| `hard` | test | 0.00 | 14.43 | 0.00 | high variance |

### Headroom

- `saturated` (train): no headroom, every seed found every expected bug
- `saturated` (train): no coverage headroom, 100% branch coverage on every seed
- `hard` (test): no signal, no seed found an expected bug

### Harness and target problems

- `flaky` seed 2: unexpected failing tests: `FlakyTest::test_setup`
- `hard` seed 3: forge test run timed out: forge test timed out after 3s

"#]]
        );
    }

    #[test]
    fn renders_comparison() {
        let mut current = baseline();
        current.label = Some("candidate".to_string());
        current.extra_forge_args = vec!["--fuzz-guidance".to_string(), "g.json".to_string()];
        let hard = current.targets.iter_mut().find(|t| t.name == "hard").unwrap();
        for run in &mut hard.runs {
            run.bugs_found = vec!["invariant_bug".to_string()];
            run.coverage.as_mut().unwrap().stats.as_mut().unwrap().branches_hit = 3;
        }
        current.summarize();
        let (markdown, verdict) = render_comparison(&baseline(), &current);
        assert_eq!(verdict, Verdict::Keep);
        assert_data_eq!(
            markdown,
            str![[r#"
# Fuzz eval comparison

- Baseline: `baseline`, forge `forge 1.0.0`, extra args none, seeds 1, 2, 3
- Current: `candidate`, forge `forge 1.0.0`, extra args `--fuzz-guidance g.json`, seeds 1, 2, 3

Deltas are `current - baseline` with 95% t-intervals: paired over seeds when both runs used the same seeds, Welch otherwise. A metric changed only if its delta interval excludes zero.

| Split | Metric | Baseline | Current | Delta [95% CI] | CIs overlap | Change |
| --- | --- | --- | --- | --- | --- | --- |
| train | bugs found | 1.67 [0.23, 3.10] | 1.67 [0.23, 3.10] | +0.00 [+0.00, +0.00] | yes | no significant change |
| train | fraction of expected | 0.83 [0.12, 1.55] | 0.83 [0.12, 1.55] | +0.00 [+0.00, +0.00] | yes | no significant change |
| train | branch coverage | 95.8% [77.9, 113.8] | 95.8% [77.9, 113.8] | +0.0 [+0.0, +0.0] | yes | no significant change |
| train | line coverage | 88.3% [81.2, 95.5] | 88.3% [81.2, 95.5] | +0.0 [+0.0, +0.0] | yes | no significant change |
| train | wall time (s) | 1.833 [0.399, 3.268] | 1.833 [0.399, 3.268] | +0.000 [+0.000, +0.000] | yes | no significant change |
| test | bugs found | 0.00 [0.00, 0.00] | 1.00 [1.00, 1.00] | +1.00 [+1.00, +1.00] | no | better |
| test | fraction of expected | 0.00 [0.00, 0.00] | 1.00 [1.00, 1.00] | +1.00 [+1.00, +1.00] | no | better |
| test | branch coverage | 41.7% [5.8, 77.5] | 75.0% [75.0, 75.0] | +33.3 [-2.5, +69.2] | yes | no significant change |
| test | line coverage | 66.7% [52.3, 81.0] | 66.7% [52.3, 81.0] | +0.0 [+0.0, +0.0] | yes | no significant change |
| test | wall time (s) | 3.000 [3.000, 3.000] | 3.000 [3.000, 3.000] | +0.000 [+0.000, +0.000] | yes | no significant change |

## Per-target bugs found

| Target | Split | Baseline | Current | Delta |
| --- | --- | ---: | ---: | ---: |
| `flaky` | train | 0.67 | 0.67 | +0.00 |
| `saturated` | train | 1.00 | 1.00 | +0.00 |
| `hard` | test | 0.00 | 1.00 | +1.00 |

Verdict: KEEP (held-out test split improved)

"#]]
        );
    }
}
