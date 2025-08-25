use clap::Parser;
use eyre::{Result, WrapErr};
use foundry_bench::{
    BENCHMARK_REPOS, BenchmarkProject, FOUNDRY_VERSIONS, RUNS, RepoConfig, get_forge_version,
    get_forge_version_details,
    results::{BenchmarkResults, HyperfineResult},
    switch_foundry_version,
};
use foundry_common::sh_println;
use rayon::prelude::*;
use std::{fs, path::PathBuf, process::Command, sync::Mutex};

const ALL_BENCHMARKS: [&str; 6] = [
    "forge_test",
    "forge_build_no_cache",
    "forge_build_with_cache",
    "forge_fuzz_test",
    "forge_coverage",
    "forge_isolate_test",
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
static FOUNDRY_LOCK: Mutex<()> = Mutex::new(());
fn switch_version_safe(version: &str) -> Result<()> {
    let _lock = FOUNDRY_LOCK.lock().unwrap();
    switch_foundry_version(version)
}

#[allow(unused_must_use)]
fn main() -> Result<()> {
    color_eyre::install()?;
    let cli = Cli::parse();

    // Check if hyperfine is installed
    let hyperfine_check = Command::new("hyperfine").arg("--version").output();
    if hyperfine_check.is_err() || !hyperfine_check.unwrap().status.success() {
        eyre::bail!(
            "hyperfine is not installed. Please install it first: https://github.com/sharkdp/hyperfine"
        );
    }

    // Determine versions to test
    let versions = if let Some(v) = cli.versions {
        v
    } else {
        FOUNDRY_VERSIONS.iter().map(|&s| s.to_string()).collect()
    };

    // Get repo configurations
    let repos = if let Some(repo_specs) = cli.repos.clone() {
        repo_specs.iter().map(|spec| spec.parse::<RepoConfig>()).collect::<Result<Vec<_>>>()?
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

    sh_println!("Running benchmarks: {}", benchmarks.join(", "));

    let mut results = BenchmarkResults::new();
    // Set the first version as baseline
    if let Some(first_version) = versions.first() {
        results.set_baseline_version(first_version.clone());
    }

    // Setup all projects upfront before version loop
    sh_println!("📦 Setting up projects to benchmark");
    let projects: Vec<(RepoConfig, BenchmarkProject)> = repos
        .par_iter()
        .map(|repo_config| -> Result<(RepoConfig, BenchmarkProject)> {
            sh_println!("Setting up {}/{}", repo_config.org, repo_config.repo);
            let project = BenchmarkProject::setup(repo_config).wrap_err(format!(
                "Failed to setup project for {}/{}",
                repo_config.org, repo_config.repo
            ))?;
            Ok((repo_config.clone(), project))
        })
        .collect::<Result<Vec<_>>>()?;

    sh_println!("✅ All projects setup complete");

    // Create a list of all benchmark tasks (same for all versions)
    let benchmark_tasks: Vec<_> = projects
        .iter()
        .flat_map(|(repo_config, project)| {
            benchmarks
                .iter()
                .map(move |benchmark| (repo_config.clone(), project, benchmark.clone()))
        })
        .collect();

    sh_println!("Will run {} benchmark tasks per version", benchmark_tasks.len());

    // Run benchmarks for each version
    for version in &versions {
        sh_println!("🔧 Switching to Foundry version: {version}");
        switch_version_safe(version)?;

        // Verify the switch and capture full version details
        let current = get_forge_version()?;
        sh_println!("Current version: {}", current.trim());

        // Get and store the full version details with commit hash and date
        let version_details = get_forge_version_details()?;
        results.add_version_details(version, version_details);

        sh_println!("Running benchmark tasks for version {version}...");

        // Run all benchmarks sequentially
        let version_results = benchmark_tasks
            .iter()
            .map(|(repo_config, project, benchmark)| -> Result<(String, String, HyperfineResult)> {
                sh_println!("Running {} on {}/{}", benchmark, repo_config.org, repo_config.repo);

                // Determine runs based on benchmark type
                let runs = match benchmark.as_str() {
                    "forge_coverage" => 1, // Coverage runs only once as an exception
                    _ => RUNS,             // Use default RUNS constant for all other benchmarks
                };

                // Run the appropriate benchmark
                let result = project.run(benchmark, version, runs, cli.verbose);

                match result {
                    Ok(hyperfine_result) => {
                        sh_println!(
                            "  {} on {}/{}: {:.3}s ± {:.3}s",
                            benchmark,
                            repo_config.org,
                            repo_config.repo,
                            hyperfine_result.mean,
                            hyperfine_result.stddev.unwrap_or(0.0)
                        );
                        Ok((repo_config.name.clone(), benchmark.clone(), hyperfine_result))
                    }
                    Err(e) => {
                        eyre::bail!(
                            "Benchmark {} failed for {}/{}: {}",
                            benchmark,
                            repo_config.org,
                            repo_config.repo,
                            e
                        );
                    }
                }
            })
            .collect::<Result<Vec<_>>>()?;

        // Add all collected results to the main results structure
        for (repo_name, benchmark, hyperfine_result) in version_results {
            results.add_result(&benchmark, version, &repo_name, hyperfine_result);
        }
    }

    // Generate markdown report
    sh_println!("📝 Generating report...");
    let markdown = results.generate_markdown(&versions, &repos);
    let output_path = cli.output_dir.join(cli.output_file);
    fs::write(&output_path, markdown).wrap_err("Failed to write output file")?;
    sh_println!("✅ Report written to: {}", output_path.display());

    Ok(())
}

#[allow(unused_must_use)]
fn install_foundry_versions(versions: &[String]) -> Result<()> {
    sh_println!("Installing Foundry versions...");

    for version in versions {
        sh_println!("Installing {version}...");

        let status = Command::new("foundryup")
            .args(["--install", version, "--force"])
            .status()
            .wrap_err("Failed to run foundryup")?;

        if !status.success() {
            eyre::bail!("Failed to install Foundry version: {}", version);
        }
    }

    sh_println!("✅ All versions installed successfully");
    Ok(())
}
