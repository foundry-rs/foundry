use crate::{criterion_types::CriterionResult, RepoConfig};
use eyre::Result;
use std::{collections::HashMap, process::Command};

/// Aggregated benchmark results
#[derive(Debug, Default)]
pub struct BenchmarkResults {
    /// Map of benchmark_name -> version -> repo -> result
    pub data: HashMap<String, HashMap<String, HashMap<String, CriterionResult>>>,
    /// Track the baseline version for comparison
    pub baseline_version: Option<String>,
}

impl BenchmarkResults {
    pub fn new() -> Self {
        Self::default()
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
            .or_default()
            .entry(version.to_string())
            .or_default()
            .insert(repo.to_string(), result);
    }

    pub fn generate_markdown(&self, versions: &[String], repos: &[RepoConfig]) -> String {
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
        for version_data in self.data.values() {
            for repo_data in version_data.values() {
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
        for (i, repo) in repos.iter().enumerate() {
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
            output.push_str(&format!("- {version}\n"));
        }
        output.push('\n');

        // Results for each benchmark type
        for (benchmark_name, version_data) in &self.data {
            output.push_str(&self.generate_benchmark_table(
                benchmark_name,
                version_data,
                versions,
                repos,
            ));
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

    /// Generate a complete markdown table for a single benchmark type
    ///
    /// This includes the section header, table header, separator, and all rows
    fn generate_benchmark_table(
        &self,
        benchmark_name: &str,
        version_data: &HashMap<String, HashMap<String, CriterionResult>>,
        versions: &[String],
        repos: &[RepoConfig],
    ) -> String {
        let mut output = String::new();

        // Section header
        output.push_str(&format!("## {}\n\n", format_benchmark_name(benchmark_name)));

        // Create table header
        output.push_str("| Repository |");
        for version in versions {
            output.push_str(&format!(" {version} |"));
        }
        output.push('\n');

        // Table separator
        output.push_str("|------------|");
        for _ in versions {
            output.push_str("----------|");
        }
        output.push('\n');

        // Table rows
        output.push_str(&generate_table_rows(version_data, versions, repos));
        output.push('\n');

        output
    }
}

/// Generate table rows for benchmark results
///
/// This function creates the markdown table rows for each repository,
/// showing the benchmark results for each version.
fn generate_table_rows(
    version_data: &HashMap<String, HashMap<String, CriterionResult>>,
    versions: &[String],
    repos: &[RepoConfig],
) -> String {
    let mut output = String::new();

    for repo in repos {
        output.push_str(&format!("| {} |", repo.name));

        for version in versions {
            let cell_content = get_benchmark_cell_content(version_data, version, &repo.name);
            output.push_str(&format!(" {cell_content} |"));
        }

        output.push('\n');
    }

    output
}

/// Get the content for a single benchmark table cell
///
/// Returns the formatted duration or "N/A" if no data is available.
/// The nested if-let statements handle the following cases:
/// 1. Check if version data exists
/// 2. Check if repository data exists for this version
fn get_benchmark_cell_content(
    version_data: &HashMap<String, HashMap<String, CriterionResult>>,
    version: &str,
    repo_name: &str,
) -> String {
    // Check if we have data for this version
    if let Some(repo_data) = version_data.get(version) {
        // Check if we have data for this repository
        if let Some(result) = repo_data.get(repo_name) {
            return format_duration(result.mean.point_estimate, &result.unit);
        }
    }

    "N/A".to_string()
}

pub fn format_benchmark_name(name: &str) -> String {
    match name {
        "forge-test" => "Forge Test",
        "forge-build-no-cache" => "Forge Build (No Cache)",
        "forge-build-with-cache" => "Forge Build (With Cache)",
        "forge-fuzz-test" => "Forge Fuzz Test",
        "forge-coverage" => "Forge Coverage",
        _ => name,
    }
    .to_string()
}

pub fn format_duration(nanos: f64, unit: &str) -> String {
    match unit {
        "ns" => {
            if nanos < 1_000.0 {
                format!("{nanos:.2} ns")
            } else if nanos < 1_000_000.0 {
                format!("{:.2} µs", nanos / 1_000.0)
            } else if nanos < 1_000_000_000.0 {
                format!("{:.2} ms", nanos / 1_000_000.0)
            } else {
                format!("{:.2} s", nanos / 1_000_000_000.0)
            }
        }
        _ => format!("{nanos:.2} {unit}"),
    }
}

pub fn get_rustc_version() -> Result<String> {
    let output = Command::new("rustc").arg("--version").output()?;

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
