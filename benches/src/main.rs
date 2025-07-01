use clap::Parser;
use color_eyre::eyre::{Result, WrapErr};
use foundry_bench::{
    criterion_types::{Change, ChangeEstimate, ConfidenceInterval, CriterionResult, Estimate},
    get_forge_version,
    results::BenchmarkResults,
    switch_foundry_version, BENCHMARK_REPOS, FOUNDRY_VERSIONS,
};
use once_cell::sync::Lazy;
use rayon::prelude::*;
use std::{
    fs::File,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    sync::Mutex,
};

/// Foundry Benchmark Runner
#[derive(Parser, Debug)]
#[clap(name = "foundry-bench", about = "Run Foundry benchmarks across multiple versions")]
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

/// Mutex to prevent concurrent foundryup calls
static FOUNDRY_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));
fn switch_version_safe(version: &str) -> Result<()> {
    let _lock = FOUNDRY_LOCK.lock().unwrap();
    switch_foundry_version(version)
}

fn run_benchmark(name: &str, version: &str, verbose: bool) -> Result<Vec<CriterionResult>> {
    // Setup paths
    let criterion_dir = PathBuf::from("../target/criterion");
    let dir_name = name.replace('_', "-");
    let group_dir = criterion_dir.join(&dir_name).join(version);

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

        // Run each benchmark in parallel
        let bench_results: Vec<(String, Vec<CriterionResult>)> = benchmarks
            .par_iter()
            .map(|benchmark| {
                println!("    Running {} benchmark...", benchmark);
                let results = run_benchmark(benchmark, version, cli.verbose).unwrap_or_else(|e| {
                    eprintln!("    Error running benchmark {}: {}", benchmark, e);
                    Vec::new()
                });
                (benchmark.clone(), results)
            })
            .collect();

        for (benchmark, bench_results) in bench_results {
            println!("      Processing {} results for {}", bench_results.len(), benchmark);
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
