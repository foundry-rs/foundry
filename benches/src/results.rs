use crate::RepoConfig;
use eyre::Result;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, process::Command, thread};

/// Hyperfine benchmark result for a single command.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HyperfineResult {
    pub command: String,
    pub mean: f64,
    pub stddev: Option<f64>,
    pub median: f64,
    pub user: f64,
    pub system: f64,
    pub min: f64,
    pub max: f64,
    pub times: Vec<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_codes: Option<Vec<i32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<HashMap<String, serde_json::Value>>,
}

/// Hyperfine JSON output format.
#[derive(Debug, Deserialize, Serialize)]
pub struct HyperfineOutput {
    pub results: Vec<HyperfineResult>,
}

/// Comparison between baseline and feature for a single benchmark + repo.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchComparison {
    pub benchmark: String,
    pub repo: String,
    pub baseline_mean: f64,
    pub feature_mean: f64,
    pub delta_pct: f64,
    pub baseline_stddev: Option<f64>,
    pub feature_stddev: Option<f64>,
    pub verdict: Verdict,
}

/// Overall verdict for a benchmark comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Improved,
    Regressed,
    Neutral,
}

impl Verdict {
    /// Determine verdict from percentage delta and noise threshold.
    pub fn from_delta(delta_pct: f64, noise_threshold: f64) -> Self {
        if delta_pct < -noise_threshold {
            Self::Improved
        } else if delta_pct > noise_threshold {
            Self::Regressed
        } else {
            Self::Neutral
        }
    }

    pub const fn emoji(self) -> &'static str {
        match self {
            Self::Improved => "🟢",
            Self::Regressed => "🔴",
            Self::Neutral => "⚪",
        }
    }
}

impl std::fmt::Display for Verdict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Improved => write!(f, "improved"),
            Self::Regressed => write!(f, "regressed"),
            Self::Neutral => write!(f, "neutral"),
        }
    }
}

/// Structured bundle output for machine consumption.
#[derive(Debug, Serialize, Deserialize)]
pub struct BenchBundle {
    pub baseline_ref: String,
    pub feature_ref: String,
    pub baseline_version: String,
    pub feature_version: String,
    pub noise_threshold: f64,
    pub timestamp: String,
    pub system: SystemInfo,
    pub comparisons: Vec<BenchComparison>,
    pub overall_verdict: Verdict,
}

/// System information captured during the benchmark run.
#[derive(Debug, Serialize, Deserialize)]
pub struct SystemInfo {
    pub os: String,
    pub cpu_cores: usize,
    pub rustc: String,
}

impl SystemInfo {
    pub fn capture() -> Self {
        Self {
            os: std::env::consts::OS.to_string(),
            cpu_cores: thread::available_parallelism().map_or(1, |n| n.get()),
            rustc: get_rustc_version().unwrap_or_else(|_| "unknown".to_string()),
        }
    }
}

/// Aggregated benchmark results for baseline vs feature comparison.
#[derive(Debug, Default)]
pub struct BenchmarkResults {
    /// benchmark_name -> "baseline"|"feature" -> repo_name -> result
    pub data: HashMap<String, HashMap<String, HashMap<String, HyperfineResult>>>,
}

impl BenchmarkResults {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_result(&mut self, benchmark: &str, side: &str, repo: &str, result: HyperfineResult) {
        self.data
            .entry(benchmark.to_string())
            .or_default()
            .entry(side.to_string())
            .or_default()
            .insert(repo.to_string(), result);
    }

    /// Build comparisons between baseline and feature results.
    pub fn compare(&self, noise_threshold: f64) -> Vec<BenchComparison> {
        let mut comparisons = Vec::new();

        for (bench_name, sides) in &self.data {
            let Some(baseline_repos) = sides.get("baseline") else {
                continue;
            };
            let Some(feature_repos) = sides.get("feature") else {
                continue;
            };

            for (repo_name, baseline_result) in baseline_repos {
                if let Some(feature_result) = feature_repos.get(repo_name) {
                    let delta_pct = if baseline_result.mean > 0.0 {
                        ((feature_result.mean - baseline_result.mean) / baseline_result.mean)
                            * 100.0
                    } else {
                        0.0
                    };

                    comparisons.push(BenchComparison {
                        benchmark: bench_name.clone(),
                        repo: repo_name.clone(),
                        baseline_mean: baseline_result.mean,
                        feature_mean: feature_result.mean,
                        delta_pct,
                        baseline_stddev: baseline_result.stddev,
                        feature_stddev: feature_result.stddev,
                        verdict: Verdict::from_delta(delta_pct, noise_threshold),
                    });
                }
            }
        }

        comparisons.sort_by(|a, b| a.benchmark.cmp(&b.benchmark).then(a.repo.cmp(&b.repo)));
        comparisons
    }

    /// Compute overall verdict from individual comparisons.
    pub fn overall_verdict(comparisons: &[BenchComparison]) -> Verdict {
        let has_regression = comparisons.iter().any(|c| c.verdict == Verdict::Regressed);
        let has_improvement = comparisons.iter().any(|c| c.verdict == Verdict::Improved);

        if has_regression {
            Verdict::Regressed
        } else if has_improvement {
            Verdict::Improved
        } else {
            Verdict::Neutral
        }
    }

    /// Generate a structured JSON bundle.
    pub fn to_bundle(
        &self,
        baseline_ref: &str,
        feature_ref: &str,
        baseline_version: &str,
        feature_version: &str,
        noise_threshold: f64,
    ) -> BenchBundle {
        let comparisons = self.compare(noise_threshold);
        let overall_verdict = Self::overall_verdict(&comparisons);

        BenchBundle {
            baseline_ref: baseline_ref.to_string(),
            feature_ref: feature_ref.to_string(),
            baseline_version: baseline_version.to_string(),
            feature_version: feature_version.to_string(),
            noise_threshold,
            timestamp: chrono::Utc::now().to_rfc3339(),
            system: SystemInfo::capture(),
            comparisons,
            overall_verdict,
        }
    }

    /// Generate markdown comparison report.
    pub fn generate_markdown(
        &self,
        baseline_ref: &str,
        feature_ref: &str,
        baseline_version: &str,
        feature_version: &str,
        repos: &[RepoConfig],
        noise_threshold: f64,
    ) -> String {
        let comparisons = self.compare(noise_threshold);
        let overall = Self::overall_verdict(&comparisons);
        let mut out = String::new();

        out.push_str("# Foundry Benchmark Results\n\n");
        out.push_str(&format!(
            "**Date**: {}\n\n",
            chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC")
        ));
        out.push_str(&format!("**Baseline**: `{baseline_ref}` ({baseline_version})\n"));
        out.push_str(&format!("**Feature**: `{feature_ref}` ({feature_version})\n"));
        out.push_str(&format!("**Overall**: {} {}\n\n", overall.emoji(), overall));

        // Group comparisons by benchmark.
        let mut by_bench: HashMap<&str, Vec<&BenchComparison>> = HashMap::new();
        for comp in &comparisons {
            by_bench.entry(&comp.benchmark).or_default().push(comp);
        }

        let mut bench_names: Vec<&&str> = by_bench.keys().collect();
        bench_names.sort();

        for bench_name in bench_names {
            let comps = &by_bench[bench_name];
            out.push_str(&format!("## {}\n\n", format_benchmark_name(bench_name)));
            out.push_str("| Repository | Baseline | Feature | Delta | Verdict |\n");
            out.push_str("|------------|----------|---------|-------|---------|\n");

            for comp in comps {
                let repo_display = repos
                    .iter()
                    .find(|r| r.name == comp.repo)
                    .map(|r| r.name.as_str())
                    .unwrap_or(&comp.repo);

                out.push_str(&format!(
                    "| {} | {} | {} | {:+.1}% | {} {} |\n",
                    repo_display,
                    format_duration_seconds(comp.baseline_mean),
                    format_duration_seconds(comp.feature_mean),
                    comp.delta_pct,
                    comp.verdict.emoji(),
                    comp.verdict,
                ));
            }
            out.push('\n');
        }

        // System info.
        out.push_str("## System Information\n\n");
        out.push_str(&format!("- **OS**: {}\n", std::env::consts::OS));
        out.push_str(&format!(
            "- **CPU**: {} cores\n",
            thread::available_parallelism().map_or(1, |n| n.get())
        ));
        out.push_str(&format!(
            "- **Rustc**: {}\n",
            get_rustc_version().unwrap_or_else(|_| "unknown".to_string())
        ));

        out
    }
}

pub fn format_benchmark_name(name: &str) -> String {
    match name {
        "forge_test" => "Forge Test",
        "forge_build_no_cache" => "Forge Build (No Cache)",
        "forge_build_with_cache" => "Forge Build (With Cache)",
        "forge_fuzz_test" => "Forge Fuzz Test",
        "forge_coverage" => "Forge Coverage",
        "forge_isolate_test" => "Forge Test (Isolated)",
        "forge_invariant_test" => "Forge Invariant Test",
        "forge_fork_test" => "Forge Fork Test",
        _ => name,
    }
    .to_string()
}

pub fn format_duration_seconds(seconds: f64) -> String {
    if seconds < 0.001 {
        format!("{:.2}ms", seconds * 1000.0)
    } else if seconds < 1.0 {
        format!("{seconds:.3}s")
    } else if seconds < 60.0 {
        format!("{seconds:.2}s")
    } else {
        let minutes = (seconds / 60.0).floor();
        let remaining_seconds = seconds % 60.0;
        format!("{minutes:.0}m {remaining_seconds:.1}s")
    }
}

pub fn get_rustc_version() -> Result<String> {
    let output = Command::new("rustc").arg("--version").output()?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
