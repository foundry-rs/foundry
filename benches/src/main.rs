use clap::Parser;
use eyre::{OptionExt, Result, WrapErr};
use foundry_bench::{
    criterion_types::{Change, ChangeEstimate, CriterionResult, Estimate},
    get_forge_version,
    results::BenchmarkResults,
    switch_foundry_version, RepoConfig, BENCHMARK_REPOS, FOUNDRY_VERSIONS,
};
use foundry_common::sh_println;
use once_cell::sync::Lazy;
use rayon::prelude::*;
use std::{
    fs::File,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Mutex,
};

const ALL_BENCHMARKS: [&str; 5] = [
    "forge_test",
    "forge_build_no_cache",
    "forge_build_with_cache",
    "forge_fuzz_test",
    "forge_coverage",
];

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

    /// Directory where the aggregated benchmark results will be written.
    #[clap(long, default_value = ".")]
    output_dir: PathBuf,

    /// Name of the output file (default: LATEST.md)
    #[clap(long, default_value = "LATEST.md")]
    output_file: String,

    /// Run only specific benchmarks (comma-separated:
    /// forge_test,forge_build_no_cache,forge_build_with_cache,forge_fuzz_test,forge_coverage)
    #[clap(long, value_delimiter = ',')]
    benchmarks: Option<Vec<String>>,

    /// Run only on specific repositories (comma-separated in org/repo[:rev] format:
    /// ithacaxyz/account,Vectorized/solady:main,foundry-rs/foundry:v1.0.0)
    #[clap(long, value_delimiter = ',')]
    repos: Option<Vec<String>>,
}

/// Mutex to prevent concurrent foundryup calls
static FOUNDRY_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));
fn switch_version_safe(version: &str) -> Result<()> {
    let _lock = FOUNDRY_LOCK.lock().unwrap();
    switch_foundry_version(version)
}

fn run_benchmark(
    name: &str,
    version: &str,
    repos: &[RepoConfig],
    verbose: bool,
) -> Result<Vec<CriterionResult>> {
    // Setup paths
    let criterion_dir = PathBuf::from("target/criterion");
    let dir_name = name.replace('_', "-");
    let group_dir = criterion_dir.join(&dir_name).join(version);

    // Set environment variable for the current version
    std::env::set_var("FOUNDRY_BENCH_CURRENT_VERSION", version);

    // Set environment variable for the repos to benchmark
    // Use org/repo format for proper parsing in benchmarks
    let repo_specs: Vec<String> = repos.iter().map(|r| format!("{}/{}", r.org, r.repo)).collect();
    std::env::set_var("FOUNDRY_BENCH_REPOS", repo_specs.join(","));

    // Run the benchmark
    let mut cmd = Command::new("cargo");
    cmd.args(["bench", "--bench", name]);

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

    // Collect benchmark results from criterion output
    let results = collect_benchmark_results(&group_dir, &dir_name, version, repos)?;
    let _ = sh_println!("Total results collected: {}", results.len());
    Ok(results)
}

#[allow(unused_must_use)]
fn main() -> Result<()> {
    let cli = Cli::parse();

    // Determine versions to test
    let versions = if let Some(v) = cli.versions {
        v
    } else {
        FOUNDRY_VERSIONS.iter().map(|&s| s.to_string()).collect()
    };

    // Determine repos to test
    let repos = if let Some(repo_specs) = cli.repos {
        repo_specs
            .iter()
            .map(|spec| RepoConfig::try_from(spec.as_str()))
            .collect::<Result<Vec<_>>>()?
    } else {
        BENCHMARK_REPOS.clone()
    };

    sh_println!("🚀 Foundry Benchmark Runner");
    sh_println!("Running with versions: {}", versions.join(", "));
    sh_println!(
        "Running on repos: {}",
        repos.iter().map(|r| format!("{}/{}", r.org, r.repo)).collect::<Vec<_>>().join(", ")
    );

    // Install versions if requested
    if cli.force_install {
        install_foundry_versions(&versions)?;
    }

    // Determine benchmarks to run

    let benchmarks = if let Some(b) = cli.benchmarks {
        b.into_iter().filter(|b| ALL_BENCHMARKS.contains(&b.as_str())).collect()
    } else {
        // Default: run all benchmarks except fuzz tests and coverage (which can be slow)
        vec!["forge_test", "forge_build_no_cache", "forge_build_with_cache"]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>()
    };

    sh_println!(" Running benchmarks: {}", benchmarks.join(", "));

    let mut results = BenchmarkResults::new();
    // Set the first version as baseline
    if let Some(first_version) = versions.first() {
        results.set_baseline_version(first_version.clone());
    }

    // Run benchmarks for each version
    for version in &versions {
        sh_println!("🔧 Switching to Foundry version: {version}");
        switch_version_safe(version)?;

        // Verify the switch
        let current = get_forge_version()?;
        sh_println!("Current version: {}", current.trim());

        // Run each benchmark in parallel
        let bench_results: Vec<(String, Vec<CriterionResult>)> = benchmarks
            .par_iter()
            .map(|benchmark| -> Result<(String, Vec<CriterionResult>)> {
                sh_println!("Running {benchmark} benchmark...");
                let results = run_benchmark(benchmark, version, &repos, cli.verbose)?;
                Ok((benchmark.clone(), results))
            })
            .collect::<Result<_>>()?;

        // Aggregate the results and add them to BenchmarkResults
        for (benchmark, bench_results) in bench_results {
            sh_println!("Processing {} results for {}", bench_results.len(), benchmark);
            for result in bench_results {
                // Parse ID format: benchmark-name/version/repo
                let parts: Vec<&str> = result.id.split('/').collect();
                if parts.len() >= 3 {
                    let bench_type = parts[0].to_string();
                    // Skip parts[1] which is the version (already known)
                    let repo = parts[2].to_string();

                    // Debug: show change info if present
                    if let Some(change) = &result.change {
                        if let Some(mean) = &change.mean {
                            sh_println!(
                                "Change from baseline: {:.2}% ({})",
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

    // Generate markdown report
    sh_println!("📝 Generating report...");
    let markdown = results.generate_markdown(&versions, &repos);
    let output_path = cli.output_dir.join(cli.output_file);
    let mut file = File::create(&output_path).wrap_err("Failed to create output file")?;
    file.write_all(markdown.as_bytes()).wrap_err("Failed to write output file")?;
    sh_println!("✅ Report written to: {}", output_path.display());

    Ok(())
}

#[allow(unused_must_use)]
fn install_foundry_versions(versions: &[String]) -> Result<()> {
    sh_println!("Installing Foundry versions...");

    for version in versions {
        sh_println!("Installing {version}...");

        let status = Command::new("foundryup")
            .args(["--install", version])
            .status()
            .wrap_err("Failed to run foundryup")?;

        if !status.success() {
            eyre::bail!("Failed to install Foundry version: {}", version);
        }
    }

    sh_println!("✅ All versions installed successfully");
    Ok(())
}

/// Collect benchmark results from Criterion output directory
///
/// This function reads the Criterion JSON output files and constructs CriterionResult objects.
/// It processes:
/// - benchmark.json for basic benchmark info
/// - estimates.json for mean performance values
/// - change/estimates.json for performance change data (if available)
#[allow(unused_must_use)]
fn collect_benchmark_results(
    group_dir: &PathBuf,
    benchmark_name: &str,
    version: &str,
    repos: &[RepoConfig],
) -> Result<Vec<CriterionResult>> {
    let mut results = Vec::new();

    sh_println!("Looking for results in: {}", group_dir.display());
    if !group_dir.exists() {
        eyre::bail!("Benchmark directory does not exist: {}", group_dir.display());
    }

    // Iterate through each repository directory
    for entry in std::fs::read_dir(group_dir)? {
        let entry = entry?;
        let path = entry.path();

        if !path.is_dir() {
            sh_println!("Skipping non-directory entry: {}", path.display());
            continue;
        }

        let repo_name = path
            .file_name()
            .ok_or_eyre("Failed to get repo_name using path")?
            .to_string_lossy()
            .to_string();

        // Only process repos that are in the specified repos list
        let is_valid_repo = repos.iter().any(|r| r.name == repo_name);
        if !is_valid_repo {
            sh_println!("Skipping unknown repo: {repo_name}");
            continue;
        }

        sh_println!("Processing repo: {repo_name}");

        // Process the benchmark results for this repository
        if let Some(result) = process_repo_benchmark(&path, benchmark_name, version, &repo_name)? {
            results.push(result);
        }
    }

    Ok(results)
}

/// Process benchmark results for a single repository
///
/// Returns Some(CriterionResult) if valid results are found, None otherwise
fn process_repo_benchmark(
    repo_path: &Path,
    benchmark_name: &str,
    version: &str,
    repo_name: &str,
) -> Result<Option<CriterionResult>> {
    let benchmark_json = repo_path.join("new/benchmark.json");

    if !benchmark_json.exists() {
        eyre::bail!(
            "Benchmark JSON file does not exist for {}: {}",
            repo_name,
            benchmark_json.display()
        );
    }

    // Create result ID
    let id = format!("{benchmark_name}/{version}/{repo_name}");

    // Read new estimates for mean value
    let mean_estimate = read_mean_estimate(repo_path, repo_name)?;
    // Read change data if available
    let change = read_change_data(repo_path)?;

    Ok(Some(CriterionResult { id, mean: mean_estimate, unit: "ns".to_string(), change }))
}

/// Read mean estimate from estimates.json
fn read_mean_estimate(repo_path: &Path, repo_name: &str) -> Result<Estimate> {
    let estimates_json = repo_path.join("new/estimates.json");
    if !estimates_json.exists() {
        eyre::bail!(
            "Estimates JSON file does not exist for {}: {}",
            repo_name,
            estimates_json.display()
        );
    }

    let estimates_content = std::fs::read_to_string(&estimates_json)?;
    let estimates = serde_json::from_str::<serde_json::Value>(&estimates_content)?;
    let mean_obj = estimates.get("mean").ok_or_eyre("No mean value found in estimates.json")?;
    let estimate = serde_json::from_value::<Estimate>(mean_obj.clone())
        .wrap_err("Failed to parse mean estimate from estimates.json")?;
    Ok(estimate)
}

/// Read change data from change/estimates.json if it exists
fn read_change_data(repo_path: &Path) -> Result<Option<Change>> {
    let change_json = repo_path.join("change/estimates.json");

    if !change_json.exists() {
        return Ok(None);
    }

    let change_content = std::fs::read_to_string(&change_json)?;
    let change_data = serde_json::from_str::<serde_json::Value>(&change_content)?;

    let mean_change = change_data.get("mean").and_then(|m| {
        // The change is in decimal format (e.g., 0.03 = 3%)
        let decimal = m["point_estimate"].as_f64()?;
        Some(ChangeEstimate {
            estimate: decimal * 100.0, // Convert to percentage
            unit: "%".to_string(),
        })
    });

    Ok(Some(Change { mean: mean_change, median: None, change: None }))
}
