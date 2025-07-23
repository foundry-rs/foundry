use crate::RepoConfig;
use eyre::Result;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, process::Command};

/// Hyperfine benchmark result
#[derive(Debug, Deserialize, Serialize)]
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

/// Hyperfine JSON output format
#[derive(Debug, Deserialize, Serialize)]
pub struct HyperfineOutput {
    pub results: Vec<HyperfineResult>,
}

/// Aggregated benchmark results
#[derive(Debug, Default)]
pub struct BenchmarkResults {
    /// Map of benchmark_name -> version -> repo -> result
    pub data: HashMap<String, HashMap<String, HashMap<String, HyperfineResult>>>,
    /// Track the baseline version for comparison
    pub baseline_version: Option<String>,
    /// Map of version name -> full version details
    pub version_details: HashMap<String, String>,
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
        result: HyperfineResult,
    ) {
        self.data
            .entry(benchmark.to_string())
            .or_default()
            .entry(version.to_string())
            .or_default()
            .insert(repo.to_string(), result);
    }

    pub fn add_version_details(&mut self, version: &str, details: String) {
        self.version_details.insert(version.to_string(), details);
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
            if let Some(details) = self.version_details.get(version) {
                output.push_str(&format!("- **{version}**: {}\n", details.trim()));
            } else {
                output.push_str(&format!("- {version}\n"));
            }
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
        version_data: &HashMap<String, HashMap<String, HyperfineResult>>,
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
    version_data: &HashMap<String, HashMap<String, HyperfineResult>>,
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
    version_data: &HashMap<String, HashMap<String, HyperfineResult>>,
    version: &str,
    repo_name: &str,
) -> String {
    // Check if we have data for this version
    if let Some(repo_data) = version_data.get(version) &&
    // Check if we have data for this repository
        let Some(result) = repo_data.get(repo_name)
    {
        return format_duration_seconds(result.mean);
    }

    "N/A".to_string()
}

pub fn format_benchmark_name(name: &str) -> String {
    match name {
        "forge_test" => "Forge Test",
        "forge_build_no_cache" => "Forge Build (No Cache)",
        "forge_build_with_cache" => "Forge Build (With Cache)",
        "forge_fuzz_test" => "Forge Fuzz Test",
        "forge_coverage" => "Forge Coverage",
        "forge_isolate_test" => "Forge Test (Isolated)",
        _ => name,
    }
    .to_string()
}

pub fn format_duration_seconds(seconds: f64) -> String {
    if seconds < 0.001 {
        format!("{:.2} ms", seconds * 1000.0)
    } else if seconds < 1.0 {
        format!("{seconds:.3} s")
    } else if seconds < 60.0 {
        format!("{seconds:.2} s")
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
