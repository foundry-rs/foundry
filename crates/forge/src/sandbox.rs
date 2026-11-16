//! Temporary copies of a project where Forge builds and runs generated tests.

use crate::{result::TestResult, workspace};
use eyre::{Context, Result, eyre};
use foundry_common::fs::canonicalize_path;
use foundry_compilers::{cache::SOLIDITY_FILES_CACHE_FILENAME, solc::Solc};
use foundry_config::{Config, DenyLevel};
use serde::Deserialize;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Output, Stdio},
    thread,
    time::{Duration, Instant},
};
use tempfile::TempDir;
use toml_edit::DocumentMut;

#[cfg(unix)]
use rustix::process::{Pid, Signal, kill_process_group};
#[cfg(unix)]
use std::{os::unix::process::CommandExt, sync::Mutex};

/// How often Forge checks whether an agent session with a timeout has exited. Sessions take
/// minutes, so this adds no noticeable delay.
const SESSION_POLL: Duration = Duration::from_millis(100);
/// How long one Forge build or test run in a workspace can take.
pub(crate) const RUN_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// Agent settings and instruction files that a workspace leaves out at every level. Settings can
/// hold hooks that run commands on the host. `CLAUDE.md` and `AGENTS.md` are project docs, but
/// they steer the agents that work in the copy, so a user who wants them passes them with
/// `--context`. Entries of dependency directories are links, so they keep their own files.
const AGENT_CONFIG: [&str; 14] = [
    ".agents",
    ".claude",
    ".clinerules",
    ".codex",
    ".cursor",
    ".cursorrules",
    ".gemini",
    ".windsurf",
    ".windsurfrules",
    "AGENT.md",
    "AGENTS.md",
    "CLAUDE.local.md",
    "CLAUDE.md",
    "GEMINI.md",
];
/// The environment variables that builds and test runs of generated code see, besides `HOME`,
/// `FOUNDRY_PROFILE`, and the seed settings. Generated tests run outside the agent's sandbox, so
/// they must not read the secrets of Forge's environment with the env cheatcodes.
const PASSED_ENV: [&str; 7] = ["LANG", "LC_ALL", "PATH", "SYSTEMROOT", "TEMP", "TMP", "TMPDIR"];
/// Overrides configured test filters, so they cannot hide generated tests. `$^` matches no
/// contract or test name, and no project file is under `/forge-properties-none`.
const SELECT_ALL_TESTS: [&str; 10] = [
    "--match-contract",
    ".*",
    "--match-test",
    ".*",
    "--no-match-contract",
    "$^",
    "--no-match-test",
    "$^",
    "--no-match-path",
    "/forge-properties-none/**",
];

/// The process groups of the running agent sessions, which Forge ends if it is interrupted.
#[cfg(unix)]
static SESSIONS: Mutex<Vec<Pid>> = Mutex::new(Vec::new());

/// A Solidity file that an agent wrote, by its path relative to the project root.
#[derive(Clone, Debug)]
pub(crate) struct TestFile {
    pub(crate) path: PathBuf,
    pub(crate) content: String,
}

/// Builds and runs generated code in temporary copies of the project. Generated code runs outside
/// the sandbox of whatever wrote it, so a copy leaves out hidden root entries such as `.env` and
/// agent settings, and Forge runs in it with only a few environment variables.
pub(crate) struct Sandbox<'a> {
    config: &'a Config,
    /// Whether generated tests can call `vm.ffi`, if the project enables `ffi`.
    allow_ffi: bool,
    /// Whether generated tests can use the network and the endpoints that the config names.
    allow_network: bool,
    forge: PathBuf,
    /// Project paths that workspaces leave out, besides the build output and the cache.
    skipped: Vec<PathBuf>,
    /// The workspace whose build output every new workspace copies.
    build_cache: Option<TempDir>,
    /// The home directory of builds and test runs, whose `.svm` links to the installed compilers.
    home: PathBuf,
    /// The temporary directory that holds the home directory and every workspace, so one removal
    /// cleans up after an interrupt. It drops last.
    root: TempDir,
}

impl<'a> Sandbox<'a> {
    /// Returns a sandbox for the project of `config`, whose workspaces leave out the `skipped`
    /// project paths and allow `ffi` and network access only as given.
    pub(crate) fn new(
        config: &'a Config,
        skipped: Vec<PathBuf>,
        allow_ffi: bool,
        allow_network: bool,
    ) -> Result<Self> {
        let forge = std::env::current_exe().wrap_err("failed to resolve Forge executable")?;
        let root = tempfile::Builder::new().prefix("forge-properties-").tempdir()?;
        let home = root.path().join("home");
        fs::create_dir(&home)?;
        if let Some(svm) = Solc::svm_home().filter(|svm| svm.is_dir()) {
            workspace::symlink_entry(&svm, &home.join(".svm"), fs::metadata(&svm)?.file_type())?;
        }
        Ok(Self { config, allow_ffi, allow_network, forge, skipped, build_cache: None, home, root })
    }

    /// Makes an interrupt end every session and test run and remove the workspaces before Forge
    /// exits. They run in their own process groups, so a terminal interrupt does not reach them.
    pub(crate) fn end_on_interrupt(&self, reserved: Vec<PathBuf>) -> Result<()> {
        let root = self.root.path().to_path_buf();
        ctrlc::set_handler(move || {
            // The lock stays held until Forge exits, so no thread starts another process.
            #[cfg(unix)]
            let sessions = SESSIONS.lock().unwrap();
            #[cfg(unix)]
            for group in sessions.iter() {
                let _ = kill_process_group(*group, Signal::KILL);
            }
            let _ = fs::remove_dir_all(&root);
            // Exiting skips the drops, so the reserved directories that stayed empty go here.
            for dir in &reserved {
                let _ = fs::remove_dir(dir);
            }
            std::process::exit(130);
        })?;
        Ok(())
    }

    /// Builds the project once in a workspace whose build output later workspaces copy, so a
    /// test run compiles only the files that it adds. Returns the compiler errors of a project
    /// that does not build; it gets no shared output, and each test run builds it in full.
    pub(crate) fn build(&mut self) -> Result<Option<String>> {
        let workspace = self.workspace(&[])?;
        if let Some(errors) = self.build_errors(workspace.path())? {
            return Ok(Some(errors));
        }
        self.build_cache = Some(workspace);
        Ok(None)
    }

    /// Builds a workspace and returns the compiler errors, if any. If the project denies
    /// warnings, the warnings are errors. A workspace starts from a build without such warnings,
    /// so only the files that this build compiles can warn.
    pub(crate) fn build_errors(&self, workspace: &Path) -> Result<Option<String>> {
        // The lint step can fail where the compiler succeeds, and it does not change the tests.
        let Some(output) = self.output(self.command(workspace).args(["build", "--no-lint"]))?
        else {
            return Ok(Some(format!("the build reached the timeout of {RUN_TIMEOUT:?}")));
        };
        if !output.status.success() {
            return Ok(Some(stderr(&output)));
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(stdout.find("Warning (").filter(|_| self.config.deny.warnings()).map(|start| {
            format!("the project denies compiler warnings: {}", shorten(&stdout[start..]))
        }))
    }

    /// Copies the project and the shared build output into a temporary workspace and adds the
    /// files.
    pub(crate) fn workspace(&self, files: &[TestFile]) -> Result<TempDir> {
        let config = self.config;
        let workspace =
            tempfile::Builder::new().prefix("workspace-").tempdir_in(self.root.path())?;
        let dependencies = self.dependencies();
        let skipped = [config.out.clone(), config.cache_path.clone()]
            .into_iter()
            .chain(self.skipped.iter().cloned())
            .collect::<Vec<_>>();
        copy_project_tree(config, workspace.path(), Path::new(""), &dependencies, &skipped)?;
        // Internal runs see the project's resolved config, including inherited `extends` values,
        // absolute paths, and environment overrides, without reading files outside the workspace.
        let mut rebased = workspace::rebase_config_paths(config, workspace.path());
        rebased.extends = None;
        restrict_config(&mut rebased, self.allow_ffi, self.allow_network);
        // Foundry turns the compiler cache off when it denies warnings, so a workspace builds with
        // `deny = "never"`, and `Sandbox::build_errors` checks the warnings instead.
        rebased.deny = DenyLevel::Never;
        // With dynamic test linking, every new source file makes the compiler rebuild all files, so
        // a workspace compiles without it. It changes how tests deploy contracts, not what they
        // assert.
        rebased.dynamic_test_linking = false;
        // Dependency entries link to the project's entries, and the compiler resolves links before
        // it checks the allowed paths. A source that the compiler reads itself, such as an import
        // that climbs above the root into a nested library, must stay readable.
        rebased.allow_paths.extend(
            dependencies
                .iter()
                .filter(|path| path.is_dir())
                .filter_map(|path| canonicalize_path(path).ok()),
        );
        let mut toml = rebased.to_string_pretty()?.parse::<DocumentMut>()?;
        // Inline config may name the project's other profiles, so they must still exist. The TOML
        // writer quotes names such as `""` or `a.b` that are not bare keys.
        let profiles = toml[Config::PROFILE_SECTION]
            .as_table_mut()
            .expect("the resolved config has a profile table");
        for profile in config.profiles.iter().filter(|profile| **profile != config.profile) {
            profiles.entry(profile.as_str().as_str()).or_insert_with(toml_edit::table);
        }
        fs::write(workspace.path().join(Config::FILE_NAME), toml.to_string())?;
        // Remappings are already resolved and rebased in the workspace config. The Soldeer lock
        // goes too: `forge build` then hashes each Soldeer dependency to check it, and the hash
        // panics when the dependency is a link, as in a workspace.
        for file in ["remappings.txt", "soldeer.lock"] {
            let path = workspace.path().join(file);
            if path.exists() {
                fs::remove_file(path)?;
            }
        }
        if let Some(build) = &self.build_cache {
            copy_build_output(config, build.path(), workspace.path())?;
        }
        for file in files {
            let path = workspace.path().join(&file.path);
            ensure_inside(workspace.path(), &path, &file.path)?;
            fs::create_dir_all(path.parent().expect("test path has a parent"))?;
            fs::write(path, &file.content)?;
        }
        Ok(workspace)
    }

    /// Returns the dependency directories of the project, whose entries workspaces link instead of
    /// copying them.
    pub(crate) fn dependencies(&self) -> Vec<PathBuf> {
        let config = self.config;
        config
            .libs
            .iter()
            .map(|lib| lib.as_path())
            .chain(["node_modules".as_ref(), "dependencies".as_ref()])
            .map(|path| config.root.join(path))
            .collect()
    }

    /// Returns a Forge command in a workspace. A workspace has its own resolved `foundry.toml`, so
    /// the command needs only the selected profile from the environment. Generated tests cannot
    /// read other variables.
    pub(crate) fn command(&self, workspace: &Path) -> Command {
        let mut command = Command::new(&self.forge);
        command
            .current_dir(workspace)
            .env_clear()
            .envs(PASSED_ENV.iter().filter_map(|key| Some((key, std::env::var_os(key)?))))
            .env("HOME", &self.home)
            .env("FOUNDRY_PROFILE", self.config.profile.as_str().as_str());
        command
    }

    /// Returns a command in a workspace that runs every test in the files with JSON output.
    /// Configured filters cannot hide the tests, inline config can only lower runs, and the tests
    /// use the network only if the sandbox allows it.
    pub(crate) fn test_command(&self, workspace: &Path, files: &[TestFile]) -> Command {
        // A path can hold glob characters, such as a comma in the test directory, which would
        // split the alternatives.
        let paths = files
            .iter()
            .map(|file| globset::escape(&file.path.display().to_string()).replace(',', "[,]"))
            .collect::<Vec<_>>()
            .join(",");
        let mut command = self.command(workspace);
        command
            .args(["test", "--json", "--restrict-inline-config", "--match-path"])
            .arg(format!("{{{paths}}}"))
            .args(SELECT_ALL_TESTS);
        if !self.allow_network {
            command.arg("--no-network");
        }
        command
    }

    /// Runs a command of [`Sandbox::command`] and returns its output, or `None` if it reached
    /// [`RUN_TIMEOUT`]. Generated tests can loop for as long as their gas lasts, so Forge ends the
    /// command and every process that it started at the timeout.
    pub(crate) fn output(&self, command: &mut Command) -> Result<Option<Output>> {
        let dir = tempfile::Builder::new().prefix("output-").tempdir_in(self.root.path())?;
        let (stdout, stderr) = (dir.path().join("stdout"), dir.path().join("stderr"));
        command
            .stdin(Stdio::null())
            .stdout(fs::File::create(&stdout)?)
            .stderr(fs::File::create(&stderr)?);
        let Some(status) = run_session(command, Some(RUN_TIMEOUT.as_secs()), "forge")? else {
            return Ok(None);
        };
        Ok(Some(Output { status, stdout: fs::read(stdout)?, stderr: fs::read(stderr)? }))
    }
}

/// Turns off `ffi` unless `allow_ffi`, and removes the RPC and Etherscan settings unless
/// `allow_network`. Every workspace of a [`Sandbox`] writes its config through this, so no
/// generated test can call `vm.ffi` or reach an endpoint that the config names. Config ignores
/// `FFI` environment variables.
fn restrict_config(config: &mut Config, allow_ffi: bool, allow_network: bool) {
    config.ffi &= allow_ffi;
    // Generated tests could send data to any host, and the endpoints can hold API keys.
    if !allow_network {
        config.eth_rpc_url = None;
        config.eth_rpc_jwt = None;
        config.eth_rpc_headers = None;
        config.etherscan_api_key = None;
        config.etherscan = Default::default();
        config.rpc_endpoints = Default::default();
    }
}

/// Fails if the nearest existing ancestor of `path` resolves outside `root`, for example through a
/// linked directory.
pub(crate) fn ensure_inside(root: &Path, path: &Path, orig: &Path) -> Result<()> {
    match path.ancestors().find(|ancestor| ancestor.exists()) {
        Some(existing) => workspace::ensure_within_root(root, existing, "generated test", orig),
        None => Ok(()),
    }
}

/// Copies a project directory into a workspace, without Git data, the `skipped` paths, agent
/// settings and instructions (`AGENT_CONFIG`), and, at the project root, hidden entries such as
/// `.env`.
///
/// A dependency directory becomes a real directory whose entries link to the project's entries.
/// A linked dependency directory would resolve outside the workspace, and the compiler then
/// cannot resolve a dependency's own absolute imports, such as `src/lib/X.sol` in a pnpm package.
/// A link to a path in the project leads to that path in the workspace, so it stays inside it.
fn copy_project_tree(
    config: &Config,
    copy: &Path,
    dir: &Path,
    dependencies: &[PathBuf],
    skipped: &[PathBuf],
) -> Result<()> {
    fs::create_dir_all(copy.join(dir))?;
    for entry in fs::read_dir(config.root.join(dir))? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let rel = dir.join(&name);
        let target = copy.join(&rel);
        if name == ".git"
            || AGENT_CONFIG.iter().any(|config| name == *config)
            || (dir.as_os_str().is_empty() && name.to_string_lossy().starts_with('.'))
            || skipped.contains(&path)
        {
            continue;
        }
        let file_type = entry.file_type()?;
        if dependencies.contains(&path) && path.is_dir() {
            fs::create_dir_all(&target)?;
            for dependency in fs::read_dir(&path)? {
                let dependency = dependency?;
                let link = target.join(dependency.file_name());
                workspace::symlink_entry(&dependency.path(), &link, dependency.file_type()?)?;
            }
        } else if file_type.is_symlink() {
            let link = match path.canonicalize() {
                Ok(resolved) => match resolved.strip_prefix(&config.root) {
                    Ok(resolved) => copy.join(resolved),
                    Err(_) => resolved,
                },
                // A dangling link stays as it is.
                Err(_) => fs::read_link(&path)?,
            };
            workspace::symlink_entry(&link, &target, file_type)?;
        } else if file_type.is_dir() {
            copy_project_tree(config, copy, &rel, dependencies, skipped)?;
        } else {
            fs::copy(&path, &target)?;
        }
    }
    Ok(())
}

/// Copies the build output and the compiler cache of a built workspace into another workspace.
/// The copy clones files where the file system can (APFS, Btrfs, XFS).
///
/// The compiler cache keys sources by their path relative to the root, but it stores remappings
/// with the absolute root, and other remappings invalidate it. Both workspaces have the same files
/// at the same relative paths, so moving the remappings to the new root keeps the cache valid, and
/// the compiler rebuilds only sources whose content changed.
fn copy_build_output(config: &Config, build: &Path, workspace: &Path) -> Result<()> {
    let out = workspace::relative_to_root(&config.root, &config.out);
    let cache = workspace::relative_to_root(&config.root, &config.cache_path)
        .join(SOLIDITY_FILES_CACHE_FILENAME);
    // Shared output outside the project would mix the builds of all workspaces.
    if out.is_absolute() || cache.is_absolute() || !build.join(&cache).exists() {
        return Ok(());
    }
    workspace::copy_dir_recursive(&build.join(&out), &workspace.join(&out))?;
    let mut text = fs::read_to_string(build.join(&cache))?;
    // The compiler can see either spelling of a temporary path, such as `/private/var` on macOS.
    for (from, to) in [
        (canonicalize_path(build)?, canonicalize_path(workspace)?),
        (build.to_path_buf(), workspace.to_path_buf()),
    ] {
        text = text.replace(&format!("{}/", from.display()), &format!("{}/", to.display()));
    }
    fs::create_dir_all(workspace.join(&cache).parent().expect("cache file has a parent"))?;
    fs::write(workspace.join(&cache), text)?;
    Ok(())
}

/// Returns every test result in Forge's JSON output with its contract and test name.
pub(crate) fn suite_results(output: &Output) -> Result<Vec<(String, String, TestResult)>> {
    let suites = serde_json::from_slice::<serde_json::Value>(&output.stdout)
        .wrap_err_with(|| format!("invalid Forge JSON: {}", stderr(output)))?;
    let suites = suites.as_object().ok_or_else(|| eyre!("Forge JSON is not an object"))?;
    let mut results = Vec::new();
    for (contract, suite) in suites {
        let Some(tests) = suite.get("test_results").and_then(serde_json::Value::as_object) else {
            continue;
        };
        for (test, result) in tests {
            let result = TestResult::deserialize(result)
                .wrap_err_with(|| format!("invalid result for {contract}::{test}"))?;
            results.push((contract.clone(), test.clone(), result));
        }
    }
    Ok(results)
}

/// Returns stderr without compiler warning blocks and keeps both ends when it is still long.
/// Warnings can fill the limit before the decisive error, and many tools print the error last.
/// Without a compiler error, as with `deny = "warnings"`, the warnings are the errors and stay.
pub(crate) fn stderr(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let has_error = stderr.lines().any(|line| line.starts_with("Error ("));
    let mut in_warning = false;
    let stderr = stderr
        .lines()
        .filter(|line| {
            if !has_error {
                return true;
            }
            if line.starts_with("Warning") {
                in_warning = true;
            } else if line.starts_with("Error") {
                in_warning = false;
            } else if in_warning && line.trim().is_empty() {
                in_warning = false;
                return false;
            }
            !in_warning
        })
        .collect::<Vec<_>>()
        .join("\n");
    shorten(&stderr)
}

/// Keeps both ends of a long tool output. Many tools print the decisive error last.
pub(crate) fn shorten(text: &str) -> String {
    const HALF: usize = 1_000;
    let text = text.trim();
    let chars = text.chars().count();
    if chars <= 2 * HALF {
        return text.to_string();
    }
    let head = text.chars().take(HALF).collect::<String>();
    let tail = text.chars().skip(chars - HALF).collect::<String>();
    format!("{head}\n[...]\n{tail}")
}

/// Runs a session command in its own process group until it exits or reaches the timeout in
/// seconds, then ends the process group, so no process that the session started keeps running.
/// Returns `None` if the session reached the timeout.
pub(crate) fn run_session(
    command: &mut Command,
    timeout: Option<u64>,
    session: &str,
) -> Result<Option<ExitStatus>> {
    #[cfg(unix)]
    command.process_group(0);
    // The lock spans the spawn, so an interrupt cannot miss a session that just started.
    #[cfg(unix)]
    let mut sessions = SESSIONS.lock().unwrap();
    let mut child = command.spawn().wrap_err_with(|| format!("failed to run {session} command"))?;
    #[cfg(unix)]
    let group = Pid::from_child(&child);
    #[cfg(unix)]
    {
        sessions.push(group);
        drop(sessions);
    }
    let status = match timeout {
        Some(timeout) => {
            let deadline = Instant::now() + Duration::from_secs(timeout);
            loop {
                match child.try_wait() {
                    Ok(None) if Instant::now() < deadline => thread::sleep(SESSION_POLL),
                    status => break status,
                }
            }
        }
        None => child.wait().map(Some),
    };
    // The session ends with the command: no process that it started keeps running.
    #[cfg(unix)]
    {
        let _ = kill_process_group(group, Signal::KILL);
        SESSIONS.lock().unwrap().retain(|running| *running != group);
    }
    Ok(match status? {
        Some(status) => Some(status),
        None => {
            #[cfg(not(unix))]
            let _ = child.kill();
            child.wait()?;
            None
        }
    })
}
