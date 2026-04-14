//! Foundry benchmark runner with branch-vs-branch comparison.
//!
//! Supports benchmarking forge commands (test, fuzz, invariant, fork, build) across two Foundry
//! builds (baseline vs feature), producing structured JSON output with automated regression
//! detection.

use crate::results::{HyperfineOutput, HyperfineResult};
use eyre::{Result, WrapErr};
use foundry_common::{sh_eprintln, sh_println};
use foundry_compilers::project_util::TempProject;
use foundry_test_utils::util::clone_remote;
use once_cell::sync::Lazy;
use std::{
    path::{Path, PathBuf},
    process::Command,
    str::FromStr,
};

pub mod results;

/// Default number of runs per benchmark.
pub const RUNS: u32 = 5;

/// All available benchmark types.
pub const ALL_BENCHMARKS: &[&str] = &[
    "forge_test",
    "forge_fuzz_test",
    "forge_invariant_test",
    "forge_fork_test",
    "forge_isolate_test",
    "forge_build_no_cache",
    "forge_build_with_cache",
    "forge_coverage",
];

/// Configuration for a repository to benchmark.
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
        let parts: Vec<&str> = spec.splitn(2, ':').collect();
        let repo_path = parts[0];
        let custom_rev = parts.get(1).copied();

        let path_parts: Vec<&str> = repo_path.split('/').collect();
        if path_parts.len() != 2 {
            eyre::bail!("Invalid repo format '{}'. Expected 'org/repo' or 'org/repo:rev'", spec);
        }

        let org = path_parts[0];
        let repo = path_parts[1];

        let existing_config = BENCHMARK_REPOS.iter().find(|r| r.org == org && r.repo == repo);

        let config = if let Some(existing) = existing_config {
            let mut config = existing.clone();
            if let Some(rev) = custom_rev {
                config.rev = rev.to_string();
            }
            config
        } else {
            Self {
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

/// Default repositories for benchmarking.
pub fn default_benchmark_repos() -> Vec<RepoConfig> {
    vec![
        RepoConfig {
            name: "ithacaxyz-account".to_string(),
            org: "ithacaxyz".to_string(),
            repo: "account".to_string(),
            rev: "v0.3.2".to_string(),
        },
        RepoConfig {
            name: "solady".to_string(),
            org: "Vectorized".to_string(),
            repo: "solady".to_string(),
            rev: "v0.1.22".to_string(),
        },
    ]
}

pub static BENCHMARK_REPOS: Lazy<Vec<RepoConfig>> = Lazy::new(default_benchmark_repos);

/// Path to the built-in benchmark fixture suite.
pub const BENCH_SUITE_DIR: &str = "benches/fixtures/bench-suite";

/// A benchmark project that represents a cloned repository ready for testing.
pub struct BenchmarkProject {
    pub name: String,
    pub root_path: PathBuf,
    #[expect(dead_code)]
    temp_project: TempProject,
}

impl BenchmarkProject {
    /// Set up a benchmark project by cloning a remote repository.
    #[allow(unused_must_use)]
    pub fn setup(config: &RepoConfig) -> Result<Self> {
        let temp_project =
            TempProject::dapptools().wrap_err("Failed to create temporary project")?;

        let root_path = temp_project.root().to_path_buf();
        let root = root_path.to_str().unwrap();

        // Clear temp directory.
        for entry in std::fs::read_dir(&root_path)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                std::fs::remove_dir_all(&path).ok();
            } else {
                std::fs::remove_file(&path).ok();
            }
        }

        let repo_url = format!("https://github.com/{}/{}.git", config.org, config.repo);
        clone_remote(&repo_url, root, true);

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

        Self::install_npm_dependencies(&root_path)?;

        sh_println!("  ✅ Project {} setup complete at {}", config.name, root);
        Ok(Self { name: config.name.clone(), root_path, temp_project })
    }

    /// Set up a benchmark project from a local directory (copies to a temp dir).
    #[allow(unused_must_use)]
    pub fn setup_local(source: &Path) -> Result<Self> {
        let temp_project =
            TempProject::dapptools().wrap_err("Failed to create temporary project")?;

        let root_path = temp_project.root().to_path_buf();

        // Clear temp directory.
        for entry in std::fs::read_dir(&root_path)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                std::fs::remove_dir_all(&path).ok();
            } else {
                std::fs::remove_file(&path).ok();
            }
        }

        // Copy source directory contents into temp dir.
        copy_dir_recursive(source, &root_path)?;

        let name = source
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "local".to_string());

        sh_println!("  ✅ Local project {} setup complete at {}", name, root_path.display());
        Ok(Self { name, root_path, temp_project })
    }

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

    /// Run a command with hyperfine and return the results.
    #[allow(clippy::too_many_arguments)]
    fn hyperfine(
        &self,
        benchmark_name: &str,
        label: &str,
        command: &str,
        runs: u32,
        setup: Option<&str>,
        prepare: Option<&str>,
        conclude: Option<&str>,
        verbose: bool,
        env_vars: &[(&str, &str)],
    ) -> Result<HyperfineResult> {
        let temp_dir = std::env::temp_dir();
        let json_dir =
            temp_dir.join("foundry-bench").join(benchmark_name).join(label).join(&self.name);
        std::fs::create_dir_all(&json_dir)?;

        let json_path = json_dir.join(format!("{benchmark_name}.json"));

        let mut hyperfine_cmd = Command::new("hyperfine");
        hyperfine_cmd
            .current_dir(&self.root_path)
            .arg("--runs")
            .arg(runs.to_string())
            .arg("--export-json")
            .arg(&json_path)
            .arg("--shell=bash");

        for &(key, value) in env_vars {
            hyperfine_cmd.env(key, value);
        }

        if let Some(setup_cmd) = setup {
            hyperfine_cmd.arg("--setup").arg(setup_cmd);
        }
        if let Some(prepare_cmd) = prepare {
            hyperfine_cmd.arg("--prepare").arg(prepare_cmd);
        }
        if let Some(conclude_cmd) = conclude {
            hyperfine_cmd.arg("--conclude").arg(conclude_cmd);
        }

        if verbose {
            hyperfine_cmd.arg("--show-output");
            hyperfine_cmd.stderr(std::process::Stdio::inherit());
            hyperfine_cmd.stdout(std::process::Stdio::inherit());
        }

        hyperfine_cmd.arg(command);

        let status = hyperfine_cmd.status().wrap_err("Failed to run hyperfine")?;
        if !status.success() {
            eyre::bail!("Hyperfine failed for command: {}", command);
        }

        let json_content = std::fs::read_to_string(json_path)?;
        let output: HyperfineOutput = serde_json::from_str(&json_content)?;

        output.results.into_iter().next().ok_or_else(|| eyre::eyre!("No results from hyperfine"))
    }

    /// Run a specific benchmark by name.
    pub fn run(
        &self,
        benchmark: &str,
        label: &str,
        runs: u32,
        verbose: bool,
        fork_url: Option<&str>,
    ) -> Result<HyperfineResult> {
        match benchmark {
            "forge_test" => self.bench_forge_test(label, runs, verbose),
            "forge_build_no_cache" => self.bench_forge_build_no_cache(label, runs, verbose),
            "forge_build_with_cache" => self.bench_forge_build_with_cache(label, runs, verbose),
            "forge_fuzz_test" => self.bench_forge_fuzz_test(label, runs, verbose),
            "forge_invariant_test" => self.bench_forge_invariant_test(label, runs, verbose),
            "forge_fork_test" => self.bench_forge_fork_test(label, runs, verbose, fork_url),
            "forge_coverage" => self.bench_forge_coverage(label, runs, verbose),
            "forge_isolate_test" => self.bench_forge_isolate_test(label, runs, verbose),
            _ => eyre::bail!("Unknown benchmark: {}", benchmark),
        }
    }

    fn bench_forge_test(&self, label: &str, runs: u32, verbose: bool) -> Result<HyperfineResult> {
        self.hyperfine(
            "forge_test",
            label,
            "forge test",
            runs,
            Some("forge build"),
            None,
            None,
            verbose,
            &[],
        )
    }

    fn bench_forge_fuzz_test(
        &self,
        label: &str,
        runs: u32,
        verbose: bool,
    ) -> Result<HyperfineResult> {
        self.hyperfine(
            "forge_fuzz_test",
            label,
            r#"forge test --match-test "test[^(]*\([^)]+\)""#,
            runs,
            Some("forge build"),
            None,
            None,
            verbose,
            &[],
        )
    }

    fn bench_forge_invariant_test(
        &self,
        label: &str,
        runs: u32,
        verbose: bool,
    ) -> Result<HyperfineResult> {
        self.hyperfine(
            "forge_invariant_test",
            label,
            r#"forge test --match-test "invariant""#,
            runs,
            Some("forge build"),
            None,
            None,
            verbose,
            &[],
        )
    }

    fn bench_forge_fork_test(
        &self,
        label: &str,
        runs: u32,
        verbose: bool,
        fork_url: Option<&str>,
    ) -> Result<HyperfineResult> {
        let url = fork_url.ok_or_else(|| eyre::eyre!("forge_fork_test requires --fork-url"))?;
        self.hyperfine(
            "forge_fork_test",
            label,
            r#"forge test --fork-url "$FOUNDRY_BENCH_FORK_URL""#,
            runs,
            Some("forge build"),
            None,
            None,
            verbose,
            &[("FOUNDRY_BENCH_FORK_URL", url)],
        )
    }

    fn bench_forge_build_with_cache(
        &self,
        label: &str,
        runs: u32,
        verbose: bool,
    ) -> Result<HyperfineResult> {
        self.hyperfine(
            "forge_build_with_cache",
            label,
            "FOUNDRY_LINT_LINT_ON_BUILD=false forge build",
            runs,
            None,
            Some("forge build"),
            None,
            verbose,
            &[],
        )
    }

    fn bench_forge_build_no_cache(
        &self,
        label: &str,
        runs: u32,
        verbose: bool,
    ) -> Result<HyperfineResult> {
        self.hyperfine(
            "forge_build_no_cache",
            label,
            "FOUNDRY_LINT_LINT_ON_BUILD=false forge build",
            runs,
            Some("forge clean"),
            None,
            Some("forge clean"),
            verbose,
            &[],
        )
    }

    fn bench_forge_coverage(
        &self,
        label: &str,
        runs: u32,
        verbose: bool,
    ) -> Result<HyperfineResult> {
        self.hyperfine(
            "forge_coverage",
            label,
            "forge coverage --ir-minimum",
            runs,
            None,
            None,
            None,
            verbose,
            &[],
        )
    }

    fn bench_forge_isolate_test(
        &self,
        label: &str,
        runs: u32,
        verbose: bool,
    ) -> Result<HyperfineResult> {
        self.hyperfine(
            "forge_isolate_test",
            label,
            "forge test --isolate",
            runs,
            Some("forge build"),
            None,
            None,
            verbose,
            &[],
        )
    }

    /// Get the root path of the project.
    pub fn root(&self) -> &Path {
        &self.root_path
    }
}

/// Switch to a specific Foundry version using foundryup.
#[allow(unused_must_use)]
pub fn switch_foundry_version(version: &str) -> Result<()> {
    let output = Command::new("foundryup")
        .args(["--use", version])
        .output()
        .wrap_err("Failed to run foundryup")?;

    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("command failed") && stderr.contains("forge --version") {
        eyre::bail!(
            "Foundry binaries maybe corrupted. Please reinstall by running \
             `foundryup --install <version>`"
        );
    }

    if !output.status.success() {
        sh_eprintln!("foundryup stderr: {stderr}");
        eyre::bail!("Failed to switch to foundry version: {}", version);
    }

    sh_println!("  Successfully switched to version: {version}");
    Ok(())
}

/// Install a specific Foundry version.
pub fn install_foundry_version(version: &str) -> Result<()> {
    let status = Command::new("foundryup")
        .args(["--install", version, "--force"])
        .status()
        .wrap_err("Failed to run foundryup")?;

    if !status.success() {
        eyre::bail!("Failed to install Foundry version: {}", version);
    }
    Ok(())
}

/// Get the current forge version string.
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

/// Get full forge version details including commit hash and date.
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

    let lines: Vec<&str> = full_output.lines().collect();
    if lines.len() >= 3 {
        let version = lines[0].trim();
        let commit = lines[1].trim().replace("Commit SHA: ", "");
        let timestamp = lines[2].trim().replace("Build Timestamp: ", "");

        let short_commit = &commit[..commit.len().min(7)];
        let date = timestamp.split('T').next().unwrap_or(&timestamp);

        Ok(format!("{version} ({short_commit} {date})"))
    } else {
        Ok(lines.first().unwrap_or(&"unknown").to_string())
    }
}

/// Recursively copy a directory tree.
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}
