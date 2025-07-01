use eyre::{Result, WrapErr};
use foundry_common::{sh_eprintln, sh_println};
use foundry_compilers::project_util::TempProject;
use foundry_test_utils::util::clone_remote;
use once_cell::sync::Lazy;
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use std::{
    env,
    path::{Path, PathBuf},
    process::{Command, Output},
};

pub mod criterion_types;
pub mod results;

/// Configuration for repositories to benchmark
#[derive(Debug, Clone)]
pub struct RepoConfig {
    pub name: String,
    pub org: String,
    pub repo: String,
    pub rev: String,
}

impl TryFrom<&str> for RepoConfig {
    type Error = eyre::Error;

    fn try_from(spec: &str) -> Result<Self> {
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
        // RepoConfig { name: "v4-core".to_string(), org: "Uniswap".to_string(), repo:
        // "v4-core".to_string(), rev: "main".to_string() }, RepoConfig { name:
        // "morpho-blue".to_string(), org: "morpho-org".to_string(), repo:
        // "morpho-blue".to_string(), rev: "main".to_string() }, RepoConfig { name:
        // "spark-psm".to_string(), org: "marsfoundation".to_string(), repo:
        // "spark-psm".to_string(), rev: "master".to_string() },
    ]
}

// Keep a lazy static for compatibility
pub static BENCHMARK_REPOS: Lazy<Vec<RepoConfig>> = Lazy::new(default_benchmark_repos);

/// Sample size for benchmark measurements
///
/// This controls how many times each benchmark is run for statistical analysis.
/// Higher values provide more accurate results but take longer to complete.
pub const SAMPLE_SIZE: usize = 10;

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

    /// Run forge test command and return the output
    pub fn run_forge_test(&self) -> Result<Output> {
        Command::new("forge")
            .current_dir(&self.root_path)
            .args(["test"])
            .output()
            .wrap_err("Failed to run forge test")
    }

    /// Run forge build command and return the output
    pub fn run_forge_build(&self, clean_cache: bool) -> Result<Output> {
        if clean_cache {
            // Clean first
            let _ = Command::new("forge").current_dir(&self.root_path).args(["clean"]).output();
        }

        Command::new("forge")
            .current_dir(&self.root_path)
            .args(["build"])
            .output()
            .wrap_err("Failed to run forge build")
    }

    /// Get the root path of the project
    pub fn root(&self) -> &Path {
        &self.root_path
    }

    /// Run forge test with fuzz tests only (tests with parameters)
    pub fn run_fuzz_tests(&self) -> Result<Output> {
        // Use shell to properly handle the regex pattern
        Command::new("sh")
            .current_dir(&self.root_path)
            .args(["-c", r#"forge test --match-test "test[^(]*\([^)]+\)""#])
            .output()
            .wrap_err("Failed to run forge fuzz tests")
    }

    /// Run forge coverage command with --ir-minimum flag
    pub fn run_forge_coverage(&self) -> Result<Output> {
        Command::new("forge")
            .current_dir(&self.root_path)
            .args(["coverage", "--ir-minimum"])
            .output()
            .wrap_err("Failed to run forge coverage")
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
            .map(RepoConfig::try_from)
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
