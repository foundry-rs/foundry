use clap::Parser;
use eyre::{Result, WrapErr};
use foundry_bench::{
    ALL_BENCHMARKS, BENCH_SUITE_DIR, BENCHMARK_REPOS, BenchmarkProject, RUNS, RepoConfig,
    get_forge_version_details, install_foundry_version, results::BenchmarkResults,
    switch_foundry_version,
};
use foundry_common::sh_println;
use rayon::prelude::*;
use std::{fs, path::PathBuf, process::Command, sync::Mutex};

/// Foundry Benchmark Runner
///
/// Compare two Foundry versions (baseline vs feature) across test repositories. Produces
/// structured JSON output suitable for automated regression detection and CI integration.
#[derive(Parser, Debug)]
#[clap(name = "foundry-bench")]
struct Cli {
    /// Baseline Foundry version (e.g., stable, nightly, v1.2.0, or a commit hash)
    #[clap(long, default_value = "stable")]
    baseline: String,

    /// Feature Foundry version to compare against the baseline
    #[clap(long, default_value = "nightly")]
    feature: String,

    /// Force install Foundry versions before benchmarking
    #[clap(long)]
    force_install: bool,

    /// Show verbose output (hyperfine --show-output)
    #[clap(long)]
    verbose: bool,

    /// Directory where benchmark results will be written
    #[clap(long, default_value = ".")]
    output_dir: PathBuf,

    /// Name of the markdown output file
    #[clap(long, default_value = "LATEST.md")]
    output_file: String,

    /// Number of runs per benchmark (default: 5)
    #[clap(long, default_value_t = RUNS)]
    runs: u32,

    /// Comma-separated list of benchmarks to run
    #[clap(long, value_delimiter = ',')]
    benchmarks: Option<Vec<String>>,

    /// Comma-separated list of repos in org/repo[:rev] format.
    /// Ignored when --local is used.
    #[clap(long, value_delimiter = ',')]
    repos: Option<Vec<String>>,

    /// Use the built-in bench-suite fixtures instead of cloning external repos.
    /// This is the default when --repos is not specified.
    #[clap(long, default_value_t = true)]
    local: bool,

    /// Output structured JSON bundle instead of markdown
    #[clap(long)]
    json: bool,

    /// Noise threshold percentage for verdict classification (default: 3.0%)
    #[clap(long, default_value_t = 3.0)]
    noise_threshold: f64,

    /// RPC URL for fork-mode benchmarks (required when forge_fork_test is selected)
    #[clap(long)]
    fork_url: Option<String>,
}

/// Mutex to prevent concurrent foundryup calls.
static FOUNDRY_LOCK: Mutex<()> = Mutex::new(());
fn switch_version_safe(version: &str) -> Result<()> {
    let _lock = FOUNDRY_LOCK.lock().unwrap();
    switch_foundry_version(version)
}

#[allow(unused_must_use)]
fn main() -> Result<()> {
    color_eyre::install()?;
    let cli = Cli::parse();

    // Preflight: check hyperfine.
    let hyperfine_check = Command::new("hyperfine").arg("--version").output();
    if hyperfine_check.is_err() || !hyperfine_check.unwrap().status.success() {
        eyre::bail!(
            "hyperfine is not installed. Please install it first: \
             https://github.com/sharkdp/hyperfine"
        );
    }

    let use_local = cli.local && cli.repos.is_none();

    let repos: Vec<RepoConfig> = if use_local {
        vec![]
    } else if let Some(repo_specs) = cli.repos.clone() {
        repo_specs.iter().map(|spec| spec.parse::<RepoConfig>()).collect::<Result<Vec<_>>>()?
    } else {
        BENCHMARK_REPOS.clone()
    };

    sh_println!("🚀 Foundry Benchmark Runner (baseline vs feature)");
    sh_println!("  Baseline: {}", cli.baseline);
    sh_println!("  Feature:  {}", cli.feature);
    if use_local {
        sh_println!("  Suite:    built-in bench-suite");
    } else {
        sh_println!(
            "  Repos:    {}",
            repos.iter().map(|r| format!("{}/{}", r.org, r.repo)).collect::<Vec<_>>().join(", ")
        );
    }

    if cli.force_install {
        sh_println!("📦 Installing Foundry versions...");
        install_foundry_version(&cli.baseline)?;
        install_foundry_version(&cli.feature)?;
        sh_println!("✅ Versions installed");
    }

    let benchmarks: Vec<String> = if let Some(b) = cli.benchmarks {
        b.into_iter().filter(|b| ALL_BENCHMARKS.contains(&b.as_str())).collect()
    } else {
        vec!["forge_test", "forge_fuzz_test", "forge_invariant_test"]
            .into_iter()
            .map(String::from)
            .collect()
    };

    // Validate fork URL requirement.
    let needs_fork =
        benchmarks.iter().any(|b| b == "forge_fork_test" || b == "forge_multifork_test");
    if needs_fork && cli.fork_url.is_none() {
        eyre::bail!(
            "forge_fork_test and forge_multifork_test require --fork-url to be set. \
             Example: --fork-url https://eth.merkle.io"
        );
    }

    sh_println!("Running benchmarks: {}", benchmarks.join(", "));

    // Ensure output directory exists.
    fs::create_dir_all(&cli.output_dir)?;

    // Setup projects.
    sh_println!("📦 Setting up test projects...");
    let projects: Vec<(RepoConfig, BenchmarkProject)> = if use_local {
        let suite_path = std::path::Path::new(BENCH_SUITE_DIR);
        if !suite_path.exists() {
            eyre::bail!(
                "Built-in bench-suite not found at '{}'. Run from the foundry repo root.",
                BENCH_SUITE_DIR
            );
        }
        let project = BenchmarkProject::setup_local(suite_path)
            .wrap_err("Failed to setup local bench-suite")?;
        let config = RepoConfig {
            name: "bench-suite".to_string(),
            org: "local".to_string(),
            repo: "bench-suite".to_string(),
            rev: String::new(),
        };
        vec![(config, project)]
    } else {
        repos
            .par_iter()
            .map(|repo_config| -> Result<(RepoConfig, BenchmarkProject)> {
                sh_println!("  Setting up {}/{}", repo_config.org, repo_config.repo);
                let project = BenchmarkProject::setup(repo_config).wrap_err(format!(
                    "Failed to setup project for {}/{}",
                    repo_config.org, repo_config.repo
                ))?;
                Ok((repo_config.clone(), project))
            })
            .collect::<Result<Vec<_>>>()?
    };
    sh_println!("✅ All projects ready\n");

    let mut results = BenchmarkResults::new();

    // Run baseline.
    sh_println!("═══════════════════════════════════════════");
    sh_println!("  BASELINE: {}", cli.baseline);
    sh_println!("═══════════════════════════════════════════");
    switch_version_safe(&cli.baseline)?;
    let baseline_version = get_forge_version_details()?;
    sh_println!("  Version: {baseline_version}");

    run_benchmarks(
        &projects,
        &benchmarks,
        "baseline",
        &mut results,
        cli.runs,
        cli.verbose,
        cli.fork_url.as_deref(),
    )?;

    // Run feature.
    sh_println!("\n═══════════════════════════════════════════");
    sh_println!("  FEATURE: {}", cli.feature);
    sh_println!("═══════════════════════════════════════════");
    switch_version_safe(&cli.feature)?;
    let feature_version = get_forge_version_details()?;
    sh_println!("  Version: {feature_version}");

    run_benchmarks(
        &projects,
        &benchmarks,
        "feature",
        &mut results,
        cli.runs,
        cli.verbose,
        cli.fork_url.as_deref(),
    )?;

    // Generate output.
    sh_println!("\n📝 Generating report...");

    if cli.json {
        let bundle = results.to_bundle(
            &cli.baseline,
            &cli.feature,
            &baseline_version,
            &feature_version,
            cli.noise_threshold,
        );
        let json = serde_json::to_string_pretty(&bundle)?;

        let json_path = cli.output_dir.join("bundle.json");
        fs::write(&json_path, &json).wrap_err("Failed to write bundle JSON")?;
        sh_println!("✅ Bundle written to: {}", json_path.display());

        // Also print to stdout for piping.
        sh_println!("{json}");
    } else {
        let markdown = results.generate_markdown(
            &cli.baseline,
            &cli.feature,
            &baseline_version,
            &feature_version,
            &repos,
            cli.noise_threshold,
        );
        let output_path = cli.output_dir.join(&cli.output_file);
        fs::write(&output_path, &markdown).wrap_err("Failed to write markdown")?;
        sh_println!("✅ Report written to: {}", output_path.display());
    }

    // Print summary verdict.
    let comparisons = results.compare(cli.noise_threshold);
    let overall = foundry_bench::results::BenchmarkResults::overall_verdict(&comparisons);
    sh_println!("\n{} Overall verdict: {}", overall.emoji(), overall);

    // Exit with non-zero if regression detected.
    if overall == foundry_bench::results::Verdict::Regressed {
        std::process::exit(1);
    }

    Ok(())
}

/// Run all benchmarks for a given side (baseline or feature).
#[allow(unused_must_use)]
fn run_benchmarks(
    projects: &[(RepoConfig, BenchmarkProject)],
    benchmarks: &[String],
    side: &str,
    results: &mut BenchmarkResults,
    runs: u32,
    verbose: bool,
    fork_url: Option<&str>,
) -> Result<()> {
    for (repo_config, project) in projects {
        for benchmark in benchmarks {
            sh_println!("  ▶ {benchmark} on {}/{}...", repo_config.org, repo_config.repo);

            let bench_runs = match benchmark.as_str() {
                "forge_coverage" => 1,
                _ => runs,
            };

            let result =
                project.run(benchmark, side, bench_runs, verbose, fork_url).wrap_err(format!(
                    "{benchmark} failed for {}/{} on {side}",
                    repo_config.org, repo_config.repo
                ))?;

            sh_println!("    {:.3}s ± {:.3}s", result.mean, result.stddev.unwrap_or(0.0));
            results.add_result(benchmark, side, &repo_config.name, result);
        }
    }

    Ok(())
}
