use clap::Parser;
use eyre::{Result, WrapErr};
use foundry_bench::{
    BENCHMARK_REPOS, BenchmarkProject, FOUNDRY_VERSIONS, RUNS, RepoConfig, get_forge_version,
    get_forge_version_details, install_local_workspace, parse_version_specs,
    results::{BenchmarkResults, versioned_summary_filename},
    switch_foundry_version,
};
use foundry_common::sh_println;
use rayon::prelude::*;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Mutex,
};

const ALL_BENCHMARKS: [&str; 7] = [
    "forge_test",
    "forge_build_no_cache",
    "forge_build_with_cache",
    "forge_fuzz_test",
    "forge_coverage",
    "forge_isolate_test",
    "forge_symbolic_test",
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

    /// Filename for a JSON summary (benchmark/repo -> wall-time stats and, when
    /// available, symbolic solver counters). Resolved relative to --output-dir.
    /// Consumed by the nightly regression comparison script.
    #[clap(long)]
    json_output: Option<PathBuf>,

    /// Filename for the opt-in versioned symbolic benchmark sidecar.
    #[clap(long)]
    symbolic_sidecar_output: Option<PathBuf>,

    /// Run only specific benchmarks (comma-separated:
    /// forge_test,forge_build_no_cache,forge_build_with_cache,forge_fuzz_test,forge_coverage,
    /// forge_symbolic_test)
    #[clap(long, value_delimiter = ',')]
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
    #[clap(long, value_delimiter = ',')]
    repos: Option<Vec<String>>,
}

/// Mutex to prevent concurrent foundryup calls
static FOUNDRY_LOCK: Mutex<()> = Mutex::new(());

/// Activate a version: build from `source` when the spec was `name=path`,
/// otherwise switch via foundryup (or build the default workspace for `local`).
fn activate_version_safe(name: &str, source: Option<&Path>) -> Result<()> {
    let _lock = FOUNDRY_LOCK.lock().unwrap();
    match source {
        Some(path) => install_local_workspace(path),
        None => switch_foundry_version(name),
    }
}

fn forge_test_supports_symbolic() -> Result<bool> {
    let output = Command::new("forge")
        .args(["test", "--help"])
        .output()
        .wrap_err("Failed to run forge test --help")?;

    if !output.status.success() {
        eyre::bail!("forge test --help failed");
    }

    Ok(String::from_utf8_lossy(&output.stdout).contains("--symbolic")
        || String::from_utf8_lossy(&output.stderr).contains("--symbolic"))
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

    // Determine versions to test. Each entry is a display name, optionally
    // `name=path` to build that ref from source (e.g. a baseline worktree).
    let raw_versions = if let Some(v) = cli.versions {
        v
    } else {
        FOUNDRY_VERSIONS.iter().map(|&s| s.to_string()).collect()
    };
    let version_specs = parse_version_specs(&raw_versions)?;
    let versions: Vec<String> = version_specs.iter().map(|(name, _)| name.clone()).collect();
    if cli.symbolic_sidecar_output.is_some() && versions.len() != 1 {
        eyre::bail!("--symbolic-sidecar-output requires exactly one --versions entry");
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

    // Install versions if requested. Only released versions need pre-installing
    // via foundryup; source-built specs (`name=path`) and `local` are built in
    // the benchmark loop.
    if cli.force_install {
        let installable: Vec<String> = version_specs
            .iter()
            .filter(|(name, source)| source.is_none() && name != "local")
            .map(|(name, _)| name.clone())
            .collect();
        install_foundry_versions(&installable)?;
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
    for (version, source) in &version_specs {
        sh_println!("🔧 Switching to Foundry version: {version}");
        activate_version_safe(version, source.as_deref())?;

        // Verify the switch and capture full version details
        let current = get_forge_version()?;
        sh_println!("Current version: {}", current.trim());

        // Get and store the full version details with commit hash and date
        let version_details = get_forge_version_details()?;
        results.add_version_details(version, version_details);

        let supports_symbolic_test = if benchmarks.iter().any(|b| b == "forge_symbolic_test") {
            forge_test_supports_symbolic()?
        } else {
            false
        };

        sh_println!("Running benchmark tasks for version {version}...");

        // Run all benchmarks sequentially
        let mut version_results = Vec::new();
        for (repo_config, project, benchmark) in &benchmark_tasks {
            if benchmark == "forge_symbolic_test" && !supports_symbolic_test {
                sh_println!(
                    "Skipping forge_symbolic_test on {}/{} for {version}: active forge does not support --symbolic",
                    repo_config.org,
                    repo_config.repo
                );
                continue;
            }

            sh_println!("Running {} on {}/{}", benchmark, repo_config.org, repo_config.repo);

            // Determine runs based on benchmark type
            let runs = match benchmark.as_str() {
                "forge_coverage" => 1, // Coverage runs only once as an exception
                _ => RUNS,             // Use default RUNS constant for all other benchmarks
            };

            // Run the appropriate benchmark
            match project.run(benchmark, version, runs, cli.verbose) {
                Ok(hyperfine_result) => {
                    sh_println!(
                        "  {} on {}/{}: {:.3}s ± {:.3}s",
                        benchmark,
                        repo_config.org,
                        repo_config.repo,
                        hyperfine_result.mean,
                        hyperfine_result.stddev.unwrap_or(0.0)
                    );
                    version_results.push((
                        repo_config.name.clone(),
                        benchmark.clone(),
                        hyperfine_result,
                    ));
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
        }

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
        fs::create_dir_all(&cli.output_dir).wrap_err("Failed to create output directory")?;
        let output_path = cli.output_dir.join(filename);
        fs::write(&output_path, markdown).wrap_err("Failed to write output file")?;
        sh_println!("✅ Report written to: {}", output_path.display());
    }

    if let Some(json_filename) = cli.json_output {
        fs::create_dir_all(&cli.output_dir).wrap_err("Failed to create output directory")?;
        // The summary is keyed by "benchmark/repo", so it holds one version.
        // Write one file per version (suffixed) when several are benchmarked.
        if versions.len() <= 1 {
            let summary = results.generate_json_summary(&versions);
            let json = serde_json::to_string_pretty(&summary)
                .wrap_err("Failed to serialize JSON summary")?;
            let json_path = cli.output_dir.join(&json_filename);
            fs::write(&json_path, json).wrap_err("Failed to write JSON summary")?;
            sh_println!("✅ JSON summary written to: {}", json_path.display());
        } else {
            for version in &versions {
                let summary = results.generate_json_summary(std::slice::from_ref(version));
                let json = serde_json::to_string_pretty(&summary)
                    .wrap_err("Failed to serialize JSON summary")?;
                // Preserve any directory component of --json-output.
                let versioned = json_filename
                    .with_file_name(versioned_summary_filename(&json_filename, version));
                let json_path = cli.output_dir.join(versioned);
                fs::write(&json_path, json).wrap_err("Failed to write JSON summary")?;
                sh_println!("✅ JSON summary written to: {}", json_path.display());
            }
        }
    }

    if let Some(filename) = cli.symbolic_sidecar_output {
        let mut sidecars = std::collections::BTreeMap::new();
        for (benchmark, version_data) in &results.data {
            if benchmark != "forge_symbolic_test" {
                continue;
            }
            for version in &versions {
                if let Some(repo_data) = version_data.get(version) {
                    for (repo, result) in repo_data {
                        if let Some(sidecar) = &result.symbolic_sidecar {
                            sidecars.insert(format!("forge_symbolic_test/{repo}"), sidecar);
                        }
                    }
                }
            }
        }
        if sidecars.is_empty() {
            eyre::bail!("symbolic sidecar requested but no symbolic results were produced");
        }
        let path = cli.output_dir.join(filename);
        fs::create_dir_all(path.parent().unwrap_or(&cli.output_dir))?;
        fs::write(&path, serde_json::to_string_pretty(&sidecars)?)?;
        sh_println!("✅ Symbolic sidecar written to: {}", path.display());
    }

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
