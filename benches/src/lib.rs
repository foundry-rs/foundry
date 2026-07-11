//! Foundry benchmark runner.

use crate::{
    results::{HyperfineOutput, HyperfineResult},
    symbolic::{Fixture, Overlay, Sample, Sidecar},
};
use eyre::{Result, WrapErr};
use foundry_common::{sh_eprintln, sh_println};
use foundry_compilers::project_util::TempProject;
use foundry_test_utils::util::clone_remote;
use once_cell::sync::Lazy;
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    str::FromStr,
    time::Instant,
};

pub mod results;
pub mod symbolic;

/// Default number of runs for benchmarks
pub const RUNS: u32 = 5;

/// Configuration for repositories to benchmark
#[derive(Debug, Clone)]
pub struct RepoConfig {
    pub name: String,
    pub org: String,
    pub repo: String,
    pub rev: String,
    /// Optional extra arguments appended to every benchmark command for this
    /// repo (e.g. `--nmc BrokenTest` to skip a broken test contract).
    pub extra_args: Option<String>,
}

impl FromStr for RepoConfig {
    type Err = eyre::Error;

    /// Parse a repo spec of the form `org/repo[:rev][ <extra args...>]`.
    ///
    /// Anything after the first whitespace is treated as extra arguments
    /// appended to every benchmark command for this repo.
    fn from_str(spec: &str) -> Result<Self> {
        let spec = spec.trim();
        // Anything after the first whitespace is per-repo extra args.
        let (head, extra_args) = match spec.split_once(char::is_whitespace) {
            Some((head, rest)) => (head, Some(rest.trim().to_string())),
            None => (spec, None),
        };

        let (repo_path, custom_rev) = match head.split_once(':') {
            Some((path, rev)) => (path, Some(rev)),
            None => (head, None),
        };

        let (org, repo) = repo_path.split_once('/').ok_or_else(|| {
            eyre::eyre!("Invalid repo format '{spec}'. Expected 'org/repo' or 'org/repo:rev'")
        })?;

        // Inherit defaults from BENCHMARK_REPOS when available, otherwise build
        // a fresh config. Custom rev / extra args always override.
        let mut config = BENCHMARK_REPOS
            .iter()
            .find(|r| r.org == org && r.repo == repo)
            .cloned()
            .unwrap_or_else(|| Self {
                name: format!("{org}-{repo}"),
                org: org.to_string(),
                repo: repo.to_string(),
                rev: "main".to_string(),
                extra_args: None,
            });

        if let Some(rev) = custom_rev {
            config.rev = rev.to_string();
        }
        config.extra_args = extra_args;

        let _ = sh_println!("Parsed repo spec '{spec}' -> {config:?}");
        Ok(config)
    }
}

/// Available repositories for benchmarking
pub fn default_benchmark_repos() -> Vec<RepoConfig> {
    vec![
        RepoConfig {
            name: "ithacaxyz-account".to_string(),
            org: "ithacaxyz".to_string(),
            repo: "account".to_string(),
            rev: "main".to_string(),
            extra_args: None,
        },
        RepoConfig {
            name: "solady".to_string(),
            org: "Vectorized".to_string(),
            repo: "solady".to_string(),
            rev: "main".to_string(),
            extra_args: None,
        },
    ]
}

// Keep a lazy static for compatibility
pub static BENCHMARK_REPOS: Lazy<Vec<RepoConfig>> = Lazy::new(default_benchmark_repos);

/// Foundry versions to benchmark
///
/// To add more versions for comparison, install them first:
/// ```bash
/// foundryup --install stable
/// foundryup --install nightly
/// foundryup --install v0.2.0  # Example specific version
/// ```
///
/// Then add the version strings to this array. Supported formats:
/// - "stable" - Latest stable release
/// - "nightly" - Latest nightly build
/// - "v0.2.0" - Specific version tag
/// - "commit-hash" - Specific commit hash
/// - "nightly-rev" - Nightly build with specific revision
pub static FOUNDRY_VERSIONS: &[&str] = &["stable", "nightly"];

/// A benchmark project that represents a cloned repository ready for testing
pub struct BenchmarkProject {
    pub name: String,
    pub temp_project: TempProject,
    pub root_path: PathBuf,
    /// Optional extra arguments appended to every benchmark command.
    pub extra_args: Option<String>,
    pub org: String,
    pub repo: String,
    pub revision: String,
}

impl BenchmarkProject {
    /// Set up a benchmark project by cloning the repository
    #[allow(unused_must_use)]
    pub fn setup(config: &RepoConfig) -> Result<Self> {
        let temp_project =
            TempProject::dapptools().wrap_err("Failed to create temporary project")?;

        // Get root path before clearing
        let root_path = temp_project.root().to_path_buf();
        let root = root_path.to_str().unwrap();

        // Remove all files in the directory
        for entry in std::fs::read_dir(&root_path)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                std::fs::remove_dir_all(&path).ok();
            } else {
                std::fs::remove_file(&path).ok();
            }
        }

        // Clone the repository
        let repo_url = format!("https://github.com/{}/{}.git", config.org, config.repo);
        clone_remote(&repo_url, root, true);

        // Checkout specific revision if provided
        if !config.rev.is_empty() && config.rev != "main" && config.rev != "master" {
            let status = Command::new("git")
                .current_dir(root)
                .args(["checkout", &config.rev])
                .status()
                .wrap_err("Failed to checkout revision")?;

            if !status.success() {
                eyre::bail!("Git checkout failed for {}", config.name);
            }
        }

        // Git submodules are already cloned via --recursive flag
        // But npm dependencies still need to be installed
        Self::install_npm_dependencies(&root_path)?;

        sh_println!("  ✅ Project {} setup complete at {}", config.name, root);
        let revision = String::from_utf8(
            Command::new("git")
                .current_dir(&root_path)
                .args(["rev-parse", "HEAD"])
                .output()?
                .stdout,
        )?
        .trim()
        .to_string();
        Ok(Self {
            name: config.name.clone(),
            root_path,
            temp_project,
            extra_args: config.extra_args.clone(),
            org: config.org.clone(),
            repo: config.repo.clone(),
            revision,
        })
    }

    /// Append `self.extra_args` to a benchmark shell command, if any.
    fn cmd(&self, base: &str) -> String {
        match self.extra_args.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(extra) => format!("{base} {extra}"),
            None => base.to_string(),
        }
    }

    /// Install npm dependencies if package.json exists
    #[allow(unused_must_use)]
    fn install_npm_dependencies(root: &Path) -> Result<()> {
        if root.join("package.json").exists() {
            sh_println!("  📦 Running npm install...");
            let status = Command::new("npm")
                .current_dir(root)
                .args(["install"])
                .stdout(std::process::Stdio::inherit())
                .stderr(std::process::Stdio::inherit())
                .status()
                .wrap_err("Failed to run npm install")?;

            if status.success() {
                sh_println!("  ✅ npm install completed successfully");
            } else {
                sh_println!(
                    "  ⚠️  Warning: npm install failed with exit code: {:?}",
                    status.code()
                );
            }
        }
        Ok(())
    }

    /// Run a command with hyperfine and return the results
    ///
    /// # Arguments
    /// * `benchmark_name` - Name of the benchmark for organizing output
    /// * `version` - Foundry version being benchmarked
    /// * `command` - The command to benchmark
    /// * `runs` - Number of runs to perform
    /// * `setup` - Optional setup command to run before the benchmark series (e.g., "forge build")
    /// * `prepare` - Optional prepare command to run before each timing run (e.g., "forge clean")
    /// * `conclude` - Optional conclude command to run after each timing run (e.g., cleanup)
    /// * `verbose` - Whether to show command output
    ///
    /// # Hyperfine flags used:
    /// * `--runs` - Number of timing runs
    /// * `--setup` - Execute before the benchmark series (not before each run)
    /// * `--prepare` - Execute before each timing run
    /// * `--conclude` - Execute after each timing run
    /// * `--export-json` - Export results to JSON for parsing
    /// * `--shell=bash` - Use bash for shell command execution
    /// * `--show-output` - Show command output (when verbose)
    #[allow(clippy::too_many_arguments)]
    fn hyperfine(
        &self,
        benchmark_name: &str,
        version: &str,
        command: &str,
        runs: u32,
        setup: Option<&str>,
        prepare: Option<&str>,
        conclude: Option<&str>,
        verbose: bool,
    ) -> Result<HyperfineResult> {
        // Create structured temp directory for JSON output
        // Format: <temp_dir>/<benchmark_name>/<version>/<repo_name>/<benchmark_name>.json
        let temp_dir = std::env::temp_dir();
        let json_dir =
            temp_dir.join("foundry-bench").join(benchmark_name).join(version).join(&self.name);
        std::fs::create_dir_all(&json_dir)?;

        let json_path = json_dir.join(format!("{benchmark_name}.json"));

        // Build hyperfine command
        let mut hyperfine_cmd = Command::new("hyperfine");
        hyperfine_cmd
            .current_dir(&self.root_path)
            .arg("--runs")
            .arg(runs.to_string())
            .arg("--export-json")
            .arg(&json_path)
            .arg("--shell=bash");

        // Add optional setup command
        if let Some(setup_cmd) = setup {
            hyperfine_cmd.arg("--setup").arg(setup_cmd);
        }

        // Add optional prepare command
        if let Some(prepare_cmd) = prepare {
            hyperfine_cmd.arg("--prepare").arg(prepare_cmd);
        }

        // Add optional conclude command
        if let Some(conclude_cmd) = conclude {
            hyperfine_cmd.arg("--conclude").arg(conclude_cmd);
        }

        if verbose {
            hyperfine_cmd.arg("--show-output");
            hyperfine_cmd.stderr(std::process::Stdio::inherit());
            hyperfine_cmd.stdout(std::process::Stdio::inherit());
        }

        // Add the benchmark command last
        hyperfine_cmd.arg(command);

        let status = hyperfine_cmd.status().wrap_err("Failed to run hyperfine")?;
        if !status.success() {
            eyre::bail!("Hyperfine failed for command: {}", command);
        }

        // Read and parse the JSON output
        let json_content = std::fs::read_to_string(json_path)?;
        let output: HyperfineOutput = serde_json::from_str(&json_content)?;

        // Extract the first result (we only run one command at a time)
        output.results.into_iter().next().ok_or_else(|| eyre::eyre!("No results from hyperfine"))
    }

    /// Benchmark forge test without isolation.
    pub fn bench_forge_test(
        &self,
        version: &str,
        runs: u32,
        verbose: bool,
    ) -> Result<HyperfineResult> {
        // Build before running tests
        self.hyperfine(
            "forge_test",
            version,
            &self.cmd("FOUNDRY_DYNAMIC_TEST_LINKING=false FOUNDRY_ISOLATE=false forge test"),
            runs,
            Some("FOUNDRY_DYNAMIC_TEST_LINKING=false FOUNDRY_ISOLATE=false forge build"),
            None,
            None,
            verbose,
        )
    }

    /// Benchmark forge build with cache
    pub fn bench_forge_build_with_cache(
        &self,
        version: &str,
        runs: u32,
        verbose: bool,
    ) -> Result<HyperfineResult> {
        self.hyperfine(
            "forge_build_with_cache",
            version,
            &self.cmd(
                "FOUNDRY_DYNAMIC_TEST_LINKING=false FOUNDRY_LINT_LINT_ON_BUILD=false FOUNDRY_ISOLATE=false forge build",
            ),
            runs,
            None,
            Some("FOUNDRY_DYNAMIC_TEST_LINKING=false FOUNDRY_ISOLATE=false forge build"),
            None,
            verbose,
        )
    }

    /// Benchmark forge build without cache
    pub fn bench_forge_build_no_cache(
        &self,
        version: &str,
        runs: u32,
        verbose: bool,
    ) -> Result<HyperfineResult> {
        // Clean before each timing run
        self.hyperfine(
            "forge_build_no_cache",
            version,
            &self.cmd(
                "FOUNDRY_DYNAMIC_TEST_LINKING=false FOUNDRY_LINT_LINT_ON_BUILD=false FOUNDRY_ISOLATE=false forge build",
            ),
            runs,
            Some("FOUNDRY_DYNAMIC_TEST_LINKING=false FOUNDRY_ISOLATE=false forge clean"),
            None,
            Some("FOUNDRY_DYNAMIC_TEST_LINKING=false FOUNDRY_ISOLATE=false forge clean"),
            verbose,
        )
    }

    /// Benchmark forge fuzz tests without isolation.
    pub fn bench_forge_fuzz_test(
        &self,
        version: &str,
        runs: u32,
        verbose: bool,
    ) -> Result<HyperfineResult> {
        // Build before running fuzz tests
        self.hyperfine(
            "forge_fuzz_test",
            version,
            &self.cmd(
                r#"FOUNDRY_DYNAMIC_TEST_LINKING=false FOUNDRY_ISOLATE=false forge test --match-test "test[^(]*\([^)]+\)""#,
            ),
            runs,
            Some("FOUNDRY_DYNAMIC_TEST_LINKING=false FOUNDRY_ISOLATE=false forge build"),
            None,
            None,
            verbose,
        )
    }

    /// Benchmark forge coverage
    pub fn bench_forge_coverage(
        &self,
        version: &str,
        runs: u32,
        verbose: bool,
    ) -> Result<HyperfineResult> {
        // No setup needed, forge coverage builds internally
        // Use --ir-minimum to avoid "Stack too deep" errors
        self.hyperfine(
            "forge_coverage",
            version,
            &self.cmd(
                "FOUNDRY_DYNAMIC_TEST_LINKING=false FOUNDRY_ISOLATE=false forge coverage --ir-minimum",
            ),
            runs,
            None,
            None,
            None,
            verbose,
        )
    }

    /// Benchmark forge test with isolate mode
    pub fn bench_forge_isolate_test(
        &self,
        version: &str,
        runs: u32,
        verbose: bool,
    ) -> Result<HyperfineResult> {
        // Build before running tests
        self.hyperfine(
            "forge_isolate_test",
            version,
            &self.cmd("FOUNDRY_DYNAMIC_TEST_LINKING=false FOUNDRY_ISOLATE=true forge test"),
            runs,
            Some("FOUNDRY_DYNAMIC_TEST_LINKING=false FOUNDRY_ISOLATE=true forge build"),
            None,
            None,
            verbose,
        )
    }

    /// Benchmark focused symbolic checks and collect symbolic solver counters.
    pub fn bench_forge_symbolic_test(
        &self,
        _version: &str,
        runs: u32,
        verbose: bool,
    ) -> Result<HyperfineResult> {
        let fixture = Fixture::identify(&self.org, &self.repo);
        let command = self.cmd(&fixture.test_command());
        let build_command = fixture.build_command();
        let overlay = Overlay::install(&self.root_path, fixture)?;
        let benchmark = (|| -> Result<HyperfineResult> {
            let status = Command::new("bash")
                .current_dir(&self.root_path)
                .args(["-lc", &build_command])
                .status()
                .wrap_err("Failed to build project before symbolic benchmark")?;
            if !status.success() {
                eyre::bail!(
                    "forge build failed before symbolic benchmark with command: {}",
                    build_command
                );
            }

            let mut times = Vec::with_capacity(runs as usize);
            let mut samples = Vec::with_capacity(runs as usize);
            let mut exit_codes = Vec::with_capacity(runs as usize);

            for _ in 0..runs {
                let started = Instant::now();
                let output = Command::new("bash")
                    .current_dir(&self.root_path)
                    .args(["-lc", &command])
                    .output()
                    .wrap_err("Failed to run forge symbolic benchmark")?;
                let elapsed = started.elapsed().as_secs_f64();
                let exit_code = output.status.code().unwrap_or(-1);
                if !matches!(exit_code, 0 | 1) {
                    let _ = sh_eprintln!("{}", String::from_utf8_lossy(&output.stderr));
                    eyre::bail!(
                        "forge symbolic benchmark exited abnormally with code {exit_code}: {command}"
                    );
                }
                times.push(elapsed);
                exit_codes.push(exit_code);

                if verbose {
                    let _ = sh_println!("{}", String::from_utf8_lossy(&output.stderr));
                }

                let run = match symbolic::parse(&output.stdout) {
                    Ok(summary) => summary,
                    Err(err) => {
                        if !output.status.success() {
                            let _ = sh_eprintln!("{}", String::from_utf8_lossy(&output.stderr));
                            eyre::bail!(
                                "forge symbolic benchmark failed with command: {command}; {err}"
                            );
                        }
                        return Err(err);
                    }
                };
                if !output.status.success() && verbose {
                    let _ = sh_eprintln!("{}", String::from_utf8_lossy(&output.stderr));
                }
                samples.push(Sample { wall_time_seconds: elapsed, exit_code, run });
            }

            let symbolic = samples
                .get(median_index(&times))
                .map(|sample| symbolic::compatibility(&sample.run))
                .ok_or_else(|| eyre::eyre!("symbolic benchmark produced no runs"))?;
            let sidecar = Sidecar::new(
                fixture,
                &format!("{}/{}", self.org, self.repo),
                &self.revision,
                &build_command,
                &command,
                samples,
            );

            Ok(HyperfineResult {
                command,
                mean: mean(&times),
                stddev: stddev(&times),
                median: median(&times),
                user: 0.0,
                system: 0.0,
                min: times.iter().copied().reduce(f64::min).unwrap_or_default(),
                max: times.iter().copied().reduce(f64::max).unwrap_or_default(),
                times,
                exit_codes: Some(exit_codes),
                parameters: None,
                symbolic: Some(symbolic),
                symbolic_sidecar: Some(sidecar),
            })
        })();
        let cleanup = overlay.finish();
        match (benchmark, cleanup) {
            (Ok(result), Ok(())) => Ok(result),
            (Err(err), Ok(())) => Err(err),
            (Ok(_), Err(cleanup_err)) => Err(cleanup_err),
            (Err(err), Err(cleanup_err)) => Err(eyre::eyre!(
                "{err}; additionally, symbolic fixture cleanup failed: {cleanup_err}"
            )),
        }
    }

    /// Get the root path of the project
    pub fn root(&self) -> &Path {
        &self.root_path
    }

    /// Run a specific benchmark by name
    pub fn run(
        &self,
        benchmark: &str,
        version: &str,
        runs: u32,
        verbose: bool,
    ) -> Result<HyperfineResult> {
        match benchmark {
            "forge_test" => self.bench_forge_test(version, runs, verbose),
            "forge_build_no_cache" => self.bench_forge_build_no_cache(version, runs, verbose),
            "forge_build_with_cache" => self.bench_forge_build_with_cache(version, runs, verbose),
            "forge_fuzz_test" => self.bench_forge_fuzz_test(version, runs, verbose),
            "forge_coverage" => self.bench_forge_coverage(version, runs, verbose),
            "forge_isolate_test" => self.bench_forge_isolate_test(version, runs, verbose),
            "forge_symbolic_test" => self.bench_forge_symbolic_test(version, runs, verbose),
            _ => eyre::bail!("Unknown benchmark: {}", benchmark),
        }
    }
}

fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().sum::<f64>() / values.len() as f64
}

fn median(values: &[f64]) -> f64 {
    values.get(median_index(values)).copied().unwrap_or_default()
}

fn median_index(values: &[f64]) -> usize {
    let mut indices = (0..values.len()).collect::<Vec<_>>();
    indices.sort_by(|&left, &right| values[left].total_cmp(&values[right]));
    indices.get(indices.len() / 2).copied().unwrap_or_default()
}

fn stddev(values: &[f64]) -> Option<f64> {
    if values.len() < 2 {
        return None;
    }
    let mean = mean(values);
    let variance =
        values.iter().map(|value| (value - mean).powi(2)).sum::<f64>() / values.len() as f64;
    Some(variance.sqrt())
}

/// The workspace root, embedded at compile time.
/// `benches/` is one level below the workspace root.
const WORKSPACE_ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/..");
const WORKSPACE_ROOT_ENV: &str = "FOUNDRY_BENCH_WORKSPACE_ROOT";
const LOCAL_BUILD_PROFILE_ENV: &str = "FOUNDRY_BENCH_LOCAL_BUILD_PROFILE";
const LOCAL_BUILD_BINS_ENV: &str = "FOUNDRY_BENCH_LOCAL_BUILD_BINS";
const DEFAULT_LOCAL_BUILD_PROFILE: &str = "dist";
const FOUNDRY_BINS: [&str; 4] = ["forge", "cast", "anvil", "chisel"];

/// Parse a `--versions` entry into a display name and an optional source
/// workspace. `name=path` builds Foundry from `path` (a checked-out ref) and
/// labels it `name`; a bare entry uses foundryup, or the default workspace for
/// `local`.
pub fn parse_version_spec(spec: &str) -> (String, Option<PathBuf>) {
    match spec.split_once('=') {
        Some((name, path)) if !name.is_empty() && !path.is_empty() => {
            (name.to_string(), Some(PathBuf::from(path)))
        }
        _ => (spec.to_string(), None),
    }
}

/// Switch to a specific foundry version.
///
/// The special keyword `local` builds and activates the current workspace.
#[allow(unused_must_use)]
pub fn switch_foundry_version(version: &str) -> Result<()> {
    if version == "local" {
        return install_local_workspace(&workspace_root()?);
    }

    let output = Command::new("foundryup")
        .args(["--use", version])
        .output()
        .wrap_err("Failed to run foundryup")?;

    // Check if the error is about forge --version failing
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("command failed") && stderr.contains("forge --version") {
        eyre::bail!(
            "Foundry binaries maybe corrupted. Please reinstall by running `foundryup --install <version>`"
        );
    }

    if !output.status.success() {
        sh_eprintln!("foundryup stderr: {stderr}");
        eyre::bail!("Failed to switch to foundry version: {}", version);
    }

    sh_println!("  Successfully switched to version: {version}");
    Ok(())
}

/// Build and activate the shipped Foundry binaries from an explicit workspace,
/// without linking unused workspace binaries. Used to benchmark a baseline ref
/// checked out into a separate worktree.
#[allow(unused_must_use)]
pub fn install_local_workspace(workspace: &Path) -> Result<()> {
    let profile = local_build_profile();
    let bins = local_build_bins()?;
    sh_println!(
        "  Building local workspace at {} with {} profile for {}",
        workspace.display(),
        profile.to_string_lossy(),
        bins.join(", ")
    );

    let mut cmd = Command::new("cargo");
    cmd.current_dir(workspace).args(["build", "--locked", "--profile"]).arg(&profile);
    for bin in &bins {
        cmd.args(["--bin", bin]);
    }

    let status = cmd.status().wrap_err("Failed to build local Foundry workspace")?;

    if !status.success() {
        eyre::bail!("local Foundry build failed");
    }

    activate_local_binaries(workspace, &profile, &bins)?;
    sh_println!("  Successfully activated local {} build", profile.to_string_lossy());
    Ok(())
}

fn workspace_root() -> Result<PathBuf> {
    let workspace = env::var_os(WORKSPACE_ROOT_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(WORKSPACE_ROOT));
    std::fs::canonicalize(&workspace)
        .wrap_err_with(|| format!("Failed to resolve workspace root {}", workspace.display()))
}

fn local_build_profile() -> std::ffi::OsString {
    env::var_os(LOCAL_BUILD_PROFILE_ENV)
        .filter(|profile| !profile.is_empty())
        .unwrap_or_else(|| DEFAULT_LOCAL_BUILD_PROFILE.into())
}

fn local_build_bins() -> Result<Vec<String>> {
    let Some(raw_bins) = env::var_os(LOCAL_BUILD_BINS_ENV).filter(|bins| !bins.is_empty()) else {
        return Ok(FOUNDRY_BINS.into_iter().map(String::from).collect());
    };

    let bins = raw_bins
        .to_string_lossy()
        .split(|c: char| c == ',' || c.is_ascii_whitespace())
        .filter(|bin| !bin.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();

    if bins.is_empty() {
        eyre::bail!("{LOCAL_BUILD_BINS_ENV} did not contain any binary names");
    }

    Ok(bins)
}

fn activate_local_binaries(
    workspace: &Path,
    profile: &std::ffi::OsStr,
    bins: &[String],
) -> Result<()> {
    let bin_dir = foundry_bin_dir()?;
    fs::create_dir_all(&bin_dir).wrap_err_with(|| {
        format!("Failed to create Foundry bin directory at {}", bin_dir.display())
    })?;

    let local_bin_dir = workspace.join("target").join(profile);
    for bin in bins {
        let bin_name = format!("{bin}{}", env::consts::EXE_SUFFIX);
        let source = local_bin_dir.join(&bin_name);
        let destination = bin_dir.join(&bin_name);

        if !source.exists() {
            eyre::bail!("local Foundry binary not found at {}", source.display());
        }

        if fs::symlink_metadata(&destination).is_ok() {
            fs::remove_file(&destination).wrap_err_with(|| {
                format!("Failed to remove existing binary at {}", destination.display())
            })?;
        }

        fs::copy(&source, &destination).wrap_err_with(|| {
            format!("Failed to activate local binary {}", destination.display())
        })?;
    }

    Ok(())
}

fn foundry_bin_dir() -> Result<PathBuf> {
    if let Some(foundry_dir) = env::var_os("FOUNDRY_DIR") {
        return Ok(PathBuf::from(foundry_dir).join("bin"));
    }

    let base_dir = env::var_os("XDG_CONFIG_HOME")
        .or_else(|| env::var_os("HOME"))
        .map(PathBuf::from)
        .ok_or_else(|| eyre::eyre!("Neither FOUNDRY_DIR, XDG_CONFIG_HOME, nor HOME is set"))?;

    Ok(base_dir.join(".foundry").join("bin"))
}

/// Get the current forge version
pub fn get_forge_version() -> Result<String> {
    let output = Command::new("forge")
        .args(["--version"])
        .output()
        .wrap_err("Failed to get forge version")?;

    if !output.status.success() {
        eyre::bail!("forge --version failed");
    }

    let version =
        String::from_utf8(output.stdout).wrap_err("Invalid UTF-8 in forge version output")?;

    Ok(version.lines().next().unwrap_or("unknown").to_string())
}

/// Get the full forge version details including commit hash and date
pub fn get_forge_version_details() -> Result<String> {
    let output = Command::new("forge")
        .args(["--version"])
        .output()
        .wrap_err("Failed to get forge version")?;

    if !output.status.success() {
        eyre::bail!("forge --version failed");
    }

    let full_output =
        String::from_utf8(output.stdout).wrap_err("Invalid UTF-8 in forge version output")?;

    // Extract relevant lines and format them
    let lines: Vec<&str> = full_output.lines().collect();
    if lines.len() >= 3 {
        // Extract version, commit, and timestamp
        let version = lines[0].trim();
        let commit = lines[1].trim().replace("Commit SHA: ", "");
        let timestamp = lines[2].trim().replace("Build Timestamp: ", "");

        // Format as: "forge 1.2.3-nightly (51650ea 2025-06-27)"
        let short_commit = &commit[..7]; // First 7 chars of commit hash
        let date = timestamp.split('T').next().unwrap_or(&timestamp);

        Ok(format!("{version} ({short_commit} {date})"))
    } else {
        // Fallback to just the first line if format is unexpected
        Ok(lines.first().unwrap_or(&"unknown").to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_version_spec() {
        assert_eq!(parse_version_spec("stable"), ("stable".to_string(), None));
        assert_eq!(parse_version_spec("local"), ("local".to_string(), None));
        assert_eq!(
            parse_version_spec("master=../foundry-baseline"),
            ("master".to_string(), Some(PathBuf::from("../foundry-baseline")))
        );
        // A trailing/leading empty side is treated as a plain name, not a source.
        assert_eq!(parse_version_spec("=path"), ("=path".to_string(), None));
    }
}
