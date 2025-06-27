use eyre::{Result, WrapErr};
use foundry_compilers::project_util::TempProject;
use foundry_test_utils::util::clone_remote;
use std::{
    env,
    path::{Path, PathBuf},
    process::{Command, Output},
};

/// Configuration for repositories to benchmark
#[derive(Debug, Clone)]
pub struct RepoConfig {
    pub name: &'static str,
    pub org: &'static str,
    pub repo: &'static str,
    pub rev: &'static str,
}

/// Available repositories for benchmarking
pub static BENCHMARK_REPOS: &[RepoConfig] = &[
    RepoConfig { name: "ithacaxyz-account", org: "ithacaxyz", repo: "account", rev: "main" },
    // Temporarily reduced for testing
    // RepoConfig { name: "solady", org: "Vectorized", repo: "solady", rev: "main" },
    // RepoConfig { name: "v4-core", org: "Uniswap", repo: "v4-core", rev: "main" },
    // RepoConfig { name: "morpho-blue", org: "morpho-org", repo: "morpho-blue", rev: "main" },
    // RepoConfig { name: "spark-psm", org: "marsfoundation", repo: "spark-psm", rev: "master" },
];

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
/// - "nightly-<rev>" - Nightly build with specific revision
pub static FOUNDRY_VERSIONS: &[&str] = &["stable", "nightly"];

/// A benchmark project that represents a cloned repository ready for testing
pub struct BenchmarkProject {
    pub name: String,
    pub temp_project: TempProject,
    pub root_path: PathBuf,
}

impl BenchmarkProject {
    /// Set up a benchmark project by cloning the repository
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
                .args(["checkout", config.rev])
                .status()
                .wrap_err("Failed to checkout revision")?;

            if !status.success() {
                eyre::bail!("Git checkout failed for {}", config.name);
            }
        }

        // Git submodules are already cloned via --recursive flag
        // But npm dependencies still need to be installed
        Self::install_npm_dependencies(&root_path)?;

        println!("  ✅ Project {} setup complete at {}", config.name, root);
        Ok(BenchmarkProject { name: config.name.to_string(), root_path, temp_project })
    }

    /// Install npm dependencies if package.json exists
    fn install_npm_dependencies(root: &Path) -> Result<()> {
        if root.join("package.json").exists() {
            println!("  📦 Running npm install...");
            let status = Command::new("npm")
                .current_dir(root)
                .args(["install"])
                .stdout(std::process::Stdio::inherit())
                .stderr(std::process::Stdio::inherit())
                .status()
                .wrap_err("Failed to run npm install")?;

            if !status.success() {
                println!("  ⚠️  Warning: npm install failed with exit code: {:?}", status.code());
            } else {
                println!("  ✅ npm install completed successfully");
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
}

/// Switch to a specific foundry version
pub fn switch_foundry_version(version: &str) -> Result<()> {
    let output = Command::new("foundryup")
        .args(["--use", version])
        .output()
        .wrap_err("Failed to run foundryup")?;

    // Check if the error is about forge --version failing
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("command failed") && stderr.contains("forge --version") {
        eyre::bail!("Foundry binaries maybe corrupted. Please reinstall, please run `foundryup` and install the required versions.");
    }

    if !output.status.success() {
        eprintln!("foundryup stderr: {}", stderr);
        eyre::bail!("Failed to switch to foundry version: {}", version);
    }

    println!("  Successfully switched to version: {}", version);
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
        versions_env
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    } else {
        FOUNDRY_VERSIONS.iter().map(|&s| s.to_string()).collect()
    }
}
