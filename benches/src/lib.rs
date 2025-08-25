use crate::results::{HyperfineOutput, HyperfineResult};
use eyre::{Result, WrapErr};
use foundry_common::{sh_eprintln, sh_println};
use foundry_compilers::project_util::TempProject;
use foundry_test_utils::util::clone_remote;
use once_cell::sync::Lazy;
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use std::{
    env,
    path::{Path, PathBuf},
    process::Command,
    str::FromStr,
};

pub mod results;

/// Default number of runs for benchmarks
pub const RUNS: u32 = 5;

/// Configuration for repositories to benchmark
#[derive(Debug, Clone)]
pub struct RepoConfig {
    pub name: String,
    pub org: String,
    pub repo: String,
    pub rev: String,
}

impl FromStr for RepoConfig {
    type Err = eyre::Error;

    fn from_str(spec: &str) -> Result<Self> {
        // Split by ':' first to separate repo path from optional rev
        let parts: Vec<&str> = spec.splitn(2, ':').collect();
        let repo_path = parts[0];
        let custom_rev = parts.get(1).copied();

        // Now split the repo path by '/'
        let path_parts: Vec<&str> = repo_path.split('/').collect();
        if path_parts.len() != 2 {
            eyre::bail!("Invalid repo format '{}'. Expected 'org/repo' or 'org/repo:rev'", spec);
        }

        let org = path_parts[0];
        let repo = path_parts[1];

        // Try to find this repo in BENCHMARK_REPOS to get the full config
        let existing_config = BENCHMARK_REPOS.iter().find(|r| r.org == org && r.repo == repo);

        let config = if let Some(existing) = existing_config {
            // Use existing config but allow custom rev to override
            let mut config = existing.clone();
            if let Some(rev) = custom_rev {
                config.rev = rev.to_string();
            }
            config
        } else {
            // Create new config with custom rev or default
            // Name should follow the format: org-repo (with hyphen)
            RepoConfig {
                name: format!("{org}-{repo}"),
                org: org.to_string(),
                repo: repo.to_string(),
                rev: custom_rev.unwrap_or("main").to_string(),
            }
        };

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
        },
        RepoConfig {
            name: "solady".to_string(),
            org: "Vectorized".to_string(),
            repo: "solady".to_string(),
            rev: "main".to_string(),
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
        clone_remote(&repo_url, root);

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
        Ok(BenchmarkProject { name: config.name.to_string(), root_path, temp_project })
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

            if !status.success() {
                sh_println!(
                    "  ⚠️  Warning: npm install failed with exit code: {:?}",
                    status.code()
                );
            } else {
                sh_println!("  ✅ npm install completed successfully");
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

    /// Benchmark forge test
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
            "forge test",
            runs,
            Some("forge build"),
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
            "FOUNDRY_LINT_LINT_ON_BUILD=false forge build",
            runs,
            None,
            Some("forge build"),
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
            "FOUNDRY_LINT_LINT_ON_BUILD=false forge build",
            runs,
            Some("forge clean"),
            None,
            Some("forge clean"),
            verbose,
        )
    }

    /// Benchmark forge fuzz tests
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
            r#"forge test --match-test "test[^(]*\([^)]+\)""#,
            runs,
            Some("forge build"),
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
            "forge coverage --ir-minimum",
            runs,
            None,
            None,
            None,
            verbose,
        )
    }

    /// Benchmark forge test with --isolate flag
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
            "forge test --isolate",
            runs,
            Some("forge build"),
            None,
            None,
            verbose,
        )
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
            _ => eyre::bail!("Unknown benchmark: {}", benchmark),
        }
    }
}

/// Switch to a specific foundry version
#[allow(unused_must_use)]
pub fn switch_foundry_version(version: &str) -> Result<()> {
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

/// Get Foundry versions to benchmark from environment variable or default
///
/// Reads from FOUNDRY_BENCH_VERSIONS environment variable if set,
/// otherwise returns the default versions from FOUNDRY_VERSIONS constant.
///
/// The environment variable should be a comma-separated list of versions,
/// e.g., "stable,nightly,v1.2.0"
pub fn get_benchmark_versions() -> Vec<String> {
    if let Ok(versions_env) = env::var("FOUNDRY_BENCH_VERSIONS") {
        versions_env.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
    } else {
        FOUNDRY_VERSIONS.iter().map(|&s| s.to_string()).collect()
    }
}

/// Setup Repositories for benchmarking
pub fn setup_benchmark_repos() -> Vec<(RepoConfig, BenchmarkProject)> {
    // Check for FOUNDRY_BENCH_REPOS environment variable
    let repos = if let Ok(repos_env) = env::var("FOUNDRY_BENCH_REPOS") {
        // Parse repo specs from the environment variable
        // Format should be: "org1/repo1,org2/repo2"
        repos_env
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.parse::<RepoConfig>())
            .collect::<Result<Vec<_>>>()
            .expect("Failed to parse FOUNDRY_BENCH_REPOS")
    } else {
        BENCHMARK_REPOS.clone()
    };

    repos
        .par_iter()
        .map(|repo_config| {
            let project = BenchmarkProject::setup(repo_config).expect("Failed to setup project");
            (repo_config.clone(), project)
        })
        .collect()
}
