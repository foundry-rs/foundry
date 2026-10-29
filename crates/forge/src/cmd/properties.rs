//! The `forge properties` command: several agents look for bugs in the targets independently and
//! in parallel, each in one session in its own copy of the project, and prove each bug with a
//! failing test. Forge runs the tests of every agent again in a fresh copy on every seed, reports a
//! bug only when enough agents agree on its root cause, and keeps one copy of each passing test
//! file. An optional verifier then tries to disprove each reported bug.

use crate::{
    cmd::test::RerunFailure,
    result::{TestResult, TestStatus},
    workspace,
};
use alloy_primitives::{U256, keccak256};
use clap::Parser;
use eyre::{Context, Result, ensure, eyre};
use foundry_cli::{opts::BuildOpts, utils::LoadConfig};
use foundry_common::{fs::canonicalize_path, sh_println, sh_warn};
use foundry_compilers::{
    Graph,
    cache::SOLIDITY_FILES_CACHE_FILENAME,
    compilers::{
        Language,
        multi::{MultiCompilerLanguage, MultiCompilerParser},
        solc::Solc,
    },
    utils::source_files_iter,
};
use foundry_config::{Config, DenyLevel, impl_figment_convert};
use foundry_evm::fuzz::CounterExample;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet, HashSet},
    ffi::OsString,
    fs,
    io::{self, Write},
    ops::Add,
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Output, Stdio},
    sync::{
        LazyLock,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime},
};
use tempfile::TempDir;
use toml_edit::DocumentMut;

#[cfg(unix)]
use rustix::process::{Pid, Signal, kill_process_group};
#[cfg(unix)]
use std::{os::unix::process::CommandExt, sync::Mutex};

mod matcher;

/// On 12 held-out cases, four small-model agents with a quorum of two found bugs in as many cases
/// as five (8.6 against 9.0 of 12, a tie in a bootstrap over cases) at four fifths of the cost.
/// With a quorum of two, they kept no report that reviewers labelled wrong, because wrong claims
/// of independent agents rarely repeat.
const DEFAULT_AGENTS: usize = 4;
const DEFAULT_QUORUM: usize = 2;
/// How often Forge checks whether an agent session with a timeout has exited. Sessions take
/// minutes, so this adds no noticeable delay.
const SESSION_POLL: Duration = Duration::from_millis(100);
/// How long one Forge build or test run in a workspace can take.
const RUN_TIMEOUT: Duration = Duration::from_secs(10 * 60);
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
pub(crate) const SELECT_ALL_TESTS: [&str; 10] = [
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

/// The task of a verifier session. The placeholders are the targets, the group file, the verdict
/// file, and the tests of the group.
const VERIFY_PROMPT: &str = include_str!("properties/verify_prompt.md");

/// The errors of what Forge blocks in its workspaces: `vm.ffi` without `--allow-ffi`, network
/// cheatcodes without `--allow-network`, env cheatcodes, which see only a few variables, and
/// inline config that `--restrict-inline-config` refuses.
static BLOCKED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"FFI is disabled|network access is disabled|environment variable "[^"]*" not found|failed to resolve env var|inline config cannot"#)
        .unwrap()
});
/// Solidity line and block comments.
static COMMENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)//[^\n]*|/\*.*?\*/").unwrap());
/// A contract declaration and its name.
static CONTRACT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bcontract\s+(\w+)").unwrap());
/// The process groups of the running agent sessions, which Forge ends if it is interrupted.
#[cfg(unix)]
static SESSIONS: Mutex<Vec<Pid>> = Mutex::new(Vec::new());

/// Run agents that prove bugs with failing tests and report the bugs that they agree on.
#[derive(Clone, Debug, Parser)]
pub struct PropertiesArgs {
    /// Source file to look for bugs in. Can be given more than once.
    ///
    /// Defaults to every source file in the project's source directory.
    #[arg(long, value_name = "PATH")]
    target: Vec<PathBuf>,

    /// Reference material for the agents, such as docs, a specification, or a threat model.
    ///
    /// Forge gives every agent the absolute path of each file. Can be given more than once.
    #[arg(long, value_name = "PATH")]
    context: Vec<PathBuf>,

    /// Executable that runs one agent session, such as a wrapper around any coding agent.
    ///
    /// Forge runs `PATH [ARG...]` once per agent, in parallel, each in its own copy of the
    /// project, with these environment variables:
    ///
    /// FORGE_PROPERTIES_PROMPT   the task: targets, context files, and output directory
    /// FORGE_PROPERTIES_TARGETS  target files, relative to the copy, one per line
    /// FORGE_PROPERTIES_CONTEXT  absolute paths of the --context files, one per line
    /// FORGE_PROPERTIES_OUT      absolute, empty directory for the agent's tests
    /// FORGE_PROPERTIES_SEED     the agent's own seed, in hex
    /// FORGE_PROPERTIES_USAGE    file where the agent can write its model usage as JSON:
    ///                           {"input_tokens": N, "output_tokens": N, "cost_usd": X}
    ///
    /// The agent writes Solidity tests under FORGE_PROPERTIES_OUT and can run `forge test` in the
    /// copy. When it exits or reaches --timeout, Forge ends its process group and runs every
    /// `*.sol` file under FORGE_PROPERTIES_OUT in a fresh copy of the project on every seed. A
    /// test that fails on every seed is a possible bug. Its claim is the comment right above the
    /// test function, `// @bug <Contract>.<function> <file>:<line> — <claim>`, else the test name.
    /// Forge keeps the files whose tests all pass. A nonzero exit status is an error, but
    /// Forge still runs the agent's tests. In Forge's runs, inline config can only lower runs,
    /// depth, and rejects and change a few fuzzer and invariant settings, and a run that takes
    /// more than ten minutes ends.
    #[arg(long, required = true, value_name = "PATH", verbatim_doc_comment)]
    agent_command: PathBuf,

    /// Argument passed to the agent command. Can be given more than once.
    #[arg(long, allow_hyphen_values = true, value_name = "ARG")]
    agent_arg: Vec<OsString>,

    /// Executable that tries to disprove each possible bug that reaches the quorum, such as a
    /// wrapper around any coding agent. Without it, Forge runs no verifier.
    ///
    /// Forge runs `PATH [ARG...]` once per group, at most --agents at a time, each in a fresh copy
    /// of the project that also holds the tests of the group, with these environment variables:
    ///
    /// FORGE_PROPERTIES_VERIFY_PROMPT  the task: try to disprove the claims of the group
    /// FORGE_PROPERTIES_GROUP          JSON file with the agents, the claims, the tests, and the
    ///                                 failure output of the group
    /// FORGE_PROPERTIES_VERDICT        file where the verifier writes its verdict as JSON:
    ///                                 {"verdict": "confirmed" | "needs_validation" | "rejected",
    ///                                  "reason": "...",
    ///                                  "evidence": [{"file": "...", "line": N, "quote": "..."}],
    ///                                  "unresolved": "..."}
    /// FORGE_PROPERTIES_USAGE          file where the verifier can write its model usage as JSON,
    ///                                 as for an agent
    ///
    /// When the verifier exits or reaches --timeout, Forge ends its process group and reads the
    /// verdict. Forge lists the confirmed groups first, then the groups that need validation with
    /// the open fact, then the rejected groups with the disproof. A missing or invalid verdict,
    /// and a rejection without evidence, count as needs_validation. Groups below the quorum are
    /// not verified.
    #[arg(long, value_name = "PATH", verbatim_doc_comment)]
    verifier_command: Option<PathBuf>,

    /// Argument passed to the verifier command. Can be given more than once.
    #[arg(long, allow_hyphen_values = true, requires = "verifier_command", value_name = "ARG")]
    verifier_arg: Vec<OsString>,

    /// Deterministic seeds used to run the tests of every agent. Supply at least two.
    #[arg(long, required = true, action = clap::ArgAction::Append, value_name = "SEED")]
    seed: Vec<U256>,

    /// Number of agents. Each agent runs one session independently and in parallel in its own
    /// copy of the project, with its own seed.
    #[arg(long, default_value_t = DEFAULT_AGENTS, value_name = "N")]
    agents: usize,

    /// Number of agents that must report the same root cause before Forge reports it as a
    /// possible bug.
    ///
    /// Two failing tests have the same root cause when their `@bug` tags name the same function
    /// in the same Solidity file at near lines. Without both tags, their claims must name the same
    /// Solidity file and share a function name at near lines or at least 30% of their words. A
    /// test joins a group only when it has the same root cause as every test in it. Forge lists
    /// the groups below the quorum after the possible bugs. Defaults to 2, or 1 with a
    /// single agent.
    #[arg(long, value_name = "K")]
    quorum: Option<usize>,

    /// Seconds after which Forge ends an agent session and runs the tests that it wrote, or ends a
    /// verifier and reads its verdict.
    #[arg(long, value_name = "SECS")]
    timeout: Option<u64>,

    /// Let generated tests run host commands with `vm.ffi` if the project enables `ffi`.
    ///
    /// Generated tests run under Forge, outside the sandbox of the agent that wrote them, so
    /// Forge turns `ffi` off in its copies of the project by default.
    #[arg(long)]
    allow_ffi: bool,

    /// Let generated tests open network connections, for example with `vm.createFork`.
    ///
    /// By default, cheatcodes that open a connection fail in Forge's copies of the project, and
    /// the copies have no RPC or Etherscan settings.
    #[arg(long)]
    allow_network: bool,

    #[command(flatten)]
    build: BuildOpts,
}

impl_figment_convert!(PropertiesArgs, build);

/// Model usage that an agent reports.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
struct Usage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    cost_usd: f64,
}

impl Add for Usage {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self {
            input_tokens: self.input_tokens + other.input_tokens,
            output_tokens: self.output_tokens + other.output_tokens,
            cost_usd: self.cost_usd + other.cost_usd,
        }
    }
}

/// A Solidity file that an agent wrote, by its path relative to the project root.
#[derive(Clone, Debug)]
pub(crate) struct TestFile {
    pub(crate) path: PathBuf,
    pub(crate) content: String,
}

/// A test that fails on every seed.
#[derive(Clone, Debug, Serialize)]
struct Failure {
    /// `Contract::test`.
    test: String,
    path: PathBuf,
    reason: String,
    /// The comment above the test.
    #[serde(skip_serializing_if = "Option::is_none")]
    claim: Option<String>,
}

/// A test of one agent that fails on every seed.
#[derive(Debug, Serialize)]
struct AgentFailure {
    agent: usize,
    #[serde(flatten)]
    failure: Failure,
    /// The copy of the test file for review, relative to the project root.
    file: PathBuf,
}

impl std::fmt::Display for AgentFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let failure = &self.failure;
        write!(f, "  agent {}: {} ({})", self.agent, failure.test, failure.reason)?;
        if let Some(claim) = &failure.claim {
            write!(f, "\n    claim: {claim}")?;
        }
        write!(f, "\n    file: {}", self.file.display())
    }
}

/// Files of an agent that Forge did not run.
#[derive(Debug, Serialize)]
struct Rejected {
    agent: usize,
    paths: Vec<PathBuf>,
    reason: String,
}

/// The failing tests of all agents that have one root cause.
#[derive(Debug, Serialize)]
struct BugGroup {
    /// The agents that report the root cause.
    agents: Vec<usize>,
    /// The failing tests, by agent. The first test has a claim, if any test has.
    failures: Vec<AgentFailure>,
    /// What the verifier concluded, if there is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    verification: Option<Verification>,
}

/// The verdict of a verifier.
///
/// The order is the order of the report.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
enum Verdict {
    /// Every attempt to disprove the claim failed.
    Confirmed,
    /// One open fact decides the claim.
    #[default]
    NeedsValidation,
    /// A disproof holds for every claim of the group.
    Rejected,
}

/// A quote from a file that supports a verdict.
#[derive(Debug, Deserialize, Serialize)]
struct Evidence {
    #[serde(default)]
    file: String,
    #[serde(default)]
    line: u64,
    #[serde(default)]
    quote: String,
}

/// The verdict file of a verifier.
#[derive(Debug, Default, Deserialize, Serialize)]
struct Judgment {
    verdict: Verdict,
    #[serde(default)]
    reason: String,
    #[serde(default)]
    evidence: Vec<Evidence>,
    /// The open fact of a verdict that needs validation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    unresolved: Option<String>,
}

/// What one verifier session left for a bug group.
#[derive(Debug, Serialize)]
struct Verification {
    #[serde(flatten)]
    judgment: Judgment,
    /// What went wrong with the verifier, such as a missing verdict. A group whose verifier left
    /// no valid verdict needs validation.
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    usage: Option<Usage>,
}

impl Verification {
    /// Returns the verification of a group whose verifier left no valid verdict.
    fn failed(error: String) -> Self {
        Self { judgment: Judgment::default(), error: Some(error), usage: None }
    }
}

impl std::fmt::Display for Verification {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let judgment = &self.judgment;
        let mut lines = Vec::new();
        if !judgment.reason.is_empty() {
            lines.push(format!("  verifier: {}", judgment.reason));
        }
        if let Some(error) = &self.error {
            lines.push(format!("  verifier error: {error}"));
        }
        if let Some(unresolved) = judgment.unresolved.as_deref().filter(|fact| !fact.is_empty()) {
            lines.push(format!("    unresolved: {unresolved}"));
        }
        for evidence in &judgment.evidence {
            let quote = evidence.quote.split_whitespace().collect::<Vec<_>>().join(" ");
            lines.push(format!("    evidence: {}:{}: {quote}", evidence.file, evidence.line));
        }
        write!(f, "{}", lines.join("\n"))
    }
}

/// The tests of some files, run on every seed.
#[derive(Default)]
struct TestRun {
    /// Tests that fail on every seed.
    possible_bugs: Vec<Failure>,
    /// Tests that pass on every seed.
    passing: Vec<RerunFailure>,
    /// Files with a test that does not pass on every seed.
    failing_files: HashSet<PathBuf>,
    /// Seeds on which Forge could not run the tests.
    errors: Vec<String>,
}

/// What one agent session left.
struct Session {
    id: usize,
    /// Why the agent command failed, if it did.
    error: Option<String>,
    timed_out: bool,
    usage: Option<Usage>,
    possible_bugs: Vec<AgentFailure>,
    rejected: Vec<Rejected>,
    /// The files whose tests pass on every seed, with the files that they import.
    kept: Vec<TestFile>,
    /// Every file that the agent wrote and Forge ran.
    files: Vec<TestFile>,
    /// The files of `files` that each of them imports, directly or not, as the compiler resolves
    /// the imports.
    imports: BTreeMap<PathBuf, BTreeSet<PathBuf>>,
}

/// The output directories that a run reserved. The ones it left empty are removed on drop.
struct RunDirs(Vec<PathBuf>);

impl Drop for RunDirs {
    fn drop(&mut self) {
        for dir in &self.0 {
            let _ = fs::remove_dir(dir);
        }
    }
}

impl PropertiesArgs {
    pub fn run(mut self) -> Result<()> {
        ensure!(self.seed.len() >= 2, "at least two --seed values are required");
        ensure!(
            self.seed.iter().collect::<HashSet<_>>().len() == self.seed.len(),
            "--seed values must be distinct"
        );
        ensure!(self.agents > 0, "--agents must be greater than zero");
        let quorum = self.quorum.unwrap_or(self.agents.min(DEFAULT_QUORUM));
        ensure!((1..=self.agents).contains(&quorum), "--quorum must be between 1 and --agents");
        ensure!(
            self.target.iter().all(|path| workspace::is_safe_relative_path(path)),
            "--target paths must be project-relative paths"
        );
        // Workspaces map project paths by the resolved root, so the root must have no links.
        // `--config-path` resolves already.
        let paths = &mut self.build.project_paths;
        if paths.config_path.is_none() {
            let root = paths.try_project_root()?;
            paths.root = Some(root.canonicalize().wrap_err("failed to resolve project root")?);
        }
        let mut config = self.load_config()?;
        restrict_config(&mut config, self.allow_ffi, self.allow_network);
        let generated_tests =
            workspace::relative_to_root(&config.root, &config.test).join("generated");
        workspace::ensure_safe_relative_path(&generated_tests, "generated test", &config.test)?;
        // Each run writes to directories of its own, so a run never meets the files of another.
        // Creating them reserves them against concurrent runs.
        let generated_dir = config.root.join(&generated_tests);
        ensure_inside(&config.root, &generated_dir, &generated_tests)?;
        fs::create_dir_all(&generated_dir)
            .wrap_err_with(|| format!("failed to create {}", generated_dir.display()))?;
        let mut run = 1;
        let run_dirs = loop {
            let mut dirs = RunDirs(Vec::new());
            for id in 1..=self.agents {
                let dir = generated_dir.join(format!("run-{run}-agent-{id}"));
                match fs::create_dir(&dir) {
                    Ok(()) => dirs.0.push(dir),
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => break,
                    Err(error) => {
                        return Err(error)
                            .wrap_err_with(|| format!("failed to create {}", dir.display()));
                    }
                }
            }
            if dirs.0.len() == self.agents {
                break dirs;
            }
            run += 1;
        };
        let cache_root = config.cache_path.join("properties");
        let verifier_command = self
            .verifier_command
            .as_ref()
            .map(|command| command.canonicalize().wrap_err("failed to resolve verifier command"))
            .transpose()?;
        // Workspaces leave out the project's generated tests, so every agent starts without
        // earlier generated tests.
        let mut sandbox = Sandbox::new(&config, vec![config.root.join(&generated_tests)])?;
        sandbox.end_on_interrupt(run_dirs.0.clone())?;
        if let Some(errors) = sandbox.build()? {
            sh_warn!("the project does not build, so every test run builds it: {errors}")?;
        }
        let campaign = Campaign {
            args: &self,
            config: &config,
            sandbox,
            verifier_command,
            generated_tests,
            run,
            cache_root,
            quorum,
        };
        campaign.run()
    }
}

/// The size and modification time of each project file, by path relative to the root.
type Fingerprint = BTreeMap<PathBuf, (u64, Option<SystemTime>)>;

/// The settings of one `forge properties` run.
struct Campaign<'a> {
    args: &'a PropertiesArgs,
    config: &'a Config,
    sandbox: Sandbox<'a>,
    /// The verifier command, if the run has a verifier.
    verifier_command: Option<PathBuf>,
    /// The directory of generated tests, relative to the project root.
    generated_tests: PathBuf,
    /// The number of this run. Agent `N` writes to `run-<run>-agent-<N>` in `generated_tests`.
    run: usize,
    cache_root: PathBuf,
    quorum: usize,
}

/// Turns off `ffi` unless `allow_ffi`, and removes the RPC and Etherscan settings unless
/// `allow_network`. Every workspace of a [`Sandbox`] writes this config, so no generated test can
/// call `vm.ffi` or reach an endpoint that the config names. Config ignores `FFI` environment
/// variables.
pub(crate) fn restrict_config(config: &mut Config, allow_ffi: bool, allow_network: bool) {
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

/// Builds and runs generated code in temporary copies of the project. Generated code runs outside
/// the sandbox of whatever wrote it, so a copy leaves out hidden root entries such as `.env` and
/// agent settings, and Forge runs in it with only a few environment variables.
pub(crate) struct Sandbox<'a> {
    config: &'a Config,
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

/// The inputs that every agent shares.
struct AgentInputs {
    agent_command: PathBuf,
    /// The target files, relative to the project root.
    targets: Vec<PathBuf>,
    /// The absolute paths of the context files.
    context: Vec<PathBuf>,
}

impl Campaign<'_> {
    fn run(&self) -> Result<()> {
        let agent_command =
            self.args.agent_command.canonicalize().wrap_err("failed to resolve agent command")?;
        let context = self
            .args
            .context
            .iter()
            .map(|path| {
                canonicalize_path(path)
                    .wrap_err_with(|| format!("failed to resolve context {}", path.display()))
            })
            .collect::<Result<Vec<_>>>()?;
        let targets = if self.args.target.is_empty() {
            source_files_iter(&self.config.src, MultiCompilerLanguage::FILE_EXTENSIONS)
                .map(|path| workspace::relative_to_root(&self.config.root, &path))
                .collect()
        } else {
            self.args.target.clone()
        };
        let inputs = AgentInputs { agent_command, targets, context };
        // The prompts, logs, usage, verdicts, and test copies of an earlier run go.
        if self.cache_root.exists() {
            fs::remove_dir_all(&self.cache_root)?;
        }
        // Agents work in copies of the project, but the copies link the entries of dependency
        // directories. A shared file that changes while they run taints the run.
        let fingerprint = project_fingerprint(self.config, &self.sandbox.dependencies());
        // An agent error ends only that agent, and the run reports it after the other agents, so
        // their results are not lost.
        let results = thread::scope(|scope| {
            let inputs = &inputs;
            let agents = (1..=self.args.agents)
                .map(|id| scope.spawn(move || self.run_agent(id, inputs)))
                .collect::<Vec<_>>();
            agents
                .into_iter()
                .map(|agent| agent.join().expect("agent thread panicked"))
                .collect::<Vec<_>>()
        });
        let mut sessions = Vec::new();
        let mut errors = Vec::new();
        for (id, result) in (1..).zip(results) {
            match result {
                Ok(session) => {
                    errors
                        .extend(session.error.as_ref().map(|error| format!("agent {id}: {error}")));
                    sessions.push(session);
                }
                Err(error) => errors.push(format!("agent {id}: {error:#}")),
            }
        }
        self.report(&sessions, &inputs, &fingerprint)?;
        ensure!(errors.is_empty(), "{}", errors.join("\n"));
        Ok(())
    }

    /// Prints the possible bugs that reach the quorum and writes one copy of each kept file to the
    /// project. A verifier runs first, and the project files must match `fingerprint` after it.
    fn report(
        &self,
        sessions: &[Session],
        inputs: &AgentInputs,
        fingerprint: &Fingerprint,
    ) -> Result<()> {
        let failures = sessions
            .iter()
            .flat_map(|session| &session.possible_bugs)
            .map(|failure| AgentFailure {
                agent: failure.agent,
                failure: failure.failure.clone(),
                file: failure.file.clone(),
            })
            .collect::<Vec<_>>();
        // A test that fails only because Forge blocked a cheatcode or its inline config is not
        // evidence of a bug.
        let (blocked, failures) = failures
            .into_iter()
            .partition::<Vec<_>, _>(|failure| BLOCKED.is_match(&failure.failure.reason));
        let (mut possible_bugs, below_quorum) = group_failures(failures)
            .into_iter()
            .partition::<Vec<_>, _>(|group| group.agents.len() >= self.quorum);
        if let Some(command) = &self.verifier_command {
            self.verify_groups(command, &mut possible_bugs, sessions, inputs);
        }
        let after = project_fingerprint(self.config, &self.sandbox.dependencies());
        let tainted = after
            .iter()
            .filter(|(path, metadata)| fingerprint.get(*path) != Some(*metadata))
            .map(|(path, _)| path)
            .chain(fingerprint.keys().filter(|path| !after.contains_key(*path)))
            .cloned()
            .collect::<Vec<_>>();
        let print_group = |label: &str, group: &BugGroup| -> Result<()> {
            let lead = &group.failures[0].failure;
            let summary = lead.claim.as_deref().unwrap_or(&lead.test);
            sh_println!("{label} ({}/{} agents): {summary}", group.agents.len(), self.args.agents)?;
            for failure in &group.failures {
                sh_println!("{failure}")?;
            }
            if let Some(verification) = &group.verification {
                sh_println!("{verification}")?;
            }
            Ok(())
        };
        for group in &possible_bugs {
            let label = match group.verification.as_ref().map(|v| v.judgment.verdict) {
                None => "possible bug",
                Some(Verdict::Confirmed) => "confirmed",
                Some(Verdict::NeedsValidation) => "needs validation",
                Some(Verdict::Rejected) => "rejected",
            };
            print_group(label, group)?;
        }
        let report_path = self.cache_root.join("report.json");
        let report_display = report_path.strip_prefix(&self.config.root).unwrap_or(&report_path);
        // The groups below the quorum hold most of the bugs that agents find, so they follow the
        // possible bugs as a separate list.
        if !below_quorum.is_empty() {
            sh_println!(
                "{} possible bug(s) below the quorum of {}:",
                below_quorum.len(),
                self.quorum
            )?;
            for group in &below_quorum {
                let label = if group.agents.len() == 1 { "single-agent" } else { "below quorum" };
                print_group(label, group)?;
            }
        }
        // An agent's kept files go to the project together or not at all: a path that leads
        // outside the project, or that a concurrent run took, holds back the agent's files. A
        // file that repeats a generated test of the project or of another agent is not written,
        // unless a written file of its agent imports it or it imports one, so every written test
        // compiles against the files that it ran with. Nothing is kept when files that the
        // workspaces share with the project changed while the agents ran.
        let normalize =
            |content: &str| COMMENT.replace_all(content, "").split_whitespace().collect::<String>();
        let mut seen = collect_tests(&self.config.root, &self.generated_tests, 0, &mut vec![])
            .unwrap_or_default()
            .iter()
            .map(|file| normalize(&file.content))
            .collect::<HashSet<_>>();
        let mut held_back = Vec::new();
        let mut kept = Vec::new();
        let mut duplicates = Vec::new();
        for session in sessions.iter().filter(|_| tainted.is_empty()) {
            let conflict = session.kept.iter().find_map(|file| {
                let path = self.config.root.join(&file.path);
                // A linked directory on the path must not lead outside the project.
                if let Err(error) = ensure_inside(&self.config.root, &path, &file.path) {
                    return Some(format!("{error:#}"));
                }
                path.exists().then(|| format!("{} exists already", file.path.display()))
            });
            if let Some(reason) = conflict {
                let paths = session.kept.iter().map(|file| file.path.clone()).collect();
                held_back.push(Rejected { agent: session.id, paths, reason });
                continue;
            }
            let files = &session.kept;
            let imports = |file: &TestFile, other: &TestFile| {
                session.imports.get(&file.path).is_some_and(|i| i.contains(&other.path))
            };
            let mut write =
                files.iter().map(|file| seen.insert(normalize(&file.content))).collect::<Vec<_>>();
            while let Some(index) = (0..files.len()).find(|&i| {
                !write[i]
                    && files.iter().zip(&write).any(|(other, written)| {
                        *written && (imports(other, &files[i]) || imports(&files[i], other))
                    })
            }) {
                write[index] = true;
            }
            for (file, write) in files.iter().zip(write) {
                if write {
                    kept.push(file);
                } else {
                    duplicates.push(&file.path);
                }
            }
        }
        let rejected = sessions
            .iter()
            .flat_map(|session| &session.rejected)
            .chain(&held_back)
            .collect::<Vec<_>>();
        if !rejected.is_empty() {
            sh_println!(
                "{} generated test file(s) did not compile or were not allowed: see {}",
                rejected.iter().map(|rejected| rejected.paths.len()).sum::<usize>(),
                report_display.display()
            )?;
        }
        if !blocked.is_empty() {
            sh_warn!(
                "{} failing test(s) used something that Forge blocks (ffi, environment, network, or inline config), so they are not possible bugs: see {}",
                blocked.len(),
                report_display.display()
            )?;
        }
        let timed_out = sessions
            .iter()
            .filter(|session| session.timed_out)
            .map(|session| session.id)
            .collect::<Vec<_>>();
        if !timed_out.is_empty() {
            sh_warn!(
                "{} agent(s) reached the timeout, so Forge ended them and ran the tests that they wrote",
                timed_out.len()
            )?;
        }
        let usage = sessions
            .iter()
            .filter_map(|session| {
                Some(serde_json::json!({ "agent": session.id, "usage": session.usage? }))
            })
            .collect::<Vec<_>>();
        if let Some(total) = sessions.iter().filter_map(|session| session.usage).reduce(Usage::add)
        {
            sh_println!(
                "agent usage: {} input tokens, {} output tokens, {:.2} USD",
                total.input_tokens,
                total.output_tokens,
                total.cost_usd
            )?;
        }
        if let Some(total) = possible_bugs
            .iter()
            .filter_map(|group| group.verification.as_ref()?.usage)
            .reduce(Usage::add)
        {
            sh_println!(
                "verifier usage: {} input tokens, {} output tokens, {:.2} USD",
                total.input_tokens,
                total.output_tokens,
                total.cost_usd
            )?;
        }

        fs::create_dir_all(&self.cache_root)?;
        let report = serde_json::json!({
            "agents": self.args.agents,
            "quorum": self.quorum,
            "possible_bugs": possible_bugs,
            "below_quorum": below_quorum,
            "blocked": blocked,
            "rejected": rejected,
            "timed_out": timed_out,
            "tainted": tainted,
            "kept": kept.iter().map(|file| &file.path).collect::<Vec<_>>(),
            "duplicates": duplicates,
            "usage": usage,
        });
        fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;

        // Agents can reach the project's dependencies through their copies, so a change can come
        // from an agent, and no result of the run can be trusted.
        ensure!(
            tainted.is_empty(),
            "{} project file(s) changed while the agents ran, so Forge kept no generated test: see {}",
            tainted.len(),
            report_display.display()
        );
        if kept.is_empty() {
            sh_println!("no generated test file was kept")?;
            return Ok(());
        }
        for file in &kept {
            let path = self.config.root.join(&file.path);
            fs::create_dir_all(path.parent().expect("test path has a parent"))?;
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .and_then(|mut target| target.write_all(file.content.as_bytes()))
                .wrap_err_with(|| format!("failed to write {}", file.path.display()))?;
        }
        sh_println!("added {} generated test file(s):", kept.len())?;
        for file in &kept {
            sh_println!("  {}", file.path.display())?;
        }
        if !duplicates.is_empty() {
            sh_println!(
                "skipped {} generated test file(s) that repeat a file of another agent or the project",
                duplicates.len()
            )?;
        }
        Ok(())
    }

    /// Runs one agent session in its own copy of the project, then runs the tests that it wrote.
    fn run_agent(&self, id: usize, inputs: &AgentInputs) -> Result<Session> {
        let args = self.args;
        let cache = self.cache_root.join(format!("agent-{id}"));
        fs::create_dir_all(&cache)?;
        let workspace = self.sandbox.workspace(&[])?;
        let out = self.generated_tests.join(format!("run-{}-agent-{id}", self.run));
        let out_dir = workspace.path().join(&out);
        ensure_inside(workspace.path(), &out_dir, &out)?;
        fs::create_dir_all(&out_dir)?;
        let prompt = cache.join("prompt.md");
        fs::write(&prompt, session_prompt(&inputs.targets, &inputs.context, &out_dir))?;
        let usage_path = cache.join("usage.json");
        let log_path = cache.join("agent.log");
        let log = fs::File::create(&log_path)?;
        // Agents differ by seed.
        let seed: U256 =
            keccak256([args.seed[0].to_be_bytes::<32>(), U256::from(id).to_be_bytes()].concat())
                .into();
        let lines = |paths: &[PathBuf]| {
            paths.iter().map(|path| path.display().to_string()).collect::<Vec<_>>().join("\n")
        };

        let mut command = Command::new(&inputs.agent_command);
        command
            .args(&args.agent_arg)
            .current_dir(workspace.path())
            .env("FORGE_PROPERTIES_PROMPT", &prompt)
            .env("FORGE_PROPERTIES_TARGETS", lines(&inputs.targets))
            .env("FORGE_PROPERTIES_CONTEXT", lines(&inputs.context))
            .env("FORGE_PROPERTIES_OUT", &out_dir)
            .env("FORGE_PROPERTIES_SEED", format!("{seed:#x}"))
            .env("FORGE_PROPERTIES_USAGE", &usage_path)
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log);
        let status = run_session(&mut command, args.timeout, "agent")?;
        let log_display = log_path.strip_prefix(&self.config.root).unwrap_or(&log_path);
        let error = status.filter(|status| !status.success()).map(|status| {
            format!("agent command failed ({status}): see {}", log_display.display())
        });
        let usage = read_usage(&usage_path, &format!("agent {id}"))?;
        let mut session = Session {
            id,
            error,
            timed_out: status.is_none(),
            usage,
            possible_bugs: vec![],
            rejected: vec![],
            kept: vec![],
            files: vec![],
            imports: BTreeMap::new(),
        };
        session.files = collect_tests(workspace.path(), &out, id, &mut session.rejected)?;
        let files = &session.files;
        // Forge writes only passing files to the project, so the copies keep the failing tests.
        let copy = |path: &Path| cache.join("tests").join(path.strip_prefix(&out).unwrap_or(path));
        for file in files {
            let path = copy(&file.path);
            fs::create_dir_all(path.parent().expect("test path has a parent"))?;
            fs::write(&path, &file.content)?;
        }
        if files.is_empty() {
            return Ok(session);
        }

        let workspace = self.sandbox.workspace(files)?;
        let imports = collected_imports(self.config, workspace.path(), files)?;
        let run = match self.sandbox.build_errors(workspace.path())? {
            None => self.run_tests(workspace.path(), files)?,
            // A file that does not compile, such as one that the agent was writing at the
            // timeout, must not hide the other tests. Each file that no other file imports then
            // runs on its own, with the files that it imports. Files that import each other in a
            // cycle have no such file, so the first of them that no group holds yet starts one.
            // A group that does not compile rejects and holds only its first file, so the files
            // that it imports still run without it.
            Some(_) => {
                let no_imports = BTreeSet::new();
                let (roots, others) = files.iter().partition::<Vec<_>, _>(|file| {
                    !imports.values().any(|imported| imported.contains(&file.path))
                });
                let mut grouped = HashSet::new();
                // A file that several groups import runs in each of them, but its results count
                // once.
                let mut ran = HashSet::new();
                let mut run = TestRun::default();
                for root in roots.into_iter().chain(others) {
                    if grouped.contains(&root.path) {
                        continue;
                    }
                    let imported = imports.get(&root.path).unwrap_or(&no_imports);
                    let group = files
                        .iter()
                        .filter(|file| file.path == root.path || imported.contains(&file.path))
                        .cloned()
                        .collect::<Vec<_>>();
                    let workspace = self.sandbox.workspace(&group)?;
                    if let Some(errors) = self.sandbox.build_errors(workspace.path())? {
                        grouped.insert(root.path.clone());
                        let paths = vec![root.path.clone()];
                        session.rejected.push(Rejected { agent: id, paths, reason: errors });
                        continue;
                    }
                    grouped.extend(group.iter().map(|file| file.path.clone()));
                    let group_run = self.run_tests(workspace.path(), &group)?;
                    run.possible_bugs.extend(
                        group_run.possible_bugs.into_iter().filter(|bug| !ran.contains(&bug.path)),
                    );
                    run.passing.extend(
                        group_run
                            .passing
                            .into_iter()
                            .filter(|test| !ran.contains(&test_path(test))),
                    );
                    run.failing_files.extend(group_run.failing_files);
                    run.errors.extend(group_run.errors);
                    ran.extend(group.into_iter().map(|file| file.path));
                }
                run
            }
        };
        if !run.errors.is_empty() {
            let paths = files.iter().map(|file| file.path.clone()).collect();
            session.rejected.push(Rejected { agent: id, paths, reason: run.errors.join("; ") });
        }
        let kept = kept_files(files, &imports, &run)?;
        session.kept = files.iter().filter(|file| kept.contains(&file.path)).cloned().collect();
        session.imports = imports;
        let root = &self.config.root;
        session.possible_bugs = run
            .possible_bugs
            .into_iter()
            .map(|failure| {
                let file = copy(&failure.path);
                let file = file.strip_prefix(root).map(Path::to_path_buf).unwrap_or(file);
                AgentFailure { agent: id, failure, file }
            })
            .collect();
        Ok(session)
    }

    /// Runs every test in the files on every seed.
    fn run_tests(&self, workspace: &Path, files: &[TestFile]) -> Result<TestRun> {
        let paths =
            files.iter().map(|file| file.path.display().to_string()).collect::<Vec<_>>().join(",");
        let mut run = TestRun::default();
        // The passing seeds and the failures of each test.
        let mut outcomes = BTreeMap::<(String, String), (usize, Vec<String>)>::new();
        for seed in &self.args.seed {
            let mut command = self.forge_command(workspace, seed);
            command.args(["test", "--json", "--match-path", &format!("{{{paths}}}")]);
            command.args(SELECT_ALL_TESTS);
            command.arg("--restrict-inline-config");
            if !self.args.allow_network {
                command.arg("--no-network");
            }
            let Some(output) = self.sandbox.output(&mut command)? else {
                run.errors
                    .push(format!("tests reached the timeout of {RUN_TIMEOUT:?} on seed {seed}"));
                continue;
            };
            if !output.status.success() && output.stdout.is_empty() {
                run.errors.push(format!("tests failed on seed {seed}: {}", stderr(&output)));
                continue;
            }
            for (test, status, reason) in test_results(&output)? {
                let (passes, failures) = outcomes.entry((test.contract, test.test)).or_default();
                match status {
                    TestStatus::Success => *passes += 1,
                    TestStatus::Failure => {
                        let reason = reason
                            .filter(|reason| !reason.is_empty())
                            .unwrap_or_else(|| "the test failed without a reason".to_string());
                        failures.push(format!("seed {seed}: {reason}"));
                    }
                    TestStatus::Skipped => {}
                }
            }
        }
        let seeds = self.args.seed.len();
        for ((contract, test), (passes, failures)) in outcomes {
            let test = RerunFailure { contract, test };
            if passes == seeds {
                run.passing.push(test);
                continue;
            }
            let path = test_path(&test);
            run.failing_files.insert(path.clone());
            if failures.len() == seeds {
                let contract = test.contract.rsplit_once(':').map_or("", |(_, name)| name);
                let function =
                    test.test.split_once('(').map_or(test.test.as_str(), |(name, _)| name);
                let claim = files
                    .iter()
                    .find(|file| file.path == path)
                    .and_then(|file| test_claim(&file.content, contract, function));
                run.possible_bugs.push(Failure {
                    test: test_display_name(&test),
                    path,
                    reason: failures.join("; "),
                    claim,
                });
            }
        }
        // Possible bugs with a claim first.
        run.possible_bugs
            .sort_by(|a, b| (a.claim.is_none(), &a.test).cmp(&(b.claim.is_none(), &b.test)));
        Ok(run)
    }

    /// Returns a Forge command for one seed. Each seed persists fuzz and invariant failures in
    /// its own directory, so a counterexample found with one seed is not replayed with another.
    fn forge_command(&self, workspace: &Path, seed: &U256) -> Command {
        let seed_dir = seed_dir(self.config, workspace, seed);
        let mut command = self.sandbox.command(workspace);
        command
            .env("FOUNDRY_FUZZ_SEED", format!("{seed:#x}"))
            .env("FOUNDRY_FUZZ_FAILURE_PERSIST_DIR", seed_dir.join("fuzz"))
            .env("FOUNDRY_INVARIANT_FAILURE_PERSIST_DIR", seed_dir.join("invariant"))
            // Internal runs must not replace the project's own `--rerun` state.
            .env("FOUNDRY_TEST_FAILURES_FILE", seed_dir.join("test-failures"));
        command
    }

    /// Runs one verifier per group, at most `--agents` at a time, and orders the groups by
    /// verdict. A verifier error leaves the group in need of validation.
    fn verify_groups(
        &self,
        command: &Path,
        groups: &mut [BugGroup],
        sessions: &[Session],
        inputs: &AgentInputs,
    ) {
        let next = AtomicUsize::new(0);
        let verifications = {
            let groups = &*groups;
            thread::scope(|scope| {
                let workers = (0..self.args.agents.min(groups.len()))
                    .map(|_| {
                        scope.spawn(|| {
                            let mut done = Vec::new();
                            loop {
                                let index = next.fetch_add(1, Ordering::Relaxed);
                                let Some(group) = groups.get(index) else { break };
                                let verification = self
                                    .verify(command, index + 1, group, sessions, inputs)
                                    .unwrap_or_else(|error| {
                                        Verification::failed(format!("{error:#}"))
                                    });
                                done.push((index, verification));
                            }
                            done
                        })
                    })
                    .collect::<Vec<_>>();
                workers
                    .into_iter()
                    .flat_map(|worker| worker.join().expect("verifier thread panicked"))
                    .collect::<Vec<_>>()
            })
        };
        for (index, verification) in verifications {
            groups[index].verification = Some(verification);
        }
        // The sort is stable, so groups with one verdict keep their order.
        groups.sort_by_key(|group| group.verification.as_ref().map(|v| v.judgment.verdict));
    }

    /// Runs the verifier on one group in a fresh workspace that holds the tests of the group, then
    /// reads its verdict.
    fn verify(
        &self,
        command: &Path,
        id: usize,
        group: &BugGroup,
        sessions: &[Session],
        inputs: &AgentInputs,
    ) -> Result<Verification> {
        let cache = self.cache_root.join(format!("verify-{id}"));
        fs::create_dir_all(&cache)?;
        let workspace = self.sandbox.workspace(&group_files(sessions, group))?;
        let prompt_path = cache.join("prompt.md");
        let group_path = cache.join("group.json");
        let verdict_path = cache.join("verdict.json");
        let usage_path = cache.join("usage.json");
        let log_path = cache.join("verifier.log");
        fs::write(&prompt_path, verify_prompt(group, &inputs.targets, &group_path, &verdict_path))?;
        let failures = group.failures.iter().map(|failure| {
            serde_json::json!({
                "agent": failure.agent,
                "test": failure.failure.test,
                "path": failure.failure.path,
                "claim": failure.failure.claim,
                "failure": failure.failure.reason,
            })
        });
        let group_json = serde_json::json!({
            "targets": inputs.targets,
            "agents": group.agents,
            "failures": failures.collect::<Vec<_>>(),
        });
        fs::write(&group_path, serde_json::to_vec_pretty(&group_json)?)?;
        let log = fs::File::create(&log_path)?;

        let mut command = Command::new(command);
        command
            .args(&self.args.verifier_arg)
            .current_dir(workspace.path())
            .env("FORGE_PROPERTIES_VERIFY_PROMPT", &prompt_path)
            .env("FORGE_PROPERTIES_GROUP", &group_path)
            .env("FORGE_PROPERTIES_VERDICT", &verdict_path)
            .env("FORGE_PROPERTIES_USAGE", &usage_path)
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log);
        let status = run_session(&mut command, self.args.timeout, "verifier")?;
        let usage = read_usage(&usage_path, &format!("verifier {id}"))?;

        let mut errors = Vec::new();
        match status {
            None => errors.push("the verifier reached the timeout".to_string()),
            Some(status) if !status.success() => {
                errors.push(format!("verifier command failed ({status})"))
            }
            Some(_) => {}
        }
        let judgment = match fs::read(&verdict_path) {
            Ok(verdict) => match serde_json::from_slice::<Judgment>(&verdict) {
                // A disproof needs a quote that the group can check.
                Ok(judgment)
                    if judgment.verdict == Verdict::Rejected && judgment.evidence.is_empty() =>
                {
                    errors.push("the verifier rejected the group without evidence".to_string());
                    Judgment { verdict: Verdict::NeedsValidation, ..judgment }
                }
                Ok(judgment) => judgment,
                Err(error) => {
                    errors.push(format!("the verifier wrote an invalid verdict: {error}"));
                    Judgment::default()
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                errors.push("the verifier wrote no verdict".to_string());
                Judgment::default()
            }
            Err(error) => return Err(error).wrap_err("failed to read verifier verdict"),
        };
        let log_display = log_path.strip_prefix(&self.config.root).unwrap_or(&log_path);
        let error = (!errors.is_empty())
            .then(|| format!("{}: see {}", errors.join("; "), log_display.display()));
        Ok(Verification { judgment, error, usage })
    }
}

impl<'a> Sandbox<'a> {
    /// Returns a sandbox for the project of `config`, whose workspaces leave out the `skipped`
    /// project paths.
    pub(crate) fn new(config: &'a Config, skipped: Vec<PathBuf>) -> Result<Self> {
        let forge = std::env::current_exe().wrap_err("failed to resolve Forge executable")?;
        let root = tempfile::Builder::new().prefix("forge-properties-").tempdir()?;
        let home = root.path().join("home");
        fs::create_dir(&home)?;
        if let Some(svm) = Solc::svm_home().filter(|svm| svm.is_dir()) {
            workspace::symlink_entry(&svm, &home.join(".svm"), fs::metadata(&svm)?.file_type())?;
        }
        Ok(Self { config, forge, skipped, build_cache: None, home, root })
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
    fn dependencies(&self) -> Vec<PathBuf> {
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

/// Groups the failing tests of all agents by root cause with the root-cause matcher. Groups with a
/// claim come first, then groups with more agents.
fn group_failures(failures: Vec<AgentFailure>) -> Vec<BugGroup> {
    let claims =
        failures.iter().map(|failure| failure.failure.claim.as_deref()).collect::<Vec<_>>();
    let group_of = matcher::root_cause_groups(&claims);
    let mut groups = BTreeMap::<usize, Vec<AgentFailure>>::new();
    for (failure, group) in failures.into_iter().zip(group_of) {
        groups.entry(group).or_default().push(failure);
    }
    let mut groups = groups
        .into_values()
        .map(|mut failures| {
            // A failure with a claim leads, then the failures by agent.
            failures.sort_by(|a, b| {
                let key = |failure: &AgentFailure| {
                    (failure.failure.claim.is_none(), failure.agent, failure.failure.test.clone())
                };
                key(a).cmp(&key(b))
            });
            let agents = failures.iter().map(|failure| failure.agent).collect::<BTreeSet<_>>();
            BugGroup { agents: agents.into_iter().collect(), failures, verification: None }
        })
        .collect::<Vec<_>>();
    groups.sort_by(|a, b| {
        let key = |group: &BugGroup| {
            let lead = &group.failures[0].failure;
            (lead.claim.is_none(), Reverse(group.agents.len()), lead.test.clone())
        };
        key(a).cmp(&key(b))
    });
    groups
}

/// Returns the task of an agent session. On 26 held-out cases, plain independent small-model
/// audits with a task of this kind found 0.05 to 0.10 more planted bugs at every spend level than
/// agents that returned candidate JSON in short rounds.
fn session_prompt(targets: &[PathBuf], context: &[PathBuf], out: &Path) -> String {
    let list = |paths: &[PathBuf]| {
        paths.iter().map(|path| format!("- {}\n", path.display())).collect::<String>()
    };
    let mut prompt = format!(
        "Find bugs in these targets of the Foundry project in the current directory:\n{}",
        list(targets)
    );
    if !context.is_empty() {
        prompt.push_str(&format!(
            "\nThese context files state intended behavior:\n{}",
            list(context)
        ));
    }
    prompt.push_str(&format!(
        "\nProve each bug with a Foundry test in {} that fails because of the bug and encodes the \
         intended behavior. The code under test can be wrong: the intended behavior is what its \
         documentation, its interfaces, and the standards it claims state. Name each test after \
         the bug. Right above each test, write exactly one comment line in this form:\n\n\
         // @bug <Contract>.<function> <file>:<line> — <claim>\n\n\
         <Contract>.<function> is the target function that has the bug, <file>:<line> is the \
         target source line that breaks the intended behavior, and <claim> is one sentence. Use \
         `//`, not `///`: the compiler rejects `@bug` as a NatSpec tag. You may run `forge test` \
         freely.\n",
        out.display()
    ));
    prompt
}

/// Returns the Solidity files under the output directory `out` of a workspace, by path relative
/// to the workspace, in path order. Links are not followed, and `out` itself must not be one. A
/// file whose path has a glob character is rejected: Forge selects the files to run by a glob.
fn collect_tests(
    workspace: &Path,
    out: &Path,
    agent: usize,
    rejected: &mut Vec<Rejected>,
) -> Result<Vec<TestFile>> {
    let out_dir = workspace.join(out);
    ensure_inside(workspace, &out_dir, out)?;
    match fs::symlink_metadata(&out_dir) {
        Ok(metadata) => ensure!(
            metadata.is_dir(),
            "the generated test directory {} is not a directory",
            out.display()
        ),
        // The agent removed it.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(error) => {
            return Err(error).wrap_err_with(|| format!("failed to read {}", out.display()));
        }
    }
    let mut files = Vec::new();
    let mut pending = vec![out.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(workspace.join(&dir))? {
            let entry = entry?;
            let path = dir.join(entry.file_name());
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                pending.push(path);
                continue;
            }
            if !file_type.is_file() || path.extension().is_none_or(|extension| extension != "sol") {
                continue;
            }
            let glob = ['*', '?', '[', ']', '{', '}', ',', '\\'];
            if path.iter().any(|component| component.to_string_lossy().contains(glob)) {
                let reason = "the path has a glob character".to_string();
                rejected.push(Rejected { agent, paths: vec![path], reason });
                continue;
            }
            match fs::read_to_string(entry.path()) {
                Ok(content) => files.push(TestFile { path, content }),
                Err(error) => {
                    let reason = error.to_string();
                    rejected.push(Rejected { agent, paths: vec![path], reason });
                }
            }
        }
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

/// Returns the comment right above a test function, without comment markers, as one line. The
/// function is the first one with this name after the contract declaration and before the next
/// one. Declarations are searched outside comments, so a claim such as `the contract pays twice`
/// is not a declaration.
fn test_claim(source: &str, contract: &str, function: &str) -> Option<String> {
    // Blanking each comment byte keeps the offsets of the code.
    let code = COMMENT.replace_all(source, |comment: &regex::Captures<'_>| {
        comment[0].bytes().map(|byte| if byte == b'\n' { '\n' } else { ' ' }).collect::<String>()
    });
    let mut contracts = CONTRACT.captures_iter(&code);
    let start = contracts.find(|captures| &captures[1] == contract)?.get(0)?.end();
    let end = contracts.next().map_or(code.len(), |captures| captures.get(0).unwrap().start());
    let declaration = Regex::new(&format!(r"\bfunction\s+{}\s*\(", regex::escape(function)))
        .ok()?
        .find(&code[start..end])?;
    let line_start = source[..start + declaration.start()].rfind('\n').map_or(0, |i| i + 1);
    let mut lines = source[..line_start]
        .lines()
        .rev()
        .map(str::trim)
        .take_while(|line| {
            line.starts_with("//") || line.starts_with('*') || line.starts_with("/*")
        })
        .filter(|line| !line.contains("forge-config:"))
        .map(|line| line.trim_start_matches(['/', '*']).trim_end_matches(['*', '/']).trim())
        .map(|line| {
            match line.strip_prefix('@').map(|tagged| tagged.split_once(char::is_whitespace)) {
                // The `@bug` tag names the target, so it stays. A NatSpec tag such as `@notice` is
                // not part of the claim. The compiler rejects `/// @bug`, so
                // `@custom:bug` stays too.
                Some(Some((tag, text))) if !matches!(tag, "bug" | "custom:bug") => text,
                Some(None) => "",
                _ => line,
            }
        })
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    lines.reverse();
    // The claim starts at the tag, wherever the comment has it.
    if let Some(tag) = lines.iter().position(|line| {
        line.split_whitespace().next().is_some_and(|tag| matches!(tag, "@bug" | "@custom:bug"))
    }) {
        lines.drain(..tag);
    }
    (!lines.is_empty()).then(|| lines.join(" "))
}

/// Returns the collected files that each collected file imports, directly or not.
fn collected_imports(
    config: &Config,
    workspace: &Path,
    files: &[TestFile],
) -> Result<BTreeMap<PathBuf, BTreeSet<PathBuf>>> {
    let rebased = workspace::rebase_config_paths(config, workspace);
    let graph =
        Graph::<MultiCompilerParser>::resolve(&rebased.project_paths::<MultiCompilerLanguage>())
            .wrap_err("failed to resolve generated test imports")?;
    let paths = files.iter().map(|file| file.path.clone()).collect::<HashSet<_>>();
    let mut imports = BTreeMap::new();
    for (index, node) in graph.nodes.iter().enumerate() {
        let path = workspace::relative_to_root(workspace, node.path());
        if paths.contains(&path) {
            let imported = graph
                .all_imported_nodes(index)
                .map(|index| workspace::relative_to_root(workspace, graph.node(index).path()))
                .filter(|path| paths.contains(path))
                .collect();
            imports.insert(path, imported);
        }
    }
    Ok(imports)
}

/// Returns the files whose tests all pass on every seed, with the files that they import. A
/// passing file that imports a file with a failing test is not kept.
fn kept_files(
    files: &[TestFile],
    imports: &BTreeMap<PathBuf, BTreeSet<PathBuf>>,
    run: &TestRun,
) -> Result<HashSet<PathBuf>> {
    let passing = files
        .iter()
        .map(|file| &file.path)
        .filter(|path| {
            !run.failing_files.contains(*path)
                && run.passing.iter().any(|test| test_path(test) == **path)
        })
        .collect::<Vec<_>>();
    if passing.is_empty() {
        return Ok(HashSet::new());
    }
    if passing.len() == files.len() {
        return Ok(passing.into_iter().cloned().collect());
    }
    let mut kept = HashSet::new();
    for path in passing {
        let imported = imports.get(path).ok_or_else(|| {
            eyre!("passing test file {} is absent from the import graph", path.display())
        })?;
        if imported.iter().all(|path| !run.failing_files.contains(path)) {
            kept.insert(path.clone());
            kept.extend(imported.iter().cloned());
        }
    }
    Ok(kept)
}

/// Fails if the nearest existing ancestor of `path` resolves outside `root`, for example through a
/// linked directory.
fn ensure_inside(root: &Path, path: &Path, orig: &Path) -> Result<()> {
    match path.ancestors().find(|ancestor| ancestor.exists()) {
        Some(existing) => workspace::ensure_within_root(root, existing, "generated test", orig),
        None => Ok(()),
    }
}

/// Returns the size and modification time of each file that workspaces share with the project, by
/// path relative to the root: the files in the `dependencies` directories, whose entries workspaces
/// link, and behind links that lead outside the project. Workspaces copy every other file, so a
/// change to one cannot come from an agent. Hidden entries and the build and cache directories are
/// skipped. Each directory is read once.
fn project_fingerprint(config: &Config, dependencies: &[PathBuf]) -> Fingerprint {
    let mut files = BTreeMap::new();
    let mut read = HashSet::new();
    // Each directory, and whether workspaces share its files.
    let mut pending = vec![(config.root.clone(), false)];
    while let Some((dir, shared)) = pending.pop() {
        if !fs::canonicalize(&dir).is_ok_and(|dir| read.insert(dir)) {
            continue;
        }
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if entry.file_name().to_string_lossy().starts_with('.')
                || path == config.out
                || path == config.cache_path
            {
                continue;
            }
            let shared = shared
                || dependencies.contains(&path)
                || (entry.file_type().is_ok_and(|file_type| file_type.is_symlink())
                    && fs::canonicalize(&path)
                        .is_ok_and(|target| !target.starts_with(&config.root)));
            let Ok(metadata) = (if shared { fs::metadata(&path) } else { entry.metadata() }) else {
                continue;
            };
            if metadata.is_dir() {
                pending.push((path, shared));
            } else if shared && metadata.is_file() {
                // The path through a link, not the link's target.
                let path = path.strip_prefix(&config.root).unwrap_or(&path).to_path_buf();
                files.insert(path, (metadata.len(), metadata.modified().ok()));
            }
        }
    }
    files
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

/// Returns every test result in Forge's JSON output, with one result per invariant predicate of a
/// merged invariant campaign.
fn test_results(output: &Output) -> Result<Vec<(RerunFailure, TestStatus, Option<String>)>> {
    let mut results = Vec::new();
    for (contract, test, result) in suite_results(output)? {
        let key = |name: &str| RerunFailure { contract: contract.clone(), test: name.into() };
        // Handler assertion failures belong to the whole campaign, not to one predicate.
        let handler_failures = result
            .invariant_handler_failures
            .iter()
            .chain(result.invariant_failures.iter().filter(|f| f.predicate_name().is_none()))
            .map(|failure| {
                format!(
                    "handler {}: {}",
                    failure.name(),
                    failure_reason(failure.reason(), failure.counterexample())
                )
            })
            .collect::<Vec<_>>();
        if result.invariant_predicate_results.is_empty() {
            let reason = result.reason.clone().or_else(|| {
                let reasons = result
                    .invariant_failures
                    .iter()
                    .filter(|failure| failure.predicate_name().is_some())
                    .map(|failure| failure_reason(failure.reason(), failure.counterexample()))
                    .chain(handler_failures.iter().cloned())
                    .collect::<Vec<_>>();
                (!reasons.is_empty()).then(|| reasons.join("; "))
            });
            let reason = match (reason, result.counterexample.as_ref()) {
                (Some(reason), counterexample) => Some(failure_reason(&reason, counterexample)),
                (None, Some(counterexample)) => Some(failure_reason("", Some(counterexample))),
                (None, None) => None,
            };
            let status =
                if handler_failures.is_empty() { result.status } else { TestStatus::Failure };
            results.push((key(&test), status, reason));
            continue;
        }
        for predicate in &result.invariant_predicate_results {
            let failure = result
                .invariant_failures
                .iter()
                .find(|failure| failure.predicate_name() == Some(predicate.name.as_str()));
            let reason = predicate.reason.as_deref().or(failure.map(|failure| failure.reason()));
            let reason = reason.map(|reason| {
                failure_reason(reason, failure.and_then(|failure| failure.counterexample()))
            });
            results.push((key(&predicate.name), predicate.status, reason));
        }
        // One campaign result per seed, so the count of failing seeds stays correct.
        if !handler_failures.is_empty() {
            let reason = Some(handler_failures.join("; "));
            results.push((key(&test), TestStatus::Failure, reason));
        }
    }
    Ok(results)
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

/// Appends the counterexample calls to a failure reason.
fn failure_reason(reason: &str, counterexample: Option<&CounterExample>) -> String {
    let Some(counterexample) = counterexample else { return reason.to_string() };
    let calls = match counterexample {
        CounterExample::Single(call) => vec![call],
        CounterExample::Sequence(_, calls) => calls.iter().collect(),
    };
    let calls = calls.iter().map(|call| call.to_string().trim().to_string()).collect::<Vec<_>>();
    let counterexample = format!("counterexample: {}", calls.join(", "));
    if reason.is_empty() { counterexample } else { format!("{reason}; {counterexample}") }
}

/// Returns `Contract::test` for a test identity.
fn test_display_name(test: &RerunFailure) -> String {
    let contract = test.contract.rsplit_once(':').map_or(test.contract.as_str(), |(_, name)| name);
    let name = test.test.split_once('(').map_or(test.test.as_str(), |(name, _)| name);
    format!("{contract}::{name}")
}

/// Returns the file of a test identity.
fn test_path(test: &RerunFailure) -> PathBuf {
    PathBuf::from(test.contract.rsplit_once(':').map_or(test.contract.as_str(), |(path, _)| path))
}

/// Returns the directory for one seed's builds and persisted state in a workspace.
fn seed_dir(config: &Config, workspace: &Path, seed: &U256) -> PathBuf {
    workspace
        .join(workspace::relative_to_root(&config.root, &config.cache_path))
        .join("properties/seeds")
        .join(format!("{seed:#x}"))
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

/// Returns the task of a verifier session.
fn verify_prompt(
    group: &BugGroup,
    targets: &[PathBuf],
    group_path: &Path,
    verdict: &Path,
) -> String {
    let targets = targets.iter().map(|path| format!("- {}\n", path.display())).collect::<String>();
    let members = group
        .failures
        .iter()
        .map(|failure| {
            let claim = failure.failure.claim.as_deref().unwrap_or("(none)");
            let reason = failure.failure.reason.trim().chars().take(600).collect::<String>();
            format!(
                "- file: {}\n  test: {}\n  claim: {claim}\n  failure: {reason}\n",
                failure.failure.path.display(),
                failure.failure.test
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    // The tests come last, so a placeholder in a failure reason stays as it is.
    VERIFY_PROMPT
        .replace("{targets}", &targets)
        .replace("{group}", &group_path.display().to_string())
        .replace("{verdict}", &verdict.display().to_string())
        .replace("{members}", &members)
}

/// Returns the files of a group: the files of its failing tests, with the files of the same agent
/// that they import, directly or not.
fn group_files(sessions: &[Session], group: &BugGroup) -> Vec<TestFile> {
    let mut files = Vec::new();
    for session in sessions.iter().filter(|session| group.agents.contains(&session.id)) {
        let failing = group
            .failures
            .iter()
            .filter(|failure| failure.agent == session.id)
            .map(|failure| &failure.failure.path)
            .collect::<BTreeSet<_>>();
        let paths = failing
            .iter()
            .copied()
            .chain(failing.iter().filter_map(|path| session.imports.get(*path)).flatten())
            .collect::<BTreeSet<_>>();
        files.extend(session.files.iter().filter(|file| paths.contains(&file.path)).cloned());
    }
    files
}

/// Runs a session command in its own process group until it exits or reaches the timeout in
/// seconds, then ends the process group, so no process that the session started keeps running.
/// Returns `None` if the session reached the timeout.
fn run_session(
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

/// Reads the usage file of an agent or verifier. A missing file is no usage, and an invalid one
/// is a warning.
fn read_usage(path: &Path, session: &str) -> Result<Option<Usage>> {
    match fs::read(path) {
        Ok(usage) => Ok(serde_json::from_slice::<Usage>(&usage)
            .inspect_err(|error| {
                let _ = sh_warn!("{session} wrote invalid usage JSON: {error}");
            })
            .ok()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).wrap_err_with(|| format!("failed to read {session} usage")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failure(agent: usize, test: &str, claim: &str) -> AgentFailure {
        AgentFailure {
            agent,
            failure: Failure {
                test: test.to_string(),
                path: PathBuf::from("test/generated/Fee.t.sol"),
                reason: "revert".to_string(),
                claim: (!claim.is_empty()).then(|| claim.to_string()),
            },
            file: PathBuf::new(),
        }
    }

    // Agreeing failures form one group with the set of their agents. Groups with a claim and more
    // agents come first; a failure without a claim agrees with none.
    #[test]
    fn groups_link_agreeing_failures_by_agent() {
        let groups = group_failures(vec![
            failure(3, "C::zero", "A zero amount is charged; src/Fee.sol:11 charges()."),
            failure(1, "A::cap", "The fee is capped at 100 ether; src/Fee.sol:7 fee()."),
            failure(2, "B::cap", "Large amounts pay the maximum; src/Fee.sol:7 fee()."),
            failure(2, "B::limit", "Large amounts pay the maximum fee (src/Fee.sol:8)."),
            failure(1, "D::none", ""),
        ]);
        let groups = groups
            .iter()
            .map(|group| {
                let tests = group.failures.iter().map(|failure| failure.failure.test.as_str());
                (group.agents.clone(), tests.collect::<Vec<_>>())
            })
            .collect::<Vec<_>>();
        assert_eq!(
            groups,
            [
                (vec![1, 2], vec!["A::cap", "B::cap"]),
                // It agrees with `B::cap` but not with `A::cap`, so it does not join their group.
                (vec![2], vec!["B::limit"]),
                (vec![3], vec!["C::zero"]),
                (vec![1], vec!["D::none"]),
            ]
        );
    }

    // The claim is the comment right above the test function of the named contract, without
    // comment markers, NatSpec tags other than the bug tag, or inline config. A test without such a
    // comment has none, and the word `contract` in a comment declares no contract.
    #[test]
    fn test_claim_reads_the_comment_above_the_test() {
        let source = r#"
pragma solidity ^0.8.20;
contract FeeTest {
    // The fee is capped at 100 ether (src/Fee.sol:7).
    function testCap() public {}

    /// @notice A zero amount is charged,
    /// src/Fee.sol:11.
    /// forge-config: default.fuzz.runs = 10
    function testZero(uint256 amount) public {}

    function testBare() public {}

    // @bug Fee.fee src/Fee.sol:7 — the cap is 10 ether.
    function testTagged() public {}

    /// @custom:bug Fee.fee src/Fee.sol:7 — the cap is 10 ether.
    function testCustomTag() public {}

    /// @notice Large amounts.
    /// @custom:bug Fee.fee src/Fee.sol:7 — the cap is 10 ether,
    /// not 100 ether.
    function testLateTag() public {}

    // @bug Fee.pay src/Fee.sol:20 — the contract pays twice.
    function testPaysTwice() public {}
}
contract OtherTest {
    /* Rounds down: src/Fee.sol:6 */
    function testCap() public {}
}
"#;
        assert_eq!(
            test_claim(source, "FeeTest", "testCap").as_deref(),
            Some("The fee is capped at 100 ether (src/Fee.sol:7).")
        );
        assert_eq!(
            test_claim(source, "FeeTest", "testZero").as_deref(),
            Some("A zero amount is charged, src/Fee.sol:11.")
        );
        assert_eq!(test_claim(source, "FeeTest", "testBare"), None);
        assert_eq!(
            test_claim(source, "FeeTest", "testTagged").as_deref(),
            Some("@bug Fee.fee src/Fee.sol:7 — the cap is 10 ether.")
        );
        assert_eq!(
            test_claim(source, "FeeTest", "testCustomTag").as_deref(),
            Some("@custom:bug Fee.fee src/Fee.sol:7 — the cap is 10 ether.")
        );
        assert_eq!(
            test_claim(source, "FeeTest", "testLateTag").as_deref(),
            Some("@custom:bug Fee.fee src/Fee.sol:7 — the cap is 10 ether, not 100 ether.")
        );
        assert_eq!(
            test_claim(source, "FeeTest", "testPaysTwice").as_deref(),
            Some("@bug Fee.pay src/Fee.sol:20 — the contract pays twice.")
        );
        assert_eq!(
            test_claim(source, "OtherTest", "testCap").as_deref(),
            Some("Rounds down: src/Fee.sol:6")
        );
        assert_eq!(test_claim(source, "OtherTest", "testZero"), None);
        assert_eq!(test_claim(source, "MissingTest", "testCap"), None);
    }

    // Forge collects every Solidity file under the output directory, also in subdirectories, in
    // path order. It skips other files and links, and rejects files that set a seed, `ffi`, or
    // `fs_permissions` in inline config.
    #[test]
    fn collect_tests_reads_solidity_files_under_the_output_directory() {
        let workspace = tempfile::tempdir().unwrap();
        let out = Path::new("test/generated/agent-1");
        let write = |path: &str, content: &str| {
            let path = workspace.path().join(out).join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, content).unwrap();
        };
        write("Fee.t.sol", "contract FeeTest {}");
        write("helpers/Helper.sol", "library Helper {}");
        write("notes.md", "notes");
        write("A{B,C}.t.sol", "contract GlobTest {}");
        write("dir*/Star.t.sol", "contract StarTest {}");
        #[cfg(unix)]
        std::os::unix::fs::symlink(
            workspace.path().join(out).join("Fee.t.sol"),
            workspace.path().join(out).join("Link.t.sol"),
        )
        .unwrap();

        let mut rejected = Vec::new();
        let files = collect_tests(workspace.path(), out, 1, &mut rejected).unwrap();
        assert_eq!(
            files
                .iter()
                .map(|file| (file.path.to_str().unwrap(), file.content.as_str()))
                .collect::<Vec<_>>(),
            [
                ("test/generated/agent-1/Fee.t.sol", "contract FeeTest {}"),
                ("test/generated/agent-1/helpers/Helper.sol", "library Helper {}"),
            ]
        );
        rejected.sort_by(|a, b| a.paths.cmp(&b.paths));
        assert_eq!(
            rejected
                .iter()
                .map(|rejected| format!("{} {}", rejected.paths[0].display(), rejected.reason))
                .collect::<Vec<_>>(),
            [
                "test/generated/agent-1/A{B,C}.t.sol the path has a glob character",
                "test/generated/agent-1/dir*/Star.t.sol the path has a glob character",
            ]
        );

        // The output directory itself must not be a link.
        #[cfg(unix)]
        {
            let linked = Path::new("test/generated/agent-2");
            std::os::unix::fs::symlink(workspace.path().join(out), workspace.path().join(linked))
                .unwrap();
            let error = collect_tests(workspace.path(), linked, 2, &mut rejected).unwrap_err();
            assert!(error.to_string().contains("is not a directory"), "{error}");
        }
    }
}
