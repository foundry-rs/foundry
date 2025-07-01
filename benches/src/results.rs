use crate::{criterion_types::CriterionResult, BENCHMARK_REPOS};
use color_eyre::eyre::Result;
use std::{collections::HashMap, process::Command};

/// Aggregated benchmark results
#[derive(Debug)]
pub struct BenchmarkResults {
    /// Map of benchmark_name -> version -> repo -> result
    pub data: HashMap<String, HashMap<String, HashMap<String, CriterionResult>>>,
    /// Track the baseline version for comparison
    pub baseline_version: Option<String>,
}

impl BenchmarkResults {
    pub fn new() -> Self {
        Self { data: HashMap::new(), baseline_version: None }
    }

    pub fn set_baseline_version(&mut self, version: String) {
        self.baseline_version = Some(version);
    }

    pub fn add_result(
        &mut self,
        benchmark: &str,
        version: &str,
        repo: &str,
        result: CriterionResult,
    ) {
        self.data
            .entry(benchmark.to_string())
            .or_insert_with(HashMap::new)
            .entry(version.to_string())
            .or_insert_with(HashMap::new)
            .insert(repo.to_string(), result);
    }

    pub fn generate_markdown(&self, versions: &[String]) -> String {
        let mut output = String::new();

        // Header
        output.push_str("# Foundry Benchmark Results\n\n");
        output.push_str(&format!(
            "**Date**: {}\n\n",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
        ));

        // Summary
        output.push_str("## Summary\n\n");
        // Count actual repos that have results
        let mut repos_with_results = std::collections::HashSet::new();
        for (_, version_data) in &self.data {
            for (_, repo_data) in version_data {
                for repo_name in repo_data.keys() {
                    repos_with_results.insert(repo_name.clone());
                }
            }
        }
        output.push_str(&format!(
            "Benchmarked {} Foundry versions across {} repositories.\n\n",
            versions.len(),
            repos_with_results.len()
        ));

        // Repositories tested
        output.push_str("### Repositories Tested\n\n");
        for (i, repo) in BENCHMARK_REPOS.iter().enumerate() {
            output.push_str(&format!(
                "{}. [{}/{}](https://github.com/{}/{})\n",
                i + 1,
                repo.org,
                repo.repo,
                repo.org,
                repo.repo
            ));
        }
        output.push('\n');

        // Versions tested
        output.push_str("### Foundry Versions\n\n");
        for version in versions {
            output.push_str(&format!("- {}\n", version));
        }
        output.push('\n');

        // Results for each benchmark type
        for (benchmark_name, version_data) in &self.data {
            output.push_str(&format!("## {}\n\n", format_benchmark_name(benchmark_name)));

            // Create table header
            output.push_str("| Repository |");
            for version in versions {
                output.push_str(&format!(" {} |", version));
            }
            output.push('\n');

            // Table separator
            output.push_str("|------------|");
            for _ in versions {
                output.push_str("----------|");
            }
            output.push('\n');

            // Table rows
            for repo in BENCHMARK_REPOS {
                output.push_str(&format!("| {} |", repo.name));

                let mut values = Vec::new();
                for version in versions {
                    if let Some(repo_data) = version_data.get(version) {
                        if let Some(result) = repo_data.get(repo.name) {
                            if let Some(mean) = &result.mean {
                                let value = format_duration(
                                    mean.point_estimate,
                                    result.unit.as_deref().unwrap_or("ns"),
                                );
                                output.push_str(&format!(" {} |", value));
                                values.push(Some(mean.point_estimate));
                            } else {
                                output.push_str(" N/A |");
                                values.push(None);
                            }
                        } else {
                            output.push_str(" N/A |");
                            values.push(None);
                        }
                    } else {
                        output.push_str(" N/A |");
                        values.push(None);
                    }
                }

                output.push('\n');
            }
            output.push('\n');
        }

        // System info
        output.push_str("## System Information\n\n");
        output.push_str(&format!("- **OS**: {}\n", std::env::consts::OS));
        output.push_str(&format!("- **CPU**: {}\n", num_cpus::get()));
        output.push_str(&format!(
            "- **Rustc**: {}\n",
            get_rustc_version().unwrap_or_else(|_| "unknown".to_string())
        ));

        output
    }
}

pub fn format_benchmark_name(name: &str) -> String {
    match name {
        "forge-test" => "Forge Test Performance",
        "forge-build-no-cache" => "Forge Build Performance (No Cache)",
        "forge-build-with-cache" => "Forge Build Performance (With Cache)",
        _ => name,
    }
    .to_string()
}

pub fn format_duration(nanos: f64, unit: &str) -> String {
    match unit {
        "ns" => {
            if nanos < 1_000.0 {
                format!("{:.2} ns", nanos)
            } else if nanos < 1_000_000.0 {
                format!("{:.2} µs", nanos / 1_000.0)
            } else if nanos < 1_000_000_000.0 {
                format!("{:.2} ms", nanos / 1_000_000.0)
            } else {
                format!("{:.2} s", nanos / 1_000_000_000.0)
            }
        }
        _ => format!("{:.2} {}", nanos, unit),
    }
}

pub fn get_rustc_version() -> Result<String> {
    let output = Command::new("rustc").arg("--version").output()?;

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
