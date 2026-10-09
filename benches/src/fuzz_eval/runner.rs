//! Project preparation and forge invocation.

use super::{
    forge_json::{TestRecord, parse_test_output},
    lcov::parse_lcov,
    manifest::{Budget, Source, Target},
    results::{CoverageRun, RunStatus, SeedRun, TargetResult},
};
use eyre::{Result, WrapErr, bail, ensure};
use foundry_common::sh_eprintln;
use std::{
    collections::{BTreeMap, HashMap},
    env, fs,
    fs::File,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use wait_timeout::ChildExt;

/// Number of trailing stderr lines kept in error messages.
const STDERR_TAIL_LINES: usize = 20;

/// Settings shared by every forge invocation.
#[derive(Clone, Debug)]
pub struct RunnerConfig {
    /// `forge` binary to evaluate.
    pub forge_bin: PathBuf,
    /// Arguments appended to every `forge test` and `forge coverage` invocation.
    pub extra_args: Vec<String>,
    /// Budget values that override the manifest.
    pub budget_override: Budget,
    /// Per-invocation timeout that overrides the manifest.
    pub timeout_override: Option<u64>,
    /// Whether to run `forge coverage` for every seed.
    pub coverage: bool,
    /// Scratch directory for project copies and per-run state.
    pub work_dir: PathBuf,
    /// Directory for git checkouts, reused across evaluations.
    pub cache_dir: PathBuf,
    /// Directory for forge stdout/stderr logs.
    pub logs_dir: PathBuf,
}

/// Runs targets, preparing and building each project once.
pub struct Runner {
    config: RunnerConfig,
    /// Prepared project roots keyed by source, or the preparation error.
    projects: HashMap<Source, Result<PathBuf, String>>,
}

/// Exit state of one child process.
struct ProcessOutcome {
    exit_code: Option<i32>,
    timed_out: bool,
    wall_secs: f64,
}

impl Runner {
    pub fn new(config: RunnerConfig) -> Self {
        Self { config, projects: HashMap::new() }
    }

    /// Runs every seed of `target`. Preparation failures are recorded as errored runs rather
    /// than aborting the evaluation.
    pub fn run_target(&mut self, target: &Target, seeds: &[u64]) -> TargetResult {
        let budget = self.config.budget_override.or(&target.budget);
        let runs = match self.project_root(target) {
            Ok(root) => seeds
                .iter()
                .map(|&seed| {
                    let run = self.run_seed(target, &budget, &root, seed);
                    let _ = sh_eprintln!(
                        "  {} seed {seed}: {}, {:.3}s, bugs {:?}{}",
                        target.name,
                        run.status.as_str(),
                        run.wall_secs,
                        run.bugs_found,
                        run.branch_pct()
                            .map(|pct| format!(", branch {pct:.1}%"))
                            .unwrap_or_default()
                    );
                    run
                })
                .collect(),
            Err(err) => seeds.iter().map(|&seed| errored_run(seed, err.clone())).collect(),
        };
        TargetResult {
            name: target.name.clone(),
            split: target.split,
            source: target.source.clone(),
            expected_failures: target.expected_failures.clone(),
            failures_are_bugs: target.failures_are_bugs,
            budget,
            runs,
        }
    }

    /// Returns the prepared and built project root for `target`.
    fn project_root(&mut self, target: &Target) -> Result<PathBuf, String> {
        if let Some(prepared) = self.projects.get(&target.source) {
            return prepared.clone();
        }
        let prepared =
            self.prepare(target).map_err(|err| format!("project preparation failed: {err:#}"));
        self.projects.insert(target.source.clone(), prepared.clone());
        prepared
    }

    fn prepare(&self, target: &Target) -> Result<PathBuf> {
        let root = match &target.source {
            Source::Local { path } => {
                ensure!(path.is_dir(), "project directory {} does not exist", path.display());
                let dest = self.config.work_dir.join("projects").join(&target.name);
                if dest.exists() {
                    fs::remove_dir_all(&dest)
                        .wrap_err_with(|| format!("failed to clear {}", dest.display()))?;
                }
                copy_project(path, &dest)?;
                dest
            }
            Source::Git { repo, git_ref, subdir } => {
                let checkout = checkout_git(&self.config.cache_dir, repo, git_ref)?;
                let root =
                    subdir.as_ref().map_or_else(|| checkout.clone(), |dir| checkout.join(dir));
                ensure!(root.is_dir(), "project directory {} does not exist", root.display());
                root
            }
        };
        let _ = sh_eprintln!("building {} in {}", target.name, root.display());
        let log_dir = self.config.logs_dir.join(&target.name);
        fs::create_dir_all(&log_dir)?;
        let mut command = self.forge_command(&root, &target.env);
        command.arg("build");
        let stderr_path = log_dir.join("build.stderr.log");
        let outcome = run_process(
            &mut command,
            Duration::from_secs(self.timeout_secs(target)),
            &log_dir.join("build.stdout.log"),
            &stderr_path,
        )?;
        if outcome.timed_out {
            bail!("forge build timed out");
        }
        if outcome.exit_code != Some(0) {
            bail!("forge build failed: {}", stderr_tail(&stderr_path));
        }
        Ok(root)
    }

    fn run_seed(&self, target: &Target, budget: &Budget, root: &Path, seed: u64) -> SeedRun {
        match self.try_run_seed(target, budget, root, seed) {
            Ok(run) => run,
            Err(err) => errored_run(seed, format!("{err:#}")),
        }
    }

    fn try_run_seed(
        &self,
        target: &Target,
        budget: &Budget,
        root: &Path,
        seed: u64,
    ) -> Result<SeedRun> {
        let log_dir = self.config.logs_dir.join(&target.name);
        fs::create_dir_all(&log_dir)?;
        let stdout_path = log_dir.join(format!("seed-{seed}.test.stdout.json"));
        let stderr_path = log_dir.join(format!("seed-{seed}.test.stderr.log"));

        let mut command = self.seeded_command(target, budget, root, seed, "test")?;
        command.args(["test", "--json", "--fuzz-seed", &seed.to_string()]);
        command.args(&target.filters).args(&target.forge_args).args(&self.config.extra_args);
        let outcome = run_process(
            &mut command,
            Duration::from_secs(self.timeout_secs(target)),
            &stdout_path,
            &stderr_path,
        )?;

        let mut run = errored_run(seed, String::new());
        run.exit_code = outcome.exit_code;
        run.wall_secs = outcome.wall_secs;
        if outcome.timed_out {
            run.status = RunStatus::TimedOut;
            run.error = Some(format!("forge test timed out after {}s", self.timeout_secs(target)));
        } else {
            let stdout = fs::read_to_string(&stdout_path).unwrap_or_default();
            match parse_test_output(&stdout) {
                Some(tests) if tests.is_empty() => {
                    run.error = Some("no tests matched the target filters".to_string());
                }
                Some(tests) => {
                    run.status = RunStatus::Ok;
                    run.error = None;
                    classify(target, tests, &mut run);
                }
                None => {
                    run.error = Some(format!(
                        "forge test produced no JSON results (exit {:?}): {}",
                        outcome.exit_code,
                        stderr_tail(&stderr_path)
                    ));
                }
            }
        }
        if self.config.coverage {
            run.coverage = Some(self.run_coverage(target, budget, root, seed)?);
        }
        Ok(run)
    }

    fn run_coverage(
        &self,
        target: &Target,
        budget: &Budget,
        root: &Path,
        seed: u64,
    ) -> Result<CoverageRun> {
        let log_dir = self.config.logs_dir.join(&target.name);
        let report = log_dir.join(format!("seed-{seed}.lcov.info"));
        let stderr_path = log_dir.join(format!("seed-{seed}.coverage.stderr.log"));
        if report.exists() {
            fs::remove_file(&report)?;
        }
        let mut command = self.seeded_command(target, budget, root, seed, "coverage")?;
        command.args(["coverage", "--report", "lcov", "--report-file"]).arg(&report);
        command.args(["--fuzz-seed", &seed.to_string()]);
        command
            .args(&target.filters)
            .args(&target.forge_args)
            .args(&target.coverage_args)
            .args(&self.config.extra_args);
        let outcome = run_process(
            &mut command,
            Duration::from_secs(self.timeout_secs(target)),
            &log_dir.join(format!("seed-{seed}.coverage.stdout.log")),
            &stderr_path,
        )?;
        let mut coverage = CoverageRun {
            status: RunStatus::Error,
            error: None,
            wall_secs: outcome.wall_secs,
            stats: None,
        };
        if outcome.timed_out {
            coverage.status = RunStatus::TimedOut;
            coverage.error =
                Some(format!("forge coverage timed out after {}s", self.timeout_secs(target)));
        } else if let Ok(content) = fs::read_to_string(&report) {
            // Forge writes the report before failing on test failures, so a non-zero exit with a
            // report is expected whenever a bug is found.
            coverage.status = RunStatus::Ok;
            coverage.stats = Some(parse_lcov(&content, root, &target.coverage_paths));
        } else {
            coverage.error = Some(format!(
                "forge coverage wrote no LCOV report (exit {:?}): {}",
                outcome.exit_code,
                stderr_tail(&stderr_path)
            ));
        }
        Ok(coverage)
    }

    /// Builds a forge command with the target budget, environment, and fresh per-run state so
    /// persisted failures or corpora from one seed cannot leak into another.
    fn seeded_command(
        &self,
        target: &Target,
        budget: &Budget,
        root: &Path,
        seed: u64,
        kind: &str,
    ) -> Result<Command> {
        let state =
            self.config.work_dir.join("state").join(&target.name).join(format!("{kind}-{seed}"));
        if state.exists() {
            fs::remove_dir_all(&state)
                .wrap_err_with(|| format!("failed to clear {}", state.display()))?;
        }
        fs::create_dir_all(&state)?;
        let mut command = self.forge_command(root, &target.env);
        command
            .env("FOUNDRY_FUZZ_FAILURE_PERSIST_DIR", state.join("fuzz-failures"))
            .env("FOUNDRY_INVARIANT_FAILURE_PERSIST_DIR", state.join("invariant-failures"));
        if target.fresh_corpus {
            command
                .env("FOUNDRY_FUZZ_CORPUS_DIR", state.join("fuzz-corpus"))
                .env("FOUNDRY_INVARIANT_CORPUS_DIR", state.join("invariant-corpus"));
        }
        let args = target
            .forge_args
            .iter()
            .chain(&target.coverage_args)
            .chain(&self.config.extra_args)
            .cloned()
            .collect::<Vec<_>>();
        command.envs(budget.env(&args));
        Ok(command)
    }

    /// Builds a forge command in `root` with inherited `FOUNDRY_*` and `DAPP_*` variables removed
    /// so the caller's shell cannot change the benchmark configuration.
    fn forge_command(&self, root: &Path, target_env: &BTreeMap<String, String>) -> Command {
        let mut command = Command::new(&self.config.forge_bin);
        command.current_dir(root).stdin(Stdio::null());
        for (name, _) in env::vars_os() {
            let name_str = name.to_string_lossy();
            if name_str.starts_with("FOUNDRY_") || name_str.starts_with("DAPP_") {
                command.env_remove(&name);
            }
        }
        command.env("NO_COLOR", "1").envs(target_env);
        command
    }

    fn timeout_secs(&self, target: &Target) -> u64 {
        self.config.timeout_override.unwrap_or(target.timeout_secs)
    }
}

/// Records which expected bugs a run found and which failures are unexpected.
fn classify(target: &Target, tests: Vec<TestRecord>, run: &mut SeedRun) {
    let failing = tests.iter().filter(|test| test.failed()).collect::<Vec<_>>();
    let mut bug_durations = Vec::new();
    if target.failures_are_bugs {
        for test in &failing {
            run.bugs_found.push(format!("{}::{}", test.contract, test.name));
            bug_durations.extend(test.duration_secs);
        }
    } else {
        for expected in &target.expected_failures {
            let matching = tests.iter().filter(|test| test.matches(expected)).collect::<Vec<_>>();
            if matching.is_empty() {
                run.missing_expected.push(expected.clone());
            } else if let Some(test) = matching.iter().find(|test| test.failed()) {
                run.bugs_found.push(expected.clone());
                bug_durations.extend(test.duration_secs);
            }
        }
        run.unexpected_failures = failing
            .iter()
            .filter(|test| !target.expected_failures.iter().any(|expected| test.matches(expected)))
            .map(|test| format!("{}::{}", test.contract, test.name))
            .collect();
    }
    run.first_failure_secs = bug_durations.into_iter().reduce(f64::min);
    run.tests = tests;
}

const fn errored_run(seed: u64, error: String) -> SeedRun {
    SeedRun {
        seed,
        status: RunStatus::Error,
        error: Some(error),
        exit_code: None,
        wall_secs: 0.0,
        tests: Vec::new(),
        bugs_found: Vec::new(),
        unexpected_failures: Vec::new(),
        missing_expected: Vec::new(),
        first_failure_secs: None,
        coverage: None,
    }
}

/// Runs `command` with stdout and stderr redirected to files, killing it after `timeout`.
fn run_process(
    command: &mut Command,
    timeout: Duration,
    stdout_path: &Path,
    stderr_path: &Path,
) -> Result<ProcessOutcome> {
    let stdout = File::create(stdout_path)
        .wrap_err_with(|| format!("failed to create {}", stdout_path.display()))?;
    let stderr = File::create(stderr_path)
        .wrap_err_with(|| format!("failed to create {}", stderr_path.display()))?;
    let start = Instant::now();
    let mut child = command
        .stdout(stdout)
        .stderr(stderr)
        .spawn()
        .wrap_err_with(|| format!("failed to spawn {:?}", command.get_program()))?;
    let status = child.wait_timeout(timeout)?;
    let (exit_code, timed_out) = match status {
        Some(status) => (status.code(), false),
        None => {
            let _ = child.kill();
            let _ = child.wait();
            (None, true)
        }
    };
    Ok(ProcessOutcome { exit_code, timed_out, wall_secs: start.elapsed().as_secs_f64() })
}

fn stderr_tail(path: &Path) -> String {
    let content = fs::read_to_string(path).unwrap_or_default();
    let lines = content.lines().filter(|line| !line.trim().is_empty()).collect::<Vec<_>>();
    let tail = lines[lines.len().saturating_sub(STDERR_TAIL_LINES)..].join("\n");
    if tail.is_empty() { "<no stderr>".to_string() } else { tail }
}

/// Copies a Foundry project, skipping build outputs and caches.
fn copy_project(src: &Path, dest: &Path) -> Result<()> {
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(src).wrap_err_with(|| format!("failed to read {}", src.display()))? {
        let entry = entry?;
        let name = entry.file_name();
        if matches!(name.to_str(), Some("out" | "cache" | ".git")) {
            continue;
        }
        copy_recursive(&entry.path(), &dest.join(name))?;
    }
    Ok(())
}

fn copy_recursive(src: &Path, dest: &Path) -> Result<()> {
    if src.is_dir() {
        fs::create_dir_all(dest)?;
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            copy_recursive(&entry.path(), &dest.join(entry.file_name()))?;
        }
    } else {
        fs::copy(src, dest)
            .wrap_err_with(|| format!("failed to copy {} to {}", src.display(), dest.display()))?;
    }
    Ok(())
}

/// Clones `repo` at `git_ref` into `cache_dir`, reusing an existing checkout.
fn checkout_git(cache_dir: &Path, repo: &str, git_ref: &str) -> Result<PathBuf> {
    let dest = cache_dir.join(format!("{}@{}", sanitize(repo), sanitize(git_ref)));
    if dest.join(".git").exists() {
        return Ok(dest);
    }
    fs::create_dir_all(cache_dir)?;
    let partial = PathBuf::from(format!("{}.partial", dest.display()));
    if partial.exists() {
        fs::remove_dir_all(&partial)?;
    }
    let _ = sh_eprintln!("cloning {repo}@{git_ref}");
    git(git_command().args(["clone", "--quiet", repo]).arg(&partial))?;
    git(git_command().arg("-C").arg(&partial).args(["checkout", "--quiet", git_ref]))?;
    git(git_command().arg("-C").arg(&partial).args([
        "submodule",
        "update",
        "--quiet",
        "--init",
        "--recursive",
    ]))?;
    fs::rename(&partial, &dest)?;
    Ok(dest)
}

/// Returns a `git` command that fetches GitHub SSH remotes, including submodules, over HTTPS so
/// checkouts work without SSH keys.
fn git_command() -> Command {
    let mut command = Command::new("git");
    command.args(["-c", "url.https://github.com/.insteadOf=git@github.com:"]);
    command
}

fn git(command: &mut Command) -> Result<()> {
    let output = command.output().wrap_err("failed to run git")?;
    ensure!(
        output.status.success(),
        "{:?} failed: {}",
        command,
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(())
}

fn sanitize(value: &str) -> String {
    let value = value.trim_end_matches(".git");
    let value = value.rsplit_once("://").map_or(value, |(_, rest)| rest);
    value
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '.') { c } else { '_' })
        .collect()
}

/// Returns the first line of `forge --version`.
pub fn forge_version(forge_bin: &Path) -> Result<String> {
    let output = Command::new(forge_bin)
        .arg("--version")
        .output()
        .wrap_err_with(|| format!("failed to run {} --version", forge_bin.display()))?;
    ensure!(output.status.success(), "{} --version failed", forge_bin.display());
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout.lines().filter(|line| !line.trim().is_empty()).collect::<Vec<_>>().join("; "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Split;

    fn target(expected: &[&str], failures_are_bugs: bool) -> Target {
        Target {
            name: "t".to_string(),
            split: Split::Train,
            source: Source::Local { path: PathBuf::from("/p") },
            filters: Vec::new(),
            expected_failures: expected.iter().map(|s| s.to_string()).collect(),
            failures_are_bugs,
            budget: Budget::default(),
            timeout_secs: 1,
            forge_args: Vec::new(),
            coverage_args: Vec::new(),
            coverage_paths: Vec::new(),
            env: BTreeMap::new(),
            fresh_corpus: false,
        }
    }

    fn test(contract: &str, name: &str, status: &str, secs: f64) -> TestRecord {
        TestRecord {
            contract: contract.to_string(),
            name: name.to_string(),
            status: status.to_string(),
            kind: None,
            duration_secs: Some(secs),
        }
    }

    fn tests() -> Vec<TestRecord> {
        vec![
            test("A", "invariant_a", "Failure", 2.0),
            test("A", "testFuzz_b", "Failure", 0.5),
            test("B", "test_c", "Failure", 0.1),
            test("B", "test_d", "Success", 0.1),
        ]
    }

    #[test]
    fn classifies_expected_unexpected_and_missing() {
        let target = target(&["invariant_a", "A::testFuzz_b", "test_d", "test_missing"], false);
        let mut run = errored_run(1, String::new());
        classify(&target, tests(), &mut run);
        assert_eq!(run.bugs_found, ["invariant_a", "A::testFuzz_b"]);
        assert_eq!(run.unexpected_failures, ["B::test_c"]);
        assert_eq!(run.missing_expected, ["test_missing"]);
        assert_eq!(run.first_failure_secs, Some(0.5));
    }

    #[test]
    fn failures_are_bugs_counts_every_failure() {
        let mut run = errored_run(1, String::new());
        classify(&target(&[], true), tests(), &mut run);
        assert_eq!(run.bugs_found, ["A::invariant_a", "A::testFuzz_b", "B::test_c"]);
        assert!(run.unexpected_failures.is_empty());
        assert_eq!(run.first_failure_secs, Some(0.1));
    }

    #[test]
    fn sanitizes_cache_names() {
        assert_eq!(
            sanitize("https://github.com/Recon-Fuzz/aave-v4-scfuzzbench.git"),
            "github.com_Recon-Fuzz_aave-v4-scfuzzbench"
        );
        assert_eq!(sanitize("v0.5.6/recon"), "v0.5.6_recon");
    }
}
