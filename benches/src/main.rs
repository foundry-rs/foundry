use clap::Parser;
use eyre::{Result, WrapErr};
use foundry_bench::{
    BENCHMARK_REPOS, BenchmarkProject, FOUNDRY_VERSIONS, RUNS, RepoConfig,
    fuzz::{self, FuzzCampaignSpec},
    get_forge_version, get_forge_version_details, install_local_version,
    results::{BenchmarkResults, HyperfineResult},
    switch_foundry_version,
};
use foundry_common::sh_println;
use rayon::prelude::*;
use std::{collections::BTreeMap, fs, path::PathBuf, process::Command, sync::Mutex};

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

    /// Name of the output file. Defaults to LATEST.md unless --json-output is set
    /// without this flag, in which case no Markdown is written.
    #[clap(long)]
    output_file: Option<String>,

    /// Filename for a flat JSON summary (benchmark/repo -> mean_seconds).
    /// Resolved relative to --output-dir. Used by the nightly regression comparison script.
    ///
    /// Perf mode only — fails if combined with --fuzz-campaigns.
    #[clap(long, conflicts_with = "fuzz_campaigns")]
    json_output: Option<PathBuf>,

    /// Run only specific benchmarks (comma-separated:
    /// forge_test,forge_build_no_cache,forge_build_with_cache,forge_fuzz_test,forge_coverage).
    ///
    /// Perf mode only — fails if combined with --fuzz-campaigns.
    #[clap(long, value_delimiter = ',', conflicts_with = "fuzz_campaigns")]
    benchmarks: Option<Vec<String>>,

    /// Comma-separated list of repositories to benchmark.
    ///
    /// Each entry has the form `org/repo[:rev][ <extra args...>]`. Anything
    /// after the first whitespace is appended to every benchmark command for
    /// that repo (handy to skip a broken test contract via e.g.
    /// `--nmc BrokenTest`).
    ///
    /// Examples:
    ///   `ithacaxyz/account:v0.5.7`
    ///   `vectorized/solady:v0.1.26 --nmc 'LifebuoyTest|LibBitTest'`
    ///
    /// Perf mode only — fails if combined with --fuzz-campaigns (use that
    /// flag's per-spec repo field instead).
    #[clap(long, value_delimiter = ',', conflicts_with = "fuzz_campaigns")]
    repos: Option<Vec<String>>,

    /// Comma-separated list of fuzz campaigns to benchmark.
    ///
    /// Each entry has the form `org/repo[:rev];Contract;invariant_test[ <extra>]`.
    /// When this flag is set, the runner enters fuzz-campaign mode. It is
    /// mutually exclusive with the perf-mode flags `--benchmarks`, `--repos`
    /// and `--json-output`. Multiple invariants in the same repo are
    /// supported; the repo is cloned once and each invariant gets its own
    /// campaign per Foundry version.
    ///
    /// Example:
    ///   `Recon-Fuzz/aave-v4-scfuzzbench:v0.5.6-recon;CryticToFoundry;invariant_noop`
    #[clap(long, value_delimiter = ',')]
    fuzz_campaigns: Option<Vec<String>>,

    /// Per-campaign invariant timeout in seconds (fuzz-campaign mode only).
    #[clap(long, default_value_t = fuzz::FUZZ_TIMEOUT_SECS)]
    fuzz_timeout: u64,
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

    let fuzz_campaign_mode = cli.fuzz_campaigns.is_some();

    // Determine versions to test. Fuzz-campaign mode defaults to local+stable
    // (compare the PR branch against the current stable release). Perf mode
    // defaults to the global FOUNDRY_VERSIONS constant (stable + nightly).
    let versions = if let Some(v) = cli.versions.clone() {
        v
    } else if fuzz_campaign_mode {
        vec!["local".to_string(), "stable".to_string()]
    } else {
        FOUNDRY_VERSIONS.iter().map(|&s| s.to_string()).collect()
    };

    // Fuzz campaign mode runs an entirely separate code path: no hyperfine,
    // no perf flags, no markdown table of milliseconds.
    if let Some(specs) = cli.fuzz_campaigns.clone() {
        return run_fuzz_mode(&cli, &versions, &specs, cli.fuzz_timeout);
    }

    // Check if hyperfine is installed (perf mode only)
    let hyperfine_check = Command::new("hyperfine").arg("--version").output();
    if hyperfine_check.is_err() || !hyperfine_check.unwrap().status.success() {
        eyre::bail!(
            "hyperfine is not installed. Please install it first: https://github.com/sharkdp/hyperfine"
        );
    }

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

    // Write Markdown report unless --json-output is set without an explicit --output-file.
    let md_filename = match cli.output_file {
        Some(f) => Some(f),
        None if cli.json_output.is_none() => Some("LATEST.md".to_string()),
        None => None,
    };
    if let Some(filename) = md_filename {
        sh_println!("📝 Generating report...");
        let markdown = results.generate_markdown(&versions, &repos);
        let output_path = cli.output_dir.join(filename);
        fs::write(&output_path, markdown).wrap_err("Failed to write output file")?;
        sh_println!("✅ Report written to: {}", output_path.display());
    }

    if let Some(json_filename) = cli.json_output {
        let summary = results.generate_json_summary(&versions);
        let json =
            serde_json::to_string_pretty(&summary).wrap_err("Failed to serialize JSON summary")?;
        let json_path = cli.output_dir.join(json_filename);
        fs::write(&json_path, json).wrap_err("Failed to write JSON summary")?;
        sh_println!("✅ JSON summary written to: {}", json_path.display());
    }

    Ok(())
}

/// Drive the fuzz-campaign benchmark mode end-to-end.
///
/// Specs are grouped by `(org, repo, rev)` so each repo is cloned once and
/// then each invariant is run sequentially per Foundry version. We avoid
/// re-cloning between versions by reusing the same working tree (each campaign
/// already `forge clean`s its own cache and `corpus/`).
#[allow(unused_must_use)]
fn run_fuzz_mode(
    cli: &Cli,
    versions: &[String],
    specs: &[String],
    timeout_secs: u64,
) -> Result<()> {
    let campaigns: Vec<FuzzCampaignSpec> = specs
        .iter()
        .map(|s| s.parse::<FuzzCampaignSpec>())
        .collect::<Result<Vec<_>>>()
        .wrap_err("Failed to parse --fuzz-campaigns")?;

    sh_println!("🚀 Foundry Fuzz Campaign Runner");
    sh_println!("Running with versions: {}", versions.join(", "));
    sh_println!(
        "Running campaigns: {}",
        campaigns
            .iter()
            .map(|c| format!("{}/{} :: {}::{}", c.repo.org, c.repo.repo, c.contract, c.test))
            .collect::<Vec<_>>()
            .join(", ")
    );

    if cli.force_install {
        install_foundry_versions(versions)?;
    }

    // Group campaigns by repo so we clone each repo only once.
    let mut by_repo: BTreeMap<String, (RepoConfig, Vec<FuzzCampaignSpec>)> = BTreeMap::new();
    for spec in &campaigns {
        let key = format!("{}/{}:{}", spec.repo.org, spec.repo.repo, spec.repo.rev);
        by_repo.entry(key).or_insert_with(|| (spec.repo.clone(), Vec::new())).1.push(spec.clone());
    }

    // Materialise each repo once in a deterministic per-repo tempdir.
    let temp_root = std::env::temp_dir().join("foundry-bench-fuzz");
    std::fs::create_dir_all(&temp_root)?;

    let mut roots: BTreeMap<String, PathBuf> = BTreeMap::new();
    for (key, (repo_cfg, _)) in &by_repo {
        let safe = key.replace('/', "_").replace(':', "@");
        let project_root = temp_root.join(safe);
        sh_println!("📦 Setting up fuzz project {key}");
        fuzz::setup_fuzz_project(repo_cfg, &project_root)
            .wrap_err_with(|| format!("Failed to setup fuzz project {key}"))?;
        roots.insert(key.clone(), project_root);
    }
    sh_println!("✅ All fuzz projects ready");

    let mut results = BenchmarkResults::new();
    if let Some(first) = versions.first() {
        results.set_baseline_version(first.clone());
    }

    for version in versions {
        sh_println!("🔧 Switching to Foundry version: {version}");
        switch_version_safe(version)?;

        let current = get_forge_version()?;
        sh_println!("Current version: {}", current.trim());

        let version_details = get_forge_version_details()?;
        results.add_version_details(version, version_details);

        for (key, (_repo_cfg, specs_in_repo)) in &by_repo {
            let project_root = roots.get(key).expect("project root registered above");
            for spec in specs_in_repo {
                let label = format!("{}/{} / {}", spec.repo.org, spec.repo.repo, spec.test);
                sh_println!("▶ [{version}] {label}");
                let result =
                    fuzz::run_campaign(project_root, spec, version, timeout_secs, cli.verbose)
                        .wrap_err_with(|| format!("Fuzz campaign {label} failed for {version}"))?;
                sh_println!(
                    "  ✔ runs={} calls={} reverts={} assertion_bugs={}",
                    result.runs,
                    result.calls,
                    result.reverts,
                    result.assertion_bugs,
                );
                results.add_fuzz_result(&label, version, result);
            }
        }
    }

    let filename =
        cli.output_file.clone().unwrap_or_else(|| "forge_fuzz_campaign_bench.md".to_string());
    let markdown = results.generate_fuzz_markdown(versions, timeout_secs);
    let output_path = cli.output_dir.join(filename);
    fs::write(&output_path, markdown).wrap_err("Failed to write fuzz markdown")?;
    sh_println!("✅ Fuzz report written to: {}", output_path.display());

    Ok(())
}

#[allow(unused_must_use)]
fn install_foundry_versions(versions: &[String]) -> Result<()> {
    sh_println!("Installing Foundry versions...");

    for version in versions {
        sh_println!("Installing {version}...");

        if version == "local" {
            install_local_version()?;
            continue;
        }

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
