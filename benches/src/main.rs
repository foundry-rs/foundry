use clap::Parser;
use color_eyre::eyre::{Result, WrapErr};
use foundry_bench::{get_forge_version, switch_foundry_version, BENCHMARK_REPOS, FOUNDRY_VERSIONS};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::File,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
};

/// Foundry Benchmark Runner
#[derive(Parser, Debug)]
#[clap(
    name = "foundry-bench",
    version = "1.0.0",
    about = "Run Foundry benchmarks across multiple versions"
)]
struct Cli {
    /// Comma-separated list of Foundry versions to test (e.g., stable,nightly,v1.2.0)
    #[clap(long, value_delimiter = ',')]
    versions: Option<Vec<String>>,

    /// Force install Foundry versions
    #[clap(long)]
    force_install: bool,

    /// Show verbose output
    #[clap(long)]
    verbose: bool,

    /// Output directory for benchmark results
    #[clap(long, default_value = ".")]
    output_dir: PathBuf,

    /// Run only specific benchmarks (comma-separated:
    /// forge_test,forge_build_no_cache,forge_build_with_cache)
    #[clap(long, value_delimiter = ',')]
    benchmarks: Option<Vec<String>>,
}

/// Benchmark result from Criterion JSON output
#[derive(Debug, Deserialize, Serialize)]
struct CriterionResult {
    reason: String,
    id: Option<String>,
    report_directory: Option<String>,
    iteration_count: Option<Vec<f64>>,
    measured_values: Option<Vec<f64>>,
    unit: Option<String>,
    throughput: Option<Vec<Throughput>>,
    typical: Option<Estimate>,
    mean: Option<Estimate>,
    median: Option<Estimate>,
    slope: Option<Estimate>,
    change: Option<Change>,
}

#[derive(Debug, Deserialize, Serialize)]
struct Throughput {
    per_iteration: u64,
    unit: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct Estimate {
    confidence_interval: ConfidenceInterval,
    point_estimate: f64,
    standard_error: f64,
}

#[derive(Debug, Deserialize, Serialize)]
struct ConfidenceInterval {
    confidence_level: f64,
    lower_bound: f64,
    upper_bound: f64,
}

#[derive(Debug, Deserialize, Serialize)]
struct Change {
    mean: Option<ChangeEstimate>,
    median: Option<ChangeEstimate>,
    change: Option<String>, // "NoChange", "Improved", or "Regressed"
}

#[derive(Debug, Deserialize, Serialize)]
struct ChangeEstimate {
    estimate: f64,
    unit: String,
}

/// Aggregated benchmark results
#[derive(Debug)]
struct BenchmarkResults {
    /// Map of benchmark_name -> version -> repo -> result
    data: HashMap<String, HashMap<String, HashMap<String, CriterionResult>>>,
    /// Track the baseline version for comparison
    baseline_version: Option<String>,
}

impl BenchmarkResults {
    fn new() -> Self {
        Self { data: HashMap::new(), baseline_version: None }
    }

    fn set_baseline_version(&mut self, version: String) {
        self.baseline_version = Some(version);
    }

    fn add_result(&mut self, benchmark: &str, version: &str, repo: &str, result: CriterionResult) {
        self.data
            .entry(benchmark.to_string())
            .or_insert_with(HashMap::new)
            .entry(version.to_string())
            .or_insert_with(HashMap::new)
            .insert(repo.to_string(), result);
    }

    fn generate_markdown(&self, versions: &[String]) -> String {
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

fn format_benchmark_name(name: &str) -> String {
    match name {
        "forge-test" => "Forge Test Performance",
        "forge-build-no-cache" => "Forge Build Performance (No Cache)",
        "forge-build-with-cache" => "Forge Build Performance (With Cache)",
        _ => name,
    }
    .to_string()
}

fn format_duration(nanos: f64, unit: &str) -> String {
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

fn get_rustc_version() -> Result<String> {
    let output =
        Command::new("rustc").arg("--version").output().wrap_err("Failed to get rustc version")?;

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

use once_cell::sync::Lazy;
/// Mutex to prevent concurrent foundryup calls
use std::sync::Mutex;

static FOUNDRY_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

fn switch_version_safe(version: &str) -> Result<()> {
    let _lock = FOUNDRY_LOCK.lock().unwrap();
    switch_foundry_version(version)
}

fn run_benchmark(name: &str, version: &str, verbose: bool) -> Result<Vec<CriterionResult>> {
    println!("    Running {} benchmark...", name);

    // Setup paths
    let criterion_dir = PathBuf::from("../target/criterion");
    let dir_name = name.replace('_', "-");
    let group_dir = criterion_dir.join(&dir_name).join(version);

    // Always run fresh benchmarks
    println!("      Running fresh benchmark...");

    // Set environment variable for the current version
    std::env::set_var("FOUNDRY_BENCH_CURRENT_VERSION", version);

    // Run the benchmark
    let mut cmd = Command::new("cargo");
    cmd.args(&["bench", "--bench", name]);

    if verbose {
        cmd.stderr(Stdio::inherit());
        cmd.stdout(Stdio::inherit());
    } else {
        cmd.stderr(Stdio::null());
        cmd.stdout(Stdio::null());
    }

    let status = cmd.status().wrap_err("Failed to run benchmark")?;
    if !status.success() {
        eyre::bail!("Benchmark {} failed", name);
    }

    // Now read the results from the criterion output directory
    let mut results = Vec::new();
    println!("      Looking for results in: {}", group_dir.display());
    println!("      Directory exists: {}", group_dir.exists());
    if group_dir.exists() {
        println!("      Reading directory: {}", group_dir.display());
        for entry in std::fs::read_dir(&group_dir)? {
            let entry = entry?;
            let path = entry.path();
            println!("        Found entry: {}", path.display());
            if path.is_dir() {
                let repo_name = path.file_name().unwrap().to_string_lossy().to_string();
                
                // Only process repos that are in BENCHMARK_REPOS
                let is_valid_repo = BENCHMARK_REPOS.iter().any(|r| r.name == repo_name);
                if !is_valid_repo {
                    println!("        Skipping unknown repo: {}", repo_name);
                    continue;
                }
                
                println!("        Processing repo: {}", repo_name);
                let benchmark_json = path.join("new/benchmark.json");
                if benchmark_json.exists() {
                    let content = std::fs::read_to_string(&benchmark_json)?;
                    if let Ok(_benchmark_data) = serde_json::from_str::<serde_json::Value>(&content)
                    {
                        // Create a CriterionResult from the benchmark.json data
                        let id = format!("{}/{}/{}", dir_name, version, repo_name);

                        // Read estimates.json for the mean value
                        let estimates_json = path.join("new/estimates.json");
                        if estimates_json.exists() {
                            let estimates_content = std::fs::read_to_string(&estimates_json)?;
                            if let Ok(estimates) =
                                serde_json::from_str::<serde_json::Value>(&estimates_content)
                            {
                                if let Some(mean_obj) = estimates.get("mean") {
                                    let mean_estimate = Estimate {
                                        point_estimate: mean_obj["point_estimate"]
                                            .as_f64()
                                            .unwrap_or(0.0),
                                        standard_error: mean_obj["standard_error"]
                                            .as_f64()
                                            .unwrap_or(0.0),
                                        confidence_interval: ConfidenceInterval {
                                            confidence_level: 0.95,
                                            lower_bound: mean_obj["confidence_interval"]
                                                ["lower_bound"]
                                                .as_f64()
                                                .unwrap_or(0.0),
                                            upper_bound: mean_obj["confidence_interval"]
                                                ["upper_bound"]
                                                .as_f64()
                                                .unwrap_or(0.0),
                                        },
                                    };

                                    // Check for change data
                                    let change_json = path.join("change/estimates.json");
                                    let change = if change_json.exists() {
                                        let change_content = std::fs::read_to_string(&change_json)?;
                                        if let Ok(change_data) =
                                            serde_json::from_str::<serde_json::Value>(
                                                &change_content,
                                            )
                                        {
                                            let mean_change = change_data.get("mean").and_then(|m| {
                                                // The change is in decimal format (e.g., 0.03 = 3%)
                                                let decimal = m["point_estimate"].as_f64()?;
                                                Some(ChangeEstimate {
                                                    estimate: decimal * 100.0, // Convert to percentage
                                                    unit: "%".to_string(),
                                                })
                                            });
                                            Some(Change {
                                                mean: mean_change,
                                                median: None,
                                                change: None,
                                            })
                                        } else {
                                            None
                                        }
                                    } else {
                                        None
                                    };

                                    let result = CriterionResult {
                                        reason: "benchmark-complete".to_string(),
                                        id: Some(id.clone()),
                                        report_directory: None,
                                        iteration_count: None,
                                        measured_values: None,
                                        unit: Some("ns".to_string()),
                                        throughput: None,
                                        typical: None,
                                        mean: Some(mean_estimate),
                                        median: None,
                                        slope: None,
                                        change,
                                    };

                                    println!("      Found result: {}", id);
                                    results.push(result);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    println!("      Total results collected: {}", results.len());
    Ok(results)
}

fn install_foundry_versions(versions: &[String]) -> Result<()> {
    println!("Installing Foundry versions...");

    for version in versions {
        println!("  Installing {}...", version);

        let status = Command::new("foundryup")
            .args(&["--install", version])
            .status()
            .wrap_err("Failed to run foundryup")?;

        if !status.success() {
            eyre::bail!("Failed to install Foundry version: {}", version);
        }
    }

    println!("✅ All versions installed successfully");
    Ok(())
}

fn main() -> Result<()> {
    color_eyre::install()?;
    let cli = Cli::parse();

    // Determine versions to test
    let versions = if let Some(v) = cli.versions {
        v
    } else {
        FOUNDRY_VERSIONS.iter().map(|&s| s.to_string()).collect()
    };

    println!("🚀 Foundry Benchmark Runner");
    println!("📊 Testing versions: {}", versions.join(", "));

    // Install versions if requested
    if cli.force_install {
        install_foundry_versions(&versions)?;
    }

    // Determine benchmarks to run
    let all_benchmarks = vec!["forge_test", "forge_build_no_cache", "forge_build_with_cache"];
    let benchmarks = if let Some(b) = cli.benchmarks {
        b.into_iter().filter(|b| all_benchmarks.contains(&b.as_str())).collect()
    } else {
        // For testing, only run forge_build_with_cache
        // vec!["forge_build_with_cache".to_string()]
        all_benchmarks.into_iter().map(String::from).collect::<Vec<_>>()
    };

    println!("📈 Running benchmarks: {}", benchmarks.join(", "));

    let mut results = BenchmarkResults::new();

    // Set the first version as baseline
    if let Some(first_version) = versions.first() {
        results.set_baseline_version(first_version.clone());
    }

    // Run benchmarks for each version
    for version in &versions {
        println!("\n🔧 Switching to Foundry version: {}", version);
        switch_version_safe(version)?;

        // Verify the switch
        let current = get_forge_version()?;
        println!("   Current version: {}", current.trim());

        // Run each benchmark
        for benchmark in &benchmarks {
            let bench_results = run_benchmark(benchmark, version, cli.verbose)?;

            // Parse and store results
            println!("      Processing {} results", bench_results.len());
            for result in bench_results {
                if let Some(id) = &result.id {
                    println!("      Found result: {}", id);
                    // Parse ID format: benchmark-name/version/repo
                    let parts: Vec<&str> = id.split('/').collect();
                    if parts.len() >= 3 {
                        let bench_type = parts[0].to_string();
                        // Skip parts[1] which is the version (already known)
                        let repo = parts[2].to_string();

                        // Debug: show change info if present
                        if let Some(change) = &result.change {
                            if let Some(mean) = &change.mean {
                                println!(
                                    "        Change from baseline: {:.2}% ({})",
                                    mean.estimate,
                                    change.change.as_ref().unwrap_or(&"Unknown".to_string())
                                );
                            }
                        }

                        results.add_result(&bench_type, version, &repo, result);
                    }
                }
            }
        }
    }

    // Generate markdown report
    println!("\n📝 Generating report...");

    // Debug: print what we have collected
    println!("Collected data structure:");
    for (bench, version_data) in &results.data {
        println!("  Benchmark: {}", bench);
        for (version, repo_data) in version_data {
            println!("    Version: {}", version);
            for (repo, _) in repo_data {
                println!("      Repo: {}", repo);
            }
        }
    }

    let markdown = results.generate_markdown(&versions);

    let output_path = cli.output_dir.join("LATEST.md");
    let mut file = File::create(&output_path).wrap_err("Failed to create output file")?;
    file.write_all(markdown.as_bytes()).wrap_err("Failed to write output file")?;

    println!("✅ Report written to: {}", output_path.display());

    Ok(())
}
