//! The `forge properties` command: several external generator agents audit the targets
//! independently and in parallel, and write their documented properties as tests. Forge reports a
//! test that fails on every seed as a possible bug only when enough agents agree on its root cause,
//! and keeps one copy of each passing test file.

use crate::{
    cmd::test::RerunFailure,
    result::{TestResult, TestStatus},
    workspace,
};
use alloy_primitives::{U256, keccak256};
use clap::Parser;
use eyre::{Context, Result, ensure, eyre};
use foundry_common::{sh_println, sh_warn};
use foundry_compilers::{
    Graph,
    compilers::{
        Language,
        multi::{MultiCompilerLanguage, MultiCompilerParser},
    },
    utils::source_files_iter,
};
use foundry_config::Config;
use foundry_evm::fuzz::CounterExample;
use oracle::{CallClaim, CallKey, CallOutcome};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    ffi::OsString,
    fs,
    io::Write,
    ops::Add,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::LazyLock,
    thread,
    time::SystemTime,
};
use tempfile::TempDir;

mod oracle;

const CANDIDATE_SCHEMA: &str = "foundry/properties-candidate-v1";
const PROMPT_SCHEMA: &str = "foundry/properties-prompt-v4";
/// On 12 held-out cases, five small-model agents with a quorum of two found bugs in 9 cases, one
/// larger model in 8, at about the same cost. The quorum lost no confirmed bug and removed most
/// wrong reports, because wrong claims of independent agents rarely repeat.
const DEFAULT_AGENTS: usize = 5;
const DEFAULT_QUORUM: usize = 2;
/// Two claims agree when at least 3/10 of their combined words are shared, the threshold of the
/// offline analysis.
const CLAIM_OVERLAP: (usize, usize) = (3, 10);
/// Words that do not tell two claims apart.
const STOP_WORDS: [&str; 31] = [
    "the", "a", "an", "of", "to", "is", "in", "on", "and", "or", "must", "be", "it", "that",
    "with", "for", "not", "when", "as", "by", "at", "from", "than", "its", "this", "are", "which",
    "so", "any", "if", "no",
];
// Bound each source section so it cannot dominate the prompt. Targets hold the documentation that
// defines correct behavior, so they get the largest share; 64 KiB holds most contracts in full.
const MAX_PROMPT_TARGET_BYTES: usize = 64 * 1024;
// Interfaces and base contracts often hold the NatSpec of the functions that a target implements.
const MAX_PROMPT_INHERITED_BYTES: usize = 32 * 1024;
const MAX_PROMPT_SOURCE_BYTES: usize = 16 * 1024;
const MAX_PROMPT_TEST_FILES: usize = 2;
/// The test names listed per target function in `existing_tests`. More are counted, not named, so
/// the list stays small for large suites.
const MAX_EXISTING_TESTS_PER_FUNCTION: usize = 5;
/// Extra rounds for an agent whose last round left no kept file and no possible bug. In the swarm
/// study, 8 small-model reports named a real root cause, but their tests did not compile or run.
const MAX_REPAIR_ROUNDS: usize = 1;
/// Records every test that fails on every seed in a `--check` run, one JSON object per line.
const CHECK_JOURNAL: &str = "check-failures.jsonl";
/// Records the calls of the test contract in Forge's JSON output, for the oracle check.
const CALL_TRACES: [&str; 3] = ["-vvvv", "--trace-depth", "1"];
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

/// Solidity line and block comments.
static COMMENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)//[^\n]*|/\*.*?\*/").unwrap());
/// A contract, interface, or library declaration and its name.
static DECLARATION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?:contract|interface|library)\s+([A-Za-z_$][A-Za-z0-9_$]*)").unwrap()
});
/// A contract or interface declaration and its list of bases.
static INHERITANCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?:contract|interface)\s+[A-Za-z_$][A-Za-z0-9_$]*\s+is\s+([^{]+)\{").unwrap()
});
/// A Solidity file path.
static SOLIDITY_FILE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\w./-]+\.sol").unwrap());
/// A function name followed by its argument list.
static FUNCTION_CALL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b[A-Za-z_][A-Za-z0-9_]*\(").unwrap());
/// A camelCase identifier.
static CAMEL_CASE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b_?[a-z][A-Za-z0-9]*[A-Z][A-Za-z0-9]*\b").unwrap());
/// A lowercase word.
static WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[a-z0-9_]+").unwrap());
/// A function declaration that is not internal by convention, and its name.
static FUNCTION_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bfunction\s+([A-Za-z$][A-Za-z0-9_$]*)\s*\(").unwrap());
/// A test or invariant function declaration and its name.
static TEST_FUNCTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\bfunction\s+((?:test|invariant|statefulFuzz)[A-Za-z0-9_$]*)\s*\(").unwrap()
});
/// The path of an import directive.
static IMPORT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\bimport\s+(?:[^;]*?\bfrom\s+)?["']([^"']+)["']"#).unwrap());
/// The protocol steps, one per bullet under `## Protocol` in `guidance.md`.
static PROTOCOL: LazyLock<Vec<&'static str>> = LazyLock::new(|| guidance_section("## Protocol"));
/// The audit lenses, one per bullet under `## Lenses` in `guidance.md`, each named before its
/// colon.
static LENSES: LazyLock<Vec<&'static str>> = LazyLock::new(|| guidance_section("## Lenses"));
/// The generator rules, one per bullet under `## Rules` in `guidance.md`.
static GUIDANCE: LazyLock<Vec<&'static str>> = LazyLock::new(|| guidance_section("## Rules"));

/// Write documented properties of the targets as tests and report failing ones as possible bugs.
#[derive(Clone, Debug, Parser)]
pub struct PropertiesArgs {
    /// Root of the Foundry project.
    #[arg(long, default_value = ".", value_name = "PATH")]
    root: PathBuf,

    /// Source file to write properties for. Can be given more than once.
    ///
    /// Defaults to every source file in the project's source directory.
    #[arg(long, value_name = "PATH")]
    target: Vec<PathBuf>,

    /// Markdown instructions supplied to the property generator.
    #[arg(long, required_unless_present = "check", value_name = "PATH")]
    brief: Option<PathBuf>,

    /// Extra reference material for the generator, such as a threat model or bug-class notes.
    ///
    /// Each file is added to the prompt with its path. Can be given more than once.
    #[arg(long, value_name = "PATH")]
    context: Vec<PathBuf>,

    /// Executable that writes the tests of one agent, such as a wrapper around any coding agent.
    ///
    /// Forge runs `PATH [ARG...] PROMPT_JSON OUTPUT_JSON` once per agent and round, in the
    /// agent's copy of the project. PROMPT_JSON holds the brief, the instructions, the sources,
    /// and `output_contract`. The generator writes one candidate object with the schema of
    /// `output_contract.example` to OUTPUT_JSON and exits with status 0; another status ends the
    /// agent. It may run `output_contract.check_command` any number of times before it returns.
    #[arg(long, required_unless_present = "check", value_name = "PATH")]
    generator: Option<PathBuf>,

    /// Argument passed to the generator before PROMPT_JSON and OUTPUT_JSON.
    #[arg(long, allow_hyphen_values = true, value_name = "ARG")]
    generator_arg: Vec<OsString>,

    /// Compile and run the tests of a candidate JSON file on every seed and print the result as
    /// JSON. Generators use this to check a candidate before they return it.
    #[arg(
        long,
        value_name = "CANDIDATE_JSON",
        conflicts_with_all = ["target", "brief", "generator"]
    )]
    check: Option<PathBuf>,

    /// Deterministic seeds used to evaluate every candidate. Supply at least two.
    #[arg(long, required = true, action = clap::ArgAction::Append, value_name = "SEED")]
    seed: Vec<U256>,

    /// Number of generator rounds of each agent.
    ///
    /// An agent whose last round leaves no kept file and no possible bug, for example because its
    /// tests do not compile, gets one repair round with the errors in its feedback.
    #[arg(long, default_value_t = 1, value_name = "N")]
    rounds: usize,

    /// Number of generator agents. Each agent runs the generator independently and in parallel
    /// in its own copy of the project, with its own seed and audit lens (spec, state, code, or
    /// tests, in turn).
    #[arg(long, default_value_t = DEFAULT_AGENTS, value_name = "N")]
    agents: usize,

    /// Number of agents that must report the same root cause before Forge reports it as a
    /// possible bug.
    ///
    /// Two failing tests have the same root cause when their evidence names the same target file
    /// in `code_path` and they share a function name, or at least 30% of their claim words.
    /// Groups below the quorum are only in `cache/properties/report.json`. Defaults to 2, or 1
    /// with a single agent.
    #[arg(long, value_name = "K")]
    quorum: Option<usize>,

    /// Agent whose check journal records a `--check` run. Forge sets it in the check command of
    /// each agent.
    #[arg(long, hide = true, requires = "check", value_name = "ID")]
    agent: Option<usize>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Candidate {
    schema: String,
    rationale: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    generator: Option<GeneratorMetadata>,
    files: Vec<CandidateFile>,
    /// The audit's hypotheses, each proved or refuted by a test in `files`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    hypotheses: Vec<Hypothesis>,
    /// Why each test that the generator expects to fail is a possible bug, by `Contract::test`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    evidence: BTreeMap<String, Evidence>,
    /// What the generator spent to write this candidate, if it reports it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    usage: Option<Usage>,
}

/// Model usage that a generator reports.
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

/// One claim of the audit: what the documentation demands and how a test proves it.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct Hypothesis {
    /// Short identifier that names the test which proves the hypothesis.
    id: String,
    /// The audit lens that produced the hypothesis: `spec`, `state`, `code`, or `tests`.
    lens: String,
    /// The function under test.
    function: String,
    /// The documented behavior and where the documentation states it.
    claim: String,
    /// The concrete input or setup sequence, the call, and the result the documentation demands.
    recipe: String,
    result: HypothesisResult,
}

/// What the test of a hypothesis showed.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum HypothesisResult {
    /// The test fails: the code violates the claim.
    Finding,
    /// The test passes: the code meets the claim.
    Refuted,
    /// No test could reach the state the recipe needs.
    NotTestable,
}

/// The documentation behind a test that the generator expects to fail.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct Evidence {
    /// The documented behavior that the test asserts.
    claim: String,
    /// The documentation sentence that states the claim, verbatim.
    quote: String,
    /// Where the quote comes from: `path:line`, or a standard and its clause.
    source: String,
    basis: EvidenceBasis,
    /// The code that violates the claim: `path:line` and what it does.
    code_path: String,
}

/// What supports the claim. Possible bugs are reported in this order.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "lowercase")]
enum EvidenceBasis {
    /// Documentation states the claim: NatSpec or interface docs, a standard's requirement,
    /// sibling or contract docs, the README, or a specification.
    Documented,
    /// Only names, comments, behavior elsewhere in the implementation, or guessed intent support
    /// the claim.
    Inferred,
}

/// Self-reported metadata from the external generator.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct GeneratorMetadata {
    agent: String,
    model: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct CandidateFile {
    path: PathBuf,
    content: String,
}

#[derive(Debug, Serialize)]
struct GeneratorPrompt<'a> {
    schema: &'static str,
    round: usize,
    agent: AgentPrompt,
    project: &'a Path,
    brief: &'a str,
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    context: &'a [PromptSource],
    protocol: &'static [&'static str],
    guidance: &'static [&'static str],
    targets: &'a [PathBuf],
    seeds: &'a [U256],
    project_context: &'a ProjectContext,
    #[serde(skip_serializing_if = "Option::is_none")]
    current_candidate: Option<&'a Candidate>,
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    last_rejected_sources: &'a [PromptSource],
    previous_feedback: &'a [Round],
    output_contract: OutputContract,
}

/// Who the agent is in the run, so independent agents can diversify.
#[derive(Debug, Serialize)]
struct AgentPrompt {
    /// The agent's number, from 1.
    id: usize,
    /// The number of agents in the run.
    count: usize,
    /// The lens that the agent audits through first, named before its colon.
    lens: &'static str,
    /// A seed of this agent, for example to vary sampling or the order of hypotheses.
    seed: U256,
}

#[derive(Debug, Serialize)]
struct ProjectContext {
    remappings: Vec<String>,
    /// Whether each external call from a test contract runs as its own transaction.
    isolate: bool,
    evm_version: String,
    target_sources: Vec<PromptSource>,
    /// Interfaces and base contracts that the targets inherit, nearest first.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    inherited_sources: Vec<PromptSource>,
    test_sources: Vec<PromptSource>,
    /// The existing tests of each function of the targets, by name.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    existing_tests: BTreeMap<String, Vec<String>>,
    /// The project's test files that import a target, for the oracle check.
    #[serde(skip)]
    test_files: Vec<PathBuf>,
}

#[derive(Debug, Serialize)]
struct PromptSource {
    path: PathBuf,
    content: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    truncated: bool,
}

#[derive(Debug, Serialize)]
struct OutputContract {
    schema: &'static str,
    allowed_path_prefix: PathBuf,
    example: Candidate,
    note: &'static str,
    /// Command that compiles and runs the tests of a candidate file.
    check_command: Vec<String>,
}

/// The result of one round, in `rounds.json` and in the feedback of later prompts.
#[derive(Debug, Default, Serialize)]
struct Round {
    round: usize,
    /// Digest of the evaluated candidate, which is in `<digest>/candidate.json`.
    #[serde(skip_serializing_if = "Option::is_none")]
    candidate_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    generator: Option<GeneratorMetadata>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rationale: Option<String>,
    /// How many hypotheses the audit listed, by result.
    #[serde(skip_serializing_if = "Option::is_none")]
    audit: Option<AuditSummary>,
    /// Proposed files that Forge kept.
    kept: Vec<PathBuf>,
    reasons: Vec<String>,
    /// Tests that fail on every seed.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    possible_bugs: Vec<CheckFailure>,
    #[serde(skip_serializing_if = "Option::is_none")]
    usage: Option<Usage>,
}

/// The hypothesis counts of one proposal, by result.
#[derive(Clone, Copy, Debug, Default, Serialize)]
struct AuditSummary {
    hypotheses: usize,
    findings: usize,
    refuted: usize,
    not_testable: usize,
}

impl AuditSummary {
    fn new(hypotheses: &[Hypothesis]) -> Option<Self> {
        if hypotheses.is_empty() {
            return None;
        }
        let count = |result| hypotheses.iter().filter(|h| h.result == result).count();
        Some(Self {
            hypotheses: hypotheses.len(),
            findings: count(HypothesisResult::Finding),
            refuted: count(HypothesisResult::Refuted),
            not_testable: count(HypothesisResult::NotTestable),
        })
    }
}

impl std::fmt::Display for AuditSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "audited {} hypotheses: {} findings, {} refuted, {} not testable",
            self.hypotheses, self.findings, self.refuted, self.not_testable
        )
    }
}

/// A candidate test that fails on every seed.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct CheckFailure {
    /// `Contract::test`.
    test: String,
    path: PathBuf,
    reason: String,
    /// The generator's evidence that the failure is a bug.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    evidence: Option<Evidence>,
    /// Source of the file that holds the test. Only `--check` records it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source: Option<String>,
}

impl CheckFailure {
    /// Report order: documented evidence first, then inferred, then no evidence, then by name.
    fn report_key(&self) -> (bool, Option<EvidenceBasis>, &str) {
        let basis = self.evidence.as_ref().map(|evidence| evidence.basis);
        (basis.is_none(), basis, &self.test)
    }
}

/// A test of one agent that fails on every seed.
#[derive(Debug, Serialize)]
struct AgentFailure {
    agent: usize,
    #[serde(flatten)]
    failure: CheckFailure,
    /// Where Forge saw the failure.
    note: String,
    /// The call outcome that the failure disputes, for the oracle check.
    #[serde(skip)]
    dispute: Option<CallClaim>,
}

impl std::fmt::Display for AgentFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let failure = &self.failure;
        write!(f, "  agent {}: {} ({})", self.agent, failure.test, failure.reason)?;
        if let Some(evidence) = &failure.evidence {
            write!(
                f,
                "\n    claim: {}\n    quote: {:?} ({})\n    code path: {}",
                evidence.claim, evidence.quote, evidence.source, evidence.code_path
            )?;
        }
        write!(f, "\n    {}", self.note)
    }
}

/// The findings of one agent.
#[derive(Default)]
struct AgentFindings {
    /// Tests that fail on every seed.
    possible_bugs: Vec<AgentFailure>,
    /// Failures from the check journal that do not fail on every seed when Forge runs them again.
    unverified: Vec<AgentFailure>,
    lock_ins: Vec<LockIn>,
}

/// A kept test that its agent reported as a bug. It passes, so it can assert the behavior that
/// the agent flagged as wrong.
#[derive(Debug, Serialize)]
struct LockIn {
    agent: usize,
    /// `Contract::test`.
    test: String,
    path: PathBuf,
    /// Where the agent reported the test as a bug.
    reason: &'static str,
}

/// The failing tests of all agents that have one root cause.
#[derive(Debug, Serialize)]
struct BugGroup {
    /// The agents that report the root cause.
    agents: Vec<usize>,
    /// The failing tests, by agent. The first test has the strongest evidence.
    failures: Vec<AgentFailure>,
    /// The project tests that assert the call outcome that a failing test disputes.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    contradicts_existing_tests: Vec<String>,
}

/// What two failing tests must share to have one root cause: the target file in the evidence, and
/// a function name or most of the claim's words.
struct RootCause {
    /// The file name of the first Solidity path in `code_path`, else in `source`.
    file: String,
    /// The function calls and camelCase identifiers in `code_path` and `claim`.
    identifiers: HashSet<String>,
    /// The words of `claim`, without stop words and words of one or two letters.
    words: HashSet<String>,
}

impl RootCause {
    fn new(evidence: &Evidence) -> Option<Self> {
        let paths = format!("{} {}", evidence.code_path, evidence.source);
        let file = SOLIDITY_FILE.find(&paths)?.as_str().rsplit('/').next()?.to_string();
        let text = format!("{} {}", evidence.code_path, evidence.claim);
        let identifiers = FUNCTION_CALL
            .find_iter(&text)
            .chain(CAMEL_CASE.find_iter(&text))
            .map(|identifier| identifier.as_str().to_string())
            .collect();
        let claim = evidence.claim.to_lowercase();
        let words = WORD
            .find_iter(&claim)
            .map(|word| word.as_str())
            .filter(|word| word.len() > 2 && !STOP_WORDS.contains(word))
            .map(String::from)
            .collect();
        Some(Self { file, identifiers, words })
    }

    fn agrees(&self, other: &Self) -> bool {
        let shared = self.words.intersection(&other.words).count();
        let all = self.words.union(&other.words).count();
        self.file == other.file
            && (!self.identifiers.is_disjoint(&other.identifiers)
                || (shared > 0 && shared * CLAIM_OVERLAP.1 >= all * CLAIM_OVERLAP.0))
    }
}

/// The tests of a candidate, run on every seed.
#[derive(Default)]
struct TestRun {
    /// Problems that the generator can fix, such as compile errors and failing tests.
    reasons: Vec<String>,
    /// Tests that fail on every seed.
    possible_bugs: Vec<CheckFailure>,
    /// Tests that pass on every seed.
    passing: Vec<RerunFailure>,
    /// Files with a test that does not pass on every seed.
    failing_files: HashSet<PathBuf>,
    /// The call outcome that each possible bug disputes, by `Contract::test`, on the first seed.
    disputes: HashMap<String, CallClaim>,
}

impl PropertiesArgs {
    pub fn run(self) -> Result<()> {
        ensure!(self.seed.len() >= 2, "at least two --seed values are required");
        ensure!(
            self.seed.iter().collect::<HashSet<_>>().len() == self.seed.len(),
            "--seed values must be distinct"
        );
        ensure!(self.rounds > 0, "--rounds must be greater than zero");
        ensure!(self.agents > 0, "--agents must be greater than zero");
        let quorum = self.quorum.unwrap_or(self.agents.min(DEFAULT_QUORUM));
        ensure!((1..=self.agents).contains(&quorum), "--quorum must be between 1 and --agents");
        ensure!(
            self.target.iter().all(|path| workspace::is_safe_relative_path(path)),
            "--target paths must be project-relative paths"
        );
        let root = self.root.canonicalize().wrap_err("failed to resolve project root")?;
        let config = Config::load_with_root(&root)?.sanitized();
        let generated_tests =
            workspace::relative_to_root(&config.root, &config.test).join("generated");
        workspace::ensure_safe_relative_path(&generated_tests, "generated test", &config.test)?;
        let forge = std::env::current_exe().wrap_err("failed to resolve Forge executable")?;
        let cache_root = config.cache_path.join("properties");
        let campaign =
            Campaign { args: &self, config: &config, forge, generated_tests, cache_root, quorum };
        if let Some(path) = &self.check {
            return campaign.check(path);
        }
        campaign.run()
    }
}

/// The settings of one `forge properties` run.
struct Campaign<'a> {
    args: &'a PropertiesArgs,
    config: &'a Config,
    forge: PathBuf,
    generated_tests: PathBuf,
    cache_root: PathBuf,
    quorum: usize,
}

/// The inputs that every agent shares.
struct AgentInputs {
    brief: String,
    context: Vec<PromptSource>,
    generator: PathBuf,
    /// The target files, relative to the project root.
    targets: Vec<PathBuf>,
    project_context: ProjectContext,
}

/// The state of one generator agent.
struct Agent<'a> {
    campaign: &'a Campaign<'a>,
    id: usize,
    /// The directory of this agent's generated files, relative to the project root.
    generated_tests: PathBuf,
    /// The directory of this agent's candidates, rounds, and check journal.
    cache: PathBuf,
    rounds: Vec<Round>,
    /// The kept files.
    current: Option<Candidate>,
    /// The tests in the kept files.
    current_tests: Vec<RerunFailure>,
    /// The rejected files of the latest round.
    rejected_sources: Vec<PromptSource>,
    /// The call outcome that each possible bug of the rounds disputes, by `Contract::test`.
    disputes: HashMap<String, CallClaim>,
}

impl Campaign<'_> {
    fn run(&self) -> Result<()> {
        let (Some(brief), Some(generator)) = (&self.args.brief, &self.args.generator) else {
            eyre::bail!("--brief and --generator are required");
        };
        let brief = fs::read_to_string(brief).wrap_err("failed to read campaign brief")?;
        let context = self
            .args
            .context
            .iter()
            .map(|path| {
                let content = fs::read_to_string(path)
                    .wrap_err_with(|| format!("failed to read context {}", path.display()))?;
                Ok(PromptSource { path: path.clone(), content, truncated: false })
            })
            .collect::<Result<Vec<_>>>()?;
        let generator = generator.canonicalize().wrap_err("failed to resolve generator")?;
        let targets = if self.args.target.is_empty() {
            source_files_iter(&self.config.src, MultiCompilerLanguage::FILE_EXTENSIONS)
                .map(|path| workspace::relative_to_root(&self.config.root, &path))
                .collect()
        } else {
            self.args.target.clone()
        };
        let project_context = project_context(self.config, &targets);
        let inputs = AgentInputs { brief, context, generator, targets, project_context };

        let mut agents = (1..=self.args.agents)
            .map(|id| Agent {
                campaign: self,
                id,
                generated_tests: self.generated_tests.join(format!("agent-{id}")),
                cache: self.cache_root.join(format!("agent-{id}")),
                rounds: vec![],
                current: None,
                current_tests: vec![],
                rejected_sources: vec![],
                disputes: HashMap::new(),
            })
            .collect::<Vec<_>>();
        // Agents work in copies of the project, but the copies link some project directories. A
        // project file that changes while they run taints the run.
        let fingerprint = project_fingerprint(self.config);
        // An agent error ends only that agent, and the run reports it after the other agents, so
        // their results are not lost.
        let errors = thread::scope(|scope| {
            let inputs = &inputs;
            let agents = agents
                .iter_mut()
                .map(|agent| scope.spawn(move || agent.run(inputs)))
                .collect::<Vec<_>>();
            agents
                .into_iter()
                .map(|agent| agent.join().expect("agent thread panicked"))
                .collect::<Vec<_>>()
        });
        let errors = errors
            .into_iter()
            .zip(&agents)
            .filter_map(|(result, agent)| Some(format!("agent {}: {:#}", agent.id, result.err()?)))
            .collect::<Vec<_>>();
        let after = project_fingerprint(self.config);
        let tainted = after
            .iter()
            .filter(|(path, metadata)| fingerprint.get(*path) != Some(*metadata))
            .map(|(path, _)| path)
            .chain(fingerprint.keys().filter(|path| !after.contains_key(*path)))
            .cloned()
            .collect::<Vec<_>>();
        self.report(&agents, &inputs.project_context.test_files, &tainted)?;
        ensure!(errors.is_empty(), "{}", errors.join("\n"));
        Ok(())
    }

    /// Prints the possible bugs that reach the quorum and writes one copy of each kept file to the
    /// project.
    fn report(
        &self,
        agents: &[Agent<'_>],
        test_files: &[PathBuf],
        tainted: &[PathBuf],
    ) -> Result<()> {
        let mut failures = Vec::new();
        let mut unverified = Vec::new();
        let mut lock_ins = Vec::new();
        for agent in agents {
            let findings = agent.findings()?;
            failures.extend(findings.possible_bugs);
            unverified.extend(findings.unverified);
            lock_ins.extend(findings.lock_ins);
        }
        let (mut possible_bugs, below_quorum) = group_failures(failures)
            .into_iter()
            .partition::<Vec<_>, _>(|group| group.agents.len() >= self.quorum);
        let oracle_error = self
            .check_existing_tests(&mut possible_bugs, test_files)
            .err()
            .map(|error| format!("{error:#}"));
        // A possible bug that the project's own tests contradict is more likely a wrong claim.
        possible_bugs.sort_by_key(|group| !group.contradicts_existing_tests.is_empty());
        for group in &possible_bugs {
            let lead = &group.failures[0].failure;
            let (basis, summary) = match &lead.evidence {
                Some(evidence) if evidence.basis == EvidenceBasis::Documented => {
                    ("documented", evidence.claim.as_str())
                }
                Some(evidence) => ("inferred", evidence.claim.as_str()),
                None => ("no evidence", lead.test.as_str()),
            };
            let agreement = format!("{}/{} agents", group.agents.len(), self.args.agents);
            if group.contradicts_existing_tests.is_empty() {
                sh_println!("possible bug ({basis}, {agreement}): {summary}")?;
            } else {
                sh_println!(
                    "possible bug ({basis}, {agreement}, contradicts-existing-tests): {summary}\n  contradicting tests: {}",
                    group.contradicts_existing_tests.join(", ")
                )?;
            }
            for failure in &group.failures {
                sh_println!("{failure}")?;
            }
        }
        let report_path = self.cache_root.join("report.json");
        let report_display = report_path.strip_prefix(&self.config.root).unwrap_or(&report_path);
        if !below_quorum.is_empty() {
            sh_println!(
                "{} possible bug(s) below the quorum of {}: see {}",
                below_quorum.len(),
                self.quorum,
                report_display.display()
            )?;
        }
        if !unverified.is_empty() {
            sh_println!(
                "{} check journal failure(s) did not fail again in a fresh copy: see {}",
                unverified.len(),
                report_display.display()
            )?;
        }
        if let Some(error) = &oracle_error {
            sh_warn!("could not check possible bugs against the project's tests: {error}")?;
        }
        if !tainted.is_empty() {
            sh_warn!(
                "{} project file(s) changed while the agents ran, so the run is tainted: see {}",
                tainted.len(),
                report_display.display()
            )?;
        }
        for agent in agents {
            for round in &agent.rounds {
                if let Some(audit) = &round.audit {
                    sh_println!("audit agent {} round {}: {audit}", agent.id, round.round)?;
                }
            }
        }
        let usage = agents
            .iter()
            .filter_map(|agent| {
                let usage =
                    agent.rounds.iter().filter_map(|round| round.usage).reduce(Usage::add)?;
                Some(serde_json::json!({ "agent": agent.id, "usage": usage }))
            })
            .collect::<Vec<_>>();
        if let Some(total) = agents
            .iter()
            .flat_map(|agent| &agent.rounds)
            .filter_map(|round| round.usage)
            .reduce(Usage::add)
        {
            sh_println!(
                "generator usage: {} input tokens, {} output tokens, {:.2} USD",
                total.input_tokens,
                total.output_tokens,
                total.cost_usd
            )?;
        }

        // Keep one copy of each file. A file that another file of the same agent imports stays,
        // so the imports still resolve.
        let mut seen = HashSet::new();
        let mut kept = Vec::new();
        let mut duplicates = Vec::new();
        for current in agents.iter().filter_map(|agent| agent.current.as_ref()) {
            for file in &current.files {
                let normalized =
                    COMMENT.replace_all(&file.content, "").split_whitespace().collect::<String>();
                let imported = current.files.iter().any(|other| {
                    other.path != file.path
                        && IMPORT.captures_iter(&other.content).any(|import| {
                            Path::new(&import[1]).file_name() == file.path.file_name()
                        })
                });
                if seen.insert(normalized) || imported {
                    kept.push(file);
                } else {
                    duplicates.push(&file.path);
                }
            }
        }
        lock_ins.retain(|lock_in| kept.iter().any(|file| file.path == lock_in.path));
        fs::create_dir_all(&self.cache_root)?;
        let report = serde_json::json!({
            "agents": self.args.agents,
            "quorum": self.quorum,
            "possible_bugs": possible_bugs,
            "below_quorum": below_quorum,
            "lock_ins": lock_ins,
            "unverified": unverified,
            "oracle_error": oracle_error,
            "tainted": tainted,
            "kept": kept.iter().map(|file| &file.path).collect::<Vec<_>>(),
            "duplicates": duplicates,
            "usage": usage,
        });
        fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;

        if kept.is_empty() {
            sh_println!("no generated test file was kept")?;
            return Ok(());
        }
        // Evaluation rejects paths that exist or leave the project, so this only adds files.
        for file in &kept {
            let path = self.config.root.join(&file.path);
            fs::create_dir_all(path.parent().expect("candidate path has a parent"))?;
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
                "skipped {} generated test file(s) that repeat a file of another agent",
                duplicates.len()
            )?;
        }
        for lock_in in &lock_ins {
            sh_println!("possible lock-in: {} passes, but {}", lock_in.test, lock_in.reason)?;
        }
        Ok(())
    }

    /// Marks each possible bug whose failing test disputes a call outcome that a passing project
    /// test asserts; see the `oracle` module for the exact rule. Runs the project's tests of the
    /// targets once, on the first seed, and only if a possible bug disputes a call.
    fn check_existing_tests(&self, groups: &mut [BugGroup], test_files: &[PathBuf]) -> Result<()> {
        let disputes = groups.iter().flat_map(|group| &group.failures).any(|f| f.dispute.is_some());
        if test_files.is_empty() || !disputes {
            return Ok(());
        }
        let workspace = candidate_workspace(self.config, &[])?;
        let paths = test_files.iter().map(|path| path.display().to_string()).collect::<Vec<_>>();
        let mut command = self.forge_command(workspace.path(), &self.args.seed[0]);
        command.args(["test", "--json", "--match-path", &format!("{{{}}}", paths.join(","))]);
        // Invariant campaigns are slow and assert no single call.
        command.args(["--no-match-test", "^(invariant|statefulFuzz)"]);
        command.args(CALL_TRACES);
        let output = command.output().wrap_err("failed to run the project's tests")?;
        ensure!(
            output.status.success() || !output.stdout.is_empty(),
            "the project's tests failed: {}",
            stderr(&output)
        );
        let mut asserted = HashMap::<CallKey, Vec<(CallOutcome, String)>>::new();
        for (contract, test, result) in suite_results(&output)? {
            let name = test_display_name(&RerunFailure { contract, test });
            for claim in oracle::asserted_calls(&result) {
                asserted.entry(claim.key).or_default().push((claim.outcome, name.clone()));
            }
        }
        for group in groups {
            let mut tests = BTreeSet::new();
            for dispute in group.failures.iter().filter_map(|failure| failure.dispute.as_ref()) {
                let asserts = asserted.get(&dispute.key).into_iter().flatten();
                tests.extend(
                    asserts
                        .filter(|(outcome, _)| *outcome == dispute.outcome)
                        .map(|(_, test)| test.clone()),
                );
            }
            group.contradicts_existing_tests = tests.into_iter().collect();
        }
        Ok(())
    }

    /// Runs the `--check` command.
    fn check(&self, path: &Path) -> Result<()> {
        let candidate = read_candidate(path, &self.generated_tests)?;
        let workspace = candidate_workspace(self.config, &candidate.files)?;
        let mut run = self.run_tests(workspace.path(), &candidate)?;
        let output = serde_json::to_string_pretty(&serde_json::json!({
            "passed": run.reasons.is_empty(),
            "reasons": run.reasons,
            "possible_bugs": run.possible_bugs,
        }))?;
        // Generators often remove a failing test before they return, so the report needs this
        // record to show the possible bug.
        if !run.possible_bugs.is_empty() {
            let mut lines = Vec::new();
            for failure in &mut run.possible_bugs {
                failure.source = candidate
                    .files
                    .iter()
                    .find(|file| file.path == failure.path)
                    .map(|file| file.content.clone());
                serde_json::to_writer(&mut lines, failure)?;
                lines.push(b'\n');
            }
            // Each agent has its own journal.
            let journal = match self.args.agent {
                Some(id) => self.cache_root.join(format!("agent-{id}")),
                None => self.cache_root.clone(),
            };
            fs::create_dir_all(&journal)?;
            // One write per check keeps the lines of concurrent checks apart.
            fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(journal.join(CHECK_JOURNAL))
                .and_then(|mut file| file.write_all(&lines))
                .wrap_err("failed to write the check journal")?;
        }
        sh_println!("{output}")?;
        ensure!(run.reasons.is_empty(), "candidate check failed");
        Ok(())
    }

    /// Runs every test in the candidate files on every seed.
    fn run_tests(&self, workspace: &Path, candidate: &Candidate) -> Result<TestRun> {
        let paths = candidate
            .files
            .iter()
            .map(|file| file.path.display().to_string())
            .collect::<Vec<_>>()
            .join(",");
        let mut reasons = Vec::new();
        // The passing seeds and the failures of each test.
        let mut outcomes = BTreeMap::<(String, String), (usize, Vec<String>)>::new();
        let mut disputes = HashMap::new();
        for (index, seed) in self.args.seed.iter().enumerate() {
            let mut command = self.forge_command(workspace, seed);
            command.args(["test", "--json", "--match-path", &format!("{{{paths}}}")]);
            command.args(SELECT_ALL_TESTS);
            // The traces of one seed are enough for the oracle check.
            if index == 0 {
                command.args(CALL_TRACES);
            }
            let output = command.output().wrap_err("failed to run candidate tests")?;
            if !output.status.success() && output.stdout.is_empty() {
                reasons.push(format!("candidate tests failed on seed {seed}: {}", stderr(&output)));
                continue;
            }
            let results = test_results(&output)?;
            if results.is_empty() {
                reasons.push(format!("candidate files contain no runnable tests on seed {seed}"));
            }
            for (test, status, reason, dispute) in results {
                let name = test_display_name(&test);
                if let Some(dispute) = dispute
                    && index == 0
                {
                    disputes.insert(name.clone(), dispute);
                }
                let (passes, failures) = outcomes.entry((test.contract, test.test)).or_default();
                match status {
                    TestStatus::Success => *passes += 1,
                    TestStatus::Failure => {
                        let reason = reason
                            .filter(|reason| !reason.is_empty())
                            .unwrap_or_else(|| "the test failed without a reason".to_string());
                        reasons.push(format!("{name} failed on seed {seed}: {reason}"));
                        failures.push(format!("seed {seed}: {reason}"));
                    }
                    TestStatus::Skipped => reasons.push(format!(
                        "{name} was skipped on seed {seed}: {}",
                        reason.unwrap_or_else(|| "no reason reported".to_string())
                    )),
                }
            }
        }
        let seeds = self.args.seed.len();
        let mut run = TestRun { reasons, disputes, ..Default::default() };
        for ((contract, test), (passes, failures)) in outcomes {
            let test = RerunFailure { contract, test };
            if passes == seeds {
                run.passing.push(test);
                continue;
            }
            run.failing_files.insert(test_path(&test));
            if failures.len() == seeds {
                let name = test_display_name(&test);
                run.possible_bugs.push(CheckFailure {
                    evidence: candidate.evidence.get(&name).cloned(),
                    test: name,
                    path: test_path(&test),
                    reason: failures.join("; "),
                    source: None,
                });
            }
        }
        run.possible_bugs.sort_by(|a, b| a.report_key().cmp(&b.report_key()));
        run.disputes.retain(|name, _| run.possible_bugs.iter().any(|bug| bug.test == *name));
        Ok(run)
    }

    /// Returns a Forge command for one seed. Each seed persists fuzz and invariant failures in
    /// its own directory, so a counterexample found with one seed is not replayed with another.
    fn forge_command(&self, workspace: &Path, seed: &U256) -> Command {
        let seed_dir = seed_dir(self.config, workspace, seed);
        let mut command = Command::new(&self.forge);
        command
            .current_dir(workspace)
            .env("FOUNDRY_FUZZ_SEED", format!("{seed:#x}"))
            .env("FOUNDRY_FUZZ_FAILURE_PERSIST_DIR", seed_dir.join("fuzz"))
            .env("FOUNDRY_INVARIANT_FAILURE_PERSIST_DIR", seed_dir.join("invariant"))
            // Internal runs must not replace the project's own `--rerun` state.
            .env("FOUNDRY_TEST_FAILURES_FILE", seed_dir.join("test-failures"))
            // A workspace has its own resolved `foundry.toml`.
            .env_remove("FOUNDRY_CONFIG")
            .env_remove("FOUNDRY_REMAPPINGS")
            .env_remove("DAPP_REMAPPINGS")
            .env_remove("FOUNDRY_TEST");
        command
    }
}

impl Agent<'_> {
    /// Runs the generator rounds of this agent in its own copy of the project.
    fn run(&mut self, inputs: &AgentInputs) -> Result<()> {
        let campaign = self.campaign;
        let args = campaign.args;
        fs::create_dir_all(&self.cache)?;
        // Generator checks of this run start a new journal.
        if let Err(error) = fs::remove_file(self.cache.join(CHECK_JOURNAL))
            && error.kind() != std::io::ErrorKind::NotFound
        {
            return Err(error).wrap_err("failed to reset the check journal");
        }
        let workspace = candidate_workspace(campaign.config, &[])?;
        let check_command = [campaign.forge.as_os_str(), "properties".as_ref(), "--root".as_ref()]
            .into_iter()
            .chain([
                campaign.config.root.as_os_str(),
                "--check".as_ref(),
                "CANDIDATE_JSON".as_ref(),
            ])
            .map(|arg| arg.to_string_lossy().into_owned())
            .chain(args.seed.iter().flat_map(|seed| ["--seed".to_string(), format!("{seed:#x}")]))
            .chain(["--agent".to_string(), self.id.to_string()])
            .collect::<Vec<_>>();
        let lens = LENSES[(self.id - 1) % LENSES.len()];
        let seed = keccak256(
            [args.seed[0].to_be_bytes::<32>(), U256::from(self.id).to_be_bytes()].concat(),
        )
        .into();

        // An error ends the rounds after `rounds.json` is written, so the kept files are not lost.
        let mut error = None;
        // A last round that leaves no kept file and no possible bug, such as one whose tests do not
        // compile, earns a repair round with its errors in the feedback.
        let mut rounds = args.rounds;
        while self.rounds.len() < rounds {
            let round = self.rounds.len() + 1;
            let round_dir = tempfile::Builder::new().prefix("forge-properties-").tempdir()?;
            let prompt_path = round_dir.path().join("prompt.json");
            let candidate_path = round_dir.path().join("candidate.json");
            let example_contract = format!("GeneratedRound{round}Test");
            let prompt = GeneratorPrompt {
                schema: PROMPT_SCHEMA,
                round,
                agent: AgentPrompt { id: self.id, count: args.agents, lens, seed },
                project: workspace.path(),
                brief: &inputs.brief,
                context: &inputs.context,
                protocol: &PROTOCOL,
                guidance: &GUIDANCE,
                targets: &inputs.targets,
                seeds: &args.seed,
                project_context: &inputs.project_context,
                current_candidate: self.current.as_ref(),
                last_rejected_sources: &self.rejected_sources,
                previous_feedback: &self.rounds,
                output_contract: OutputContract {
                    schema: CANDIDATE_SCHEMA,
                    allowed_path_prefix: self.generated_tests.clone(),
                    check_command: check_command.clone(),
                    example: Candidate {
                        schema: CANDIDATE_SCHEMA.to_string(),
                        rationale: "One line per test: what it checks and why.".to_string(),
                        generator: Some(GeneratorMetadata {
                            agent: "generator name".to_string(),
                            model: "model name".to_string(),
                        }),
                        files: vec![CandidateFile {
                            path: self.generated_tests.join(format!("GeneratedRound{round}.t.sol")),
                            content: format!(
                                "pragma solidity ^0.8.0;\ncontract {example_contract} {{\n    function testProperty() external {{}}\n}}\n"
                            ),
                        }],
                        hypotheses: vec![],
                        evidence: BTreeMap::from([(
                            format!("{example_contract}::testProperty"),
                            Evidence {
                                claim: "The documented behavior that the failing test asserts.".to_string(),
                                quote: "The documentation sentence, verbatim.".to_string(),
                                source: "src/Contract.sol:12, or the standard and its clause".to_string(),
                                basis: EvidenceBasis::Documented,
                                code_path: "src/Contract.sol:40 Contract.function() and what it does instead.".to_string(),
                            },
                        )]),
                        usage: Some(Usage::default()),
                    },
                    note: "Return one JSON object with this shape. Replace the example values; generator and usage are optional. Use new file paths.",
                },
            };
            fs::write(&prompt_path, serde_json::to_vec_pretty(&prompt)?)?;
            self.rejected_sources.clear();

            let output = Command::new(&inputs.generator)
                .args(&args.generator_arg)
                .arg(&prompt_path)
                .arg(&candidate_path)
                .current_dir(workspace.path())
                .output()
                .wrap_err("failed to invoke generator")?;
            if !output.status.success() {
                error = Some(eyre!(
                    "generator failed in round {round} ({}): {}",
                    output.status,
                    stderr(&output)
                ));
                break;
            }
            let mut record = Round { round, ..Default::default() };
            match read_candidate(&candidate_path, &self.generated_tests) {
                Ok(proposal) => {
                    record.generator = proposal.generator.clone();
                    record.rationale = Some(proposal.rationale.clone());
                    record.audit = AuditSummary::new(&proposal.hypotheses);
                    record.usage = proposal.usage;
                    if let Err(error) = self.evaluate(&proposal, &mut record) {
                        record.reasons.push(error.to_string());
                    }
                    self.rejected_sources = prompt_sources(
                        proposal
                            .files
                            .iter()
                            .filter(|file| !record.kept.contains(&file.path))
                            .map(|file| (file.path.as_path(), file.content.as_str())),
                        MAX_PROMPT_SOURCE_BYTES,
                    );
                }
                Err(error) => record.reasons.push(error.to_string()),
            }
            let repair = round == rounds
                && rounds < args.rounds + MAX_REPAIR_ROUNDS
                && record.kept.is_empty()
                && record.possible_bugs.is_empty();
            self.rounds.push(record);
            if repair {
                rounds += 1;
            }
        }
        fs::write(self.cache.join("rounds.json"), serde_json::to_vec_pretty(&self.rounds)?)?;
        error.map_or(Ok(()), Err)
    }

    /// Evaluates a proposal and keeps its files whose tests pass on every seed.
    ///
    /// Problems with the proposal are errors that become rejection reasons; failures of Forge
    /// itself also stop only this round.
    fn evaluate(&mut self, proposal: &Candidate, record: &mut Round) -> Result<()> {
        let config = self.campaign.config;
        let candidate = merge_candidate(self.current.as_ref(), proposal, &self.generated_tests)?;
        for file in &proposal.files {
            let path = config.root.join(&file.path);
            ensure!(!path.exists(), "candidate would overwrite {}", file.path.display());
            // A symlinked directory on the path must not lead outside the project.
            if let Some(existing) = path.ancestors().find(|ancestor| ancestor.exists()) {
                workspace::ensure_within_root(
                    &config.root,
                    existing,
                    "generated test",
                    &file.path,
                )?;
            }
        }
        let digest = keccak256(serde_json::to_vec(&candidate)?).to_string();
        let cache = self.cache.join(&digest);
        fs::create_dir_all(&cache)?;
        fs::write(cache.join("candidate.json"), serde_json::to_vec_pretty(&candidate)?)?;
        record.candidate_digest = Some(digest);

        let workspace = candidate_workspace(config, &candidate.files)?;
        let run = self.campaign.run_tests(workspace.path(), &candidate)?;
        self.disputes.extend(run.disputes);
        record.reasons = run.reasons;
        record.possible_bugs = run.possible_bugs;
        let passing_files = proposal
            .files
            .iter()
            .filter(|file| {
                !run.failing_files.contains(&file.path)
                    && run.passing.iter().any(|test| test_path(test) == file.path)
            })
            .map(|file| file.path.clone())
            .collect::<HashSet<_>>();
        let kept =
            required_candidate_files(config, workspace.path(), &candidate.files, &passing_files)?;
        ensure!(
            kept.iter().all(|path| !run.failing_files.contains(path)),
            "a passing test imports a candidate file with a failing test"
        );
        let tests = run
            .passing
            .into_iter()
            .filter(|test| {
                let path = test_path(test);
                kept.contains(&path)
                    || self.current_tests.iter().any(|current| test_path(current) == path)
            })
            .collect::<Vec<_>>();

        if !kept.is_empty() {
            let mut current = candidate;
            current.files.retain(|file| {
                kept.contains(&file.path)
                    || self.current.as_ref().is_some_and(|current| {
                        current.files.iter().any(|kept| kept.path == file.path)
                    })
            });
            self.current = Some(current);
            self.current_tests = tests;
        }
        record.kept = kept;
        Ok(())
    }

    /// Returns the tests of this agent that fail on every seed, from its rounds and its check
    /// journal, and the kept tests that lock in a reported bug.
    fn findings(&self) -> Result<AgentFindings> {
        let config = self.campaign.config;
        let mut findings = AgentFindings::default();
        for round in &self.rounds {
            let digest = round.candidate_digest.as_deref().unwrap_or_default();
            let path = self.cache.join(digest);
            let path = path.strip_prefix(&config.root).unwrap_or(&path).display();
            let note = format!("fails on every seed; candidate: {path}");
            findings.possible_bugs.extend(round.possible_bugs.iter().map(|failure| AgentFailure {
                agent: self.id,
                failure: failure.clone(),
                note: note.clone(),
                dispute: self.disputes.get(&failure.test).cloned(),
            }));
        }

        // A kept test passes on every seed. If the agent reported it as a bug, in its evidence,
        // its hypotheses, or a check, the test can assert the behavior that the agent flagged.
        let journal = self.cache.join(CHECK_JOURNAL);
        let journal_failures = read_check_journal(&journal)?;
        let kept = |test: &str, path: &Path| {
            self.current_tests
                .iter()
                .any(|kept| test_display_name(kept) == test && test_path(kept) == path)
        };
        if let Some(current) = &self.current {
            for test in &self.current_tests {
                let name = test_display_name(test);
                let function = name.rsplit_once("::").map_or(name.as_str(), |(_, name)| name);
                let reason = if current.evidence.contains_key(&name) {
                    "its evidence reports it as a failing test"
                } else if current.hypotheses.iter().any(|hypothesis| {
                    hypothesis.id == function && hypothesis.result == HypothesisResult::Finding
                }) {
                    "its hypothesis is a finding"
                } else if journal_failures.iter().any(|failure| {
                    failure.evidence.is_some()
                        && failure.test == name
                        && failure.path == test_path(test)
                }) {
                    "it failed with evidence in a generator check"
                } else {
                    continue;
                };
                findings.lock_ins.push(LockIn {
                    agent: self.id,
                    path: test_path(test),
                    test: name,
                    reason,
                });
            }
        }

        // A generator can remove a failing test after a check. The failures that no round
        // reported and that the kept files do not hold are possible bugs. The generator can also
        // write the journal, so Forge runs them again in a fresh copy of the project.
        let journal_failures = journal_failures
            .into_iter()
            .filter(|failure| {
                !findings.possible_bugs.iter().any(|reported| reported.failure.test == failure.test)
                    && !kept(&failure.test, &failure.path)
            })
            .collect::<Vec<_>>();
        let run = self.rerun_journal_failures(&journal_failures)?;
        let journal = journal.strip_prefix(&config.root).unwrap_or(&journal);
        let note =
            format!("seen in a generator check, then removed; journal: {}", journal.display());
        for failure in journal_failures {
            let verified = run
                .possible_bugs
                .iter()
                .any(|bug| bug.test == failure.test && bug.path == failure.path);
            let failure = AgentFailure {
                agent: self.id,
                dispute: run.disputes.get(&failure.test).filter(|_| verified).cloned(),
                failure,
                note: note.clone(),
            };
            if verified {
                findings.possible_bugs.push(failure);
            } else {
                findings.unverified.push(failure);
            }
        }
        Ok(findings)
    }

    /// Runs the tests of journal failures in a fresh copy of the project on every seed. The
    /// workspace holds the latest journal source of each file and the kept files, which can hold
    /// helpers that the journal files import.
    fn rerun_journal_failures(&self, failures: &[CheckFailure]) -> Result<TestRun> {
        let mut files = Vec::<CandidateFile>::new();
        for failure in failures.iter().rev() {
            if let Some(source) = &failure.source
                && !files.iter().any(|file| file.path == failure.path)
            {
                files.push(CandidateFile { path: failure.path.clone(), content: source.clone() });
            }
        }
        if files.is_empty() {
            return Ok(TestRun::default());
        }
        for file in self.current.iter().flat_map(|current| &current.files) {
            if !files.iter().any(|known| known.path == file.path) {
                files.push(file.clone());
            }
        }
        let candidate = Candidate {
            schema: CANDIDATE_SCHEMA.to_string(),
            rationale: String::new(),
            generator: None,
            files,
            hypotheses: vec![],
            evidence: BTreeMap::new(),
            usage: None,
        };
        let workspace = candidate_workspace(self.campaign.config, &candidate.files)?;
        self.campaign.run_tests(workspace.path(), &candidate)
    }
}

/// Groups the failing tests of all agents by root cause with single linkage: two tests are in one
/// group when a chain of agreeing pairs links them. A test without evidence agrees with no other
/// test. Groups with documented evidence come first, then groups with more agents.
fn group_failures(failures: Vec<AgentFailure>) -> Vec<BugGroup> {
    let causes = failures
        .iter()
        .map(|failure| failure.failure.evidence.as_ref().and_then(RootCause::new))
        .collect::<Vec<_>>();
    let mut group_of = (0..failures.len()).collect::<Vec<_>>();
    for (i, a) in causes.iter().enumerate() {
        for (j, b) in causes.iter().enumerate().skip(i + 1) {
            if let (Some(a), Some(b)) = (a, b)
                && a.agrees(b)
                && group_of[i] != group_of[j]
            {
                let (from, to) = (group_of[j], group_of[i]);
                group_of.iter_mut().filter(|group| **group == from).for_each(|group| *group = to);
            }
        }
    }
    let mut groups = BTreeMap::<usize, Vec<AgentFailure>>::new();
    for (failure, group) in failures.into_iter().zip(group_of) {
        groups.entry(group).or_default().push(failure);
    }
    let mut groups = groups
        .into_values()
        .map(|mut failures| {
            // The failure with the strongest evidence leads, then the failures by agent.
            failures.sort_by_key(|failure| {
                let (no_evidence, basis, test) = failure.failure.report_key();
                (no_evidence, basis, failure.agent, test.to_string())
            });
            let agents = failures.iter().map(|failure| failure.agent).collect::<BTreeSet<_>>();
            BugGroup {
                agents: agents.into_iter().collect(),
                failures,
                contradicts_existing_tests: vec![],
            }
        })
        .collect::<Vec<_>>();
    groups.sort_by(|a, b| {
        let key = |group: &BugGroup| {
            let (no_evidence, basis, test) = group.failures[0].failure.report_key();
            (no_evidence, basis, Reverse(group.agents.len()), test.to_string())
        };
        key(a).cmp(&key(b))
    });
    groups
}

/// Returns the size and modification time of each project file, by path relative to the root.
/// Hidden entries and the build and cache directories are skipped, and links are not followed.
fn project_fingerprint(config: &Config) -> BTreeMap<PathBuf, (u64, Option<SystemTime>)> {
    let mut files = BTreeMap::new();
    let mut pending = vec![config.root.clone()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if entry.file_name().to_string_lossy().starts_with('.')
                || path == config.out
                || path == config.cache_path
            {
                continue;
            }
            let Ok(metadata) = entry.metadata() else { continue };
            if metadata.is_dir() {
                pending.push(path);
            } else {
                let path = workspace::relative_to_root(&config.root, &path);
                files.insert(path, (metadata.len(), metadata.modified().ok()));
            }
        }
    }
    files
}

/// Copies the project into a temporary workspace and adds the files.
fn candidate_workspace(config: &Config, files: &[CandidateFile]) -> Result<TempDir> {
    let workspace = tempfile::Builder::new().prefix("forge-properties-").tempdir()?;
    let dependencies = config
        .libs
        .iter()
        .map(|lib| lib.as_path())
        .chain(["node_modules".as_ref(), "dependencies".as_ref()])
        .map(|path| config.root.join(path))
        .collect::<Vec<_>>();
    copy_project_tree(config, workspace.path(), Path::new(""), &dependencies)?;
    // Internal runs see the project's resolved config, including inherited `extends` values,
    // absolute paths, and environment overrides, without reading files outside the workspace.
    let mut rebased = workspace::rebase_config_paths(config, workspace.path());
    rebased.extends = None;
    let mut toml = rebased.to_string_pretty()?;
    // Inline config may name the project's other profiles, so they must still exist.
    for profile in config.profiles.iter().filter(|profile| **profile != config.profile) {
        toml.push_str(&format!("\n[profile.{profile}]\n"));
    }
    fs::write(workspace.path().join(Config::FILE_NAME), toml)?;
    // Remappings are already resolved and rebased in the workspace config.
    let remappings = workspace.path().join("remappings.txt");
    if remappings.exists() {
        fs::remove_file(remappings)?;
    }
    for file in files {
        let path = workspace.path().join(&file.path);
        // A linked directory on the path must not lead outside the workspace.
        if let Some(existing) = path.ancestors().find(|ancestor| ancestor.exists()) {
            workspace::ensure_within_root(
                workspace.path(),
                existing,
                "generated test",
                &file.path,
            )?;
        }
        fs::create_dir_all(path.parent().expect("candidate path has a parent"))?;
        fs::write(path, &file.content)?;
    }
    Ok(workspace)
}

/// Copies a project directory into a workspace, without Git data, build output, and, at the
/// project root, hidden entries such as `.env`.
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
) -> Result<()> {
    fs::create_dir_all(copy.join(dir))?;
    for entry in fs::read_dir(config.root.join(dir))? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let rel = dir.join(&name);
        let target = copy.join(&rel);
        if name == ".git"
            || (dir.as_os_str().is_empty() && name.to_string_lossy().starts_with('.'))
            || path == config.out
            || path == config.cache_path
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
            copy_project_tree(config, copy, &rel, dependencies)?;
        } else {
            fs::copy(&path, &target)?;
        }
    }
    Ok(())
}

/// Keeps the imported candidate files needed by passing tests.
fn required_candidate_files(
    config: &Config,
    workspace: &Path,
    files: &[CandidateFile],
    passing: &HashSet<PathBuf>,
) -> Result<Vec<PathBuf>> {
    if passing.is_empty() || passing.len() == files.len() {
        return Ok(files
            .iter()
            .filter(|file| passing.contains(&file.path))
            .map(|file| file.path.clone())
            .collect());
    }
    let rebased = workspace::rebase_config_paths(config, workspace);
    let graph =
        Graph::<MultiCompilerParser>::resolve(&rebased.project_paths::<MultiCompilerLanguage>())
            .wrap_err("failed to resolve generated test imports")?;
    let mut required = passing.clone();
    for path in passing {
        let index = graph
            .nodes
            .iter()
            .position(|node| workspace::relative_to_root(workspace, node.path()) == *path)
            .ok_or_else(|| {
                eyre!("passing test file {} is absent from the import graph", path.display())
            })?;
        required.extend(
            graph
                .all_imported_nodes(index)
                .map(|index| workspace::relative_to_root(workspace, graph.node(index).path())),
        );
    }
    Ok(files
        .iter()
        .filter(|file| required.contains(&file.path))
        .map(|file| file.path.clone())
        .collect())
}

/// Returns every test result in Forge's JSON output, with one result per invariant predicate of a
/// merged invariant campaign.
///
/// A failing test that is not an invariant also has the call outcome that it disputes, if its
/// traces show one.
#[expect(clippy::type_complexity)]
fn test_results(
    output: &Output,
) -> Result<Vec<(RerunFailure, TestStatus, Option<String>, Option<CallClaim>)>> {
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
            results.push((key(&test), status, reason, oracle::disputed_call(&result)));
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
            results.push((key(&predicate.name), predicate.status, reason, None));
        }
        // One campaign result per seed, so the count of failing seeds stays correct.
        if !handler_failures.is_empty() {
            let reason = Some(handler_failures.join("; "));
            results.push((key(&test), TestStatus::Failure, reason, None));
        }
    }
    Ok(results)
}

/// Returns every test result in Forge's JSON output with its contract and test name.
fn suite_results(output: &Output) -> Result<Vec<(String, String, TestResult)>> {
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

fn project_context(config: &Config, targets: &[PathBuf]) -> ProjectContext {
    let target_sources = targets
        .iter()
        .filter_map(|path| Some((path, fs::read_to_string(config.root.join(path)).ok()?)))
        .collect::<Vec<_>>();
    let mut context = ProjectContext {
        remappings: config
            .remappings
            .iter()
            .map(|remapping| remapping.clone().to_relative_remapping().to_string())
            .collect(),
        isolate: config.isolate,
        evm_version: config.evm_version.to_string(),
        target_sources: prompt_sources(
            target_sources.iter().map(|(path, source)| (path.as_path(), source.as_str())),
            MAX_PROMPT_TARGET_BYTES,
        ),
        inherited_sources: vec![],
        test_sources: vec![],
        existing_tests: BTreeMap::new(),
        test_files: vec![],
    };
    let Ok(graph) =
        Graph::<MultiCompilerParser>::resolve(&config.project_paths::<MultiCompilerLanguage>())
    else {
        return context;
    };
    let target_files = graph
        .files()
        .keys()
        .filter(|path| targets.contains(&workspace::relative_to_root(&config.root, path)))
        .cloned()
        .collect::<BTreeSet<_>>();
    let inherited = inherited_files(config, &graph, &target_files)
        .into_iter()
        .map(|index| {
            let node = graph.node(index);
            (workspace::relative_to_root(&config.root, node.path()), node.content())
        })
        .collect::<Vec<_>>();
    context.inherited_sources = prompt_sources(
        inherited.iter().map(|(path, source)| (path.as_path(), *source)),
        MAX_PROMPT_INHERITED_BYTES,
    );
    // Existing tests of the targets show the project's conventions. A test file named after a
    // target, such as `Math.t.sol` for `Math.sol`, usually holds its primary suite.
    let target_stems = targets.iter().filter_map(|path| path.file_stem()).collect::<Vec<_>>();
    let mut tests = existing_test_files(config, &graph, &target_files)
        .into_iter()
        .map(|index| {
            let node = graph.node(index);
            let stem = node.path().file_name().and_then(|name| name.to_str()?.split('.').next());
            let names_target = target_stems.iter().any(|target| target.to_str() == stem);
            (!names_target, workspace::relative_to_root(&config.root, node.path()), node.content())
        })
        .collect::<Vec<_>>();
    tests.sort_by_key(|(other, path, source)| (*other, source.len(), path.clone()));
    context.test_sources = prompt_sources(
        tests.iter().take(MAX_PROMPT_TEST_FILES).map(|(_, path, source)| (path.as_path(), *source)),
        MAX_PROMPT_SOURCE_BYTES,
    );
    context.test_files = tests.iter().map(|(_, path, _)| path.clone()).collect();
    context.existing_tests = existing_tests(
        target_sources.iter().map(|(_, source)| source.as_str()),
        tests.iter().map(|(_, _, source)| *source),
    );
    context
}

/// Returns the project's test files that import a target, directly or through other files. A test
/// file is under the test directory, or ends in `.t.sol` outside the libraries. Generated tests
/// are not project tests.
fn existing_test_files(
    config: &Config,
    graph: &Graph<MultiCompilerParser>,
    targets: &BTreeSet<PathBuf>,
) -> Vec<usize> {
    let generated_tests = config.test.join("generated");
    (0..graph.nodes.len())
        .filter(|&index| {
            let path = graph.node(index).path();
            let in_libs = config.libs.iter().any(|lib| path.starts_with(config.root.join(lib)));
            let is_test = path.starts_with(&config.test)
                || (!in_libs && path.to_string_lossy().ends_with(".t.sol"));
            is_test
                && !path.starts_with(&generated_tests)
                && path.extension().is_some_and(|extension| extension == "sol")
                && graph.imports(path).iter().any(|import| targets.contains(*import))
        })
        .collect()
}

/// Maps each function that the targets declare to the existing tests whose body calls it, by
/// name. A function with no such test maps to an empty list.
///
/// The match is textual: a test that reaches a function only through a helper is not listed, and
/// a call to another function with the same name is.
fn existing_tests<'a>(
    targets: impl IntoIterator<Item = &'a str>,
    tests: impl IntoIterator<Item = &'a str>,
) -> BTreeMap<String, Vec<String>> {
    let mut functions = BTreeMap::<String, BTreeSet<String>>::new();
    for target in targets {
        let target = COMMENT.replace_all(target, " ");
        for function in FUNCTION_NAME.captures_iter(&target) {
            functions.entry(function[1].to_string()).or_default();
        }
    }
    for test in tests {
        let test = COMMENT.replace_all(test, " ");
        for function in TEST_FUNCTION.captures_iter(&test) {
            let signature_end = function.get(0).expect("a match has group 0").end();
            let Some(body) = function_body(&test[signature_end..]) else { continue };
            for call in FUNCTION_CALL.find_iter(body) {
                if let Some(tests) = functions.get_mut(call.as_str().trim_end_matches('(')) {
                    tests.insert(function[1].to_string());
                }
            }
        }
    }
    functions
        .into_iter()
        .map(|(function, tests)| {
            let mut names =
                tests.iter().take(MAX_EXISTING_TESTS_PER_FUNCTION).cloned().collect::<Vec<_>>();
            if tests.len() > names.len() {
                names.push(format!("+{} more", tests.len() - names.len()));
            }
            (function, names)
        })
        .collect()
}

/// Returns the body of the function whose signature starts the source, without its braces, or
/// `None` if the function has no body.
fn function_body(source: &str) -> Option<&str> {
    let start = source.find(['{', ';'])?;
    if source.as_bytes()[start] == b';' {
        return None;
    }
    let mut depth = 0usize;
    for (index, character) in source[start..].char_indices() {
        match character {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&source[start + 1..start + index]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Returns the bullets of one section of `guidance.md`.
fn guidance_section(heading: &str) -> Vec<&'static str> {
    include_str!("properties/guidance.md")
        .lines()
        .skip_while(|line| *line != heading)
        .skip(1)
        .take_while(|line| !line.starts_with("## "))
        .filter_map(|line| line.strip_prefix("- "))
        .collect()
}

/// Bounds sources to one shared byte budget so a single section cannot dominate the prompt.
fn prompt_sources<'a>(
    sources: impl IntoIterator<Item = (&'a Path, &'a str)>,
    budget: usize,
) -> Vec<PromptSource> {
    let mut prompt_sources = Vec::new();
    let mut total_bytes = 0;
    for (path, source) in sources {
        let end = source.floor_char_boundary(budget - total_bytes);
        if end == 0 {
            break;
        }
        total_bytes += end;
        prompt_sources.push(PromptSource {
            path: path.to_path_buf(),
            content: source[..end].to_string(),
            truncated: end < source.len(),
        });
    }
    prompt_sources
}

/// Returns the files that declare the contracts and interfaces that the targets inherit, nearest
/// first. At each inheritance depth, project files come before library files.
fn inherited_files(
    config: &Config,
    graph: &Graph<MultiCompilerParser>,
    targets: &BTreeSet<PathBuf>,
) -> Vec<usize> {
    let target_ids =
        targets.iter().filter_map(|path| graph.files().get(path).copied()).collect::<Vec<_>>();
    // Only files that a target imports can declare its bases.
    let mut declarations = HashMap::new();
    for index in target_ids.iter().flat_map(|&index| graph.all_imported_nodes(index)) {
        let source = COMMENT.replace_all(graph.node(index).content(), " ");
        for captures in DECLARATION.captures_iter(&source) {
            declarations.entry(captures[1].to_string()).or_insert(index);
        }
    }
    let mut included = target_ids.iter().copied().collect::<HashSet<_>>();
    let mut inherited = Vec::new();
    let mut level = target_ids;
    while !level.is_empty() {
        let mut next = Vec::new();
        for index in level {
            for name in base_names(graph.node(index).content()) {
                if let Some(&base) = declarations.get(&name)
                    && included.insert(base)
                {
                    next.push(base);
                }
            }
        }
        next.sort_by_key(|&index| {
            let path = graph.node(index).path();
            config.libs.iter().any(|lib| path.starts_with(config.root.join(lib)))
        });
        inherited.extend(&next);
        level = next;
    }
    inherited
}

/// Returns the names of the contracts and interfaces that the declarations in a source inherit.
fn base_names(source: &str) -> Vec<String> {
    let source = COMMENT.replace_all(source, " ");
    let mut names = Vec::new();
    for captures in INHERITANCE.captures_iter(&source) {
        // Split on commas outside of constructor arguments, such as `ERC20("Name", "N")`.
        let mut depth = 0usize;
        let mut start = 0;
        let list = &captures[1];
        for (index, character) in list.char_indices().chain([(list.len(), ',')]) {
            match character {
                '(' => depth += 1,
                ')' => depth = depth.saturating_sub(1),
                ',' if depth == 0 => {
                    let base = list[start..index].trim();
                    let path =
                        base.split(|c: char| c == '(' || c.is_whitespace()).next().unwrap_or("");
                    // A qualified name such as `Lib.Base` is declared as `Base`.
                    if let Some(name) = path.rsplit('.').next()
                        && !name.is_empty()
                    {
                        names.push(name.to_string());
                    }
                    start = index + 1;
                }
                _ => {}
            }
        }
    }
    names
}

/// Adds the proposal's files to the kept files.
fn merge_candidate(
    current: Option<&Candidate>,
    proposal: &Candidate,
    generated_tests: &Path,
) -> Result<Candidate> {
    let Some(current) = current else { return Ok(proposal.clone()) };
    let mut candidate = current.clone();
    candidate.rationale = format!("{}\n\n{}", current.rationale, proposal.rationale);
    if current.generator != proposal.generator {
        candidate.generator = None;
    }
    candidate.files.extend(proposal.files.iter().cloned());
    candidate.hypotheses.extend(proposal.hypotheses.iter().cloned());
    candidate.evidence.extend(proposal.evidence.clone());
    validate_candidate(&candidate, generated_tests)
        .wrap_err("the proposal does not fit next to the kept files")?;
    Ok(candidate)
}

/// Reads the check journal. Returns the last failure of each test, in the order of first record.
fn read_check_journal(path: &Path) -> Result<Vec<CheckFailure>> {
    let journal = match fs::read_to_string(path) {
        Ok(journal) => journal,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(error) => return Err(error).wrap_err("failed to read the check journal"),
    };
    let mut failures = Vec::<CheckFailure>::new();
    for line in journal.lines().filter(|line| !line.trim().is_empty()) {
        let failure =
            serde_json::from_str::<CheckFailure>(line).wrap_err("invalid check journal entry")?;
        match failures
            .iter_mut()
            .find(|known| known.test == failure.test && known.path == failure.path)
        {
            Some(known) => *known = failure,
            None => failures.push(failure),
        }
    }
    Ok(failures)
}

fn read_candidate(path: &Path, generated_tests: &Path) -> Result<Candidate> {
    let candidate = serde_json::from_slice::<Candidate>(
        &fs::read(path).wrap_err_with(|| format!("failed to read {}", path.display()))?,
    )
    .wrap_err("invalid candidate JSON")?;
    validate_candidate(&candidate, generated_tests)?;
    Ok(candidate)
}

fn validate_candidate(candidate: &Candidate, generated_tests: &Path) -> Result<()> {
    ensure!(candidate.schema == CANDIDATE_SCHEMA, "unsupported candidate schema");
    ensure!(!candidate.files.is_empty(), "candidate contains no files");
    let mut paths = HashSet::new();
    for file in &candidate.files {
        ensure!(
            file.path.extension().is_some_and(|extension| extension == "sol")
                && file.path.starts_with(generated_tests)
                && workspace::is_safe_relative_path(&file.path),
            "candidate file {} is not a Solidity file under {}/",
            file.path.display(),
            generated_tests.display()
        );
        ensure!(paths.insert(&file.path), "candidate contains duplicate file paths");
        // The seeds must decide every run, so a generated test cannot pin its own seed.
        ensure!(
            !has_inline_fuzz_seed(&file.content),
            "candidate {} sets a seed in inline config",
            file.path.display()
        );
    }
    Ok(())
}

/// Checks both dotted and inline-table forms of a NatSpec fuzz seed override.
fn has_inline_fuzz_seed(source: &str) -> bool {
    source.lines().filter_map(|line| line.split_once("forge-config:")).any(|(_, setting)| {
        let setting = setting.split_once("*/").map_or(setting, |(setting, _)| setting);
        setting.trim().parse::<toml_edit::DocumentMut>().is_ok_and(|config| {
            config
                .iter()
                .any(|(_, profile)| profile.get("fuzz").and_then(|fuzz| fuzz.get("seed")).is_some())
        })
    })
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
fn stderr(output: &Output) -> String {
    const HALF: usize = 1_000;
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut in_warning = false;
    let stderr = stderr
        .lines()
        .filter(|line| {
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
    let stderr = stderr.trim();
    let chars = stderr.chars().count();
    if chars <= 2 * HALF {
        return stderr.to_string();
    }
    let head = stderr.chars().take(HALF).collect::<String>();
    let tail = stderr.chars().skip(chars - HALF).collect::<String>();
    format!("{head}\n[...]\n{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failure(agent: usize, test: &str, claim: &str, code_path: &str) -> AgentFailure {
        let evidence = (!claim.is_empty()).then(|| Evidence {
            claim: claim.to_string(),
            quote: String::new(),
            source: "src/Fee.sol:3".to_string(),
            basis: EvidenceBasis::Documented,
            code_path: code_path.to_string(),
        });
        AgentFailure {
            agent,
            failure: CheckFailure {
                test: test.to_string(),
                path: PathBuf::from("test/generated/Fee.t.sol"),
                reason: "revert".to_string(),
                evidence,
                source: None,
            },
            note: String::new(),
            dispute: None,
        }
    }

    fn cause(claim: &str, code_path: &str) -> RootCause {
        RootCause::new(failure(1, "", claim, code_path).failure.evidence.as_ref().unwrap()).unwrap()
    }

    #[test]
    fn root_causes_agree_on_file_and_function_or_claim_words() {
        let cap = cause("The fee is capped at 100 ether.", "src/Fee.sol:7 fee() caps at 10 ether.");
        assert!(
            cap.agrees(&cause("Large amounts pay the maximum.", "src/Fee.sol:7 fee() is wrong"))
        );
        assert!(
            cap.agrees(&cause("The fee is capped at 100 ether for large amounts.", "src/Fee.sol"))
        );
        assert!(!cap.agrees(&cause("The fee is capped at 100 ether.", "src/Other.sol:7 fee()")));
        assert!(!cap.agrees(&cause("A zero amount is charged.", "src/Fee.sol:11 charges()")));
    }

    // Agreeing failures form one group with the set of their agents. Groups with documented
    // evidence and more agents come first; a failure without evidence agrees with none.
    #[test]
    fn groups_link_agreeing_failures_by_agent() {
        let groups = group_failures(vec![
            failure(3, "C::zero", "A zero amount is charged.", "src/Fee.sol:11 charges()"),
            failure(1, "A::cap", "The fee is capped at 100 ether.", "src/Fee.sol:7 fee()"),
            failure(2, "B::cap", "Large amounts pay the maximum.", "src/Fee.sol:7 fee()"),
            failure(2, "B::limit", "Large amounts pay the maximum fee.", "src/Fee.sol:8"),
            failure(1, "D::none", "", ""),
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
                (vec![1, 2], vec!["A::cap", "B::cap", "B::limit"]),
                (vec![3], vec!["C::zero"]),
                (vec![1], vec!["D::none"]),
            ]
        );
    }

    #[test]
    fn existing_tests_name_the_tests_that_call_each_function() {
        let target = "library Fee {\n    function fee(uint256 a) internal pure returns (uint256) {}\n    function charges(uint256 a) internal pure returns (bool) {}\n    function cap() internal pure {}\n    function _round() private {}\n}\n";
        let test = "contract FeeTest {\n    function setUp() public { Fee.cap(); }\n    function testFee() public { Fee.fee(1); }\n    // function testComment() public { Fee.cap(); }\n    function testCharges() public { if (Fee.charges(1)) { Fee.fee(2); } }\n    function testAbstract() public virtual;\n}\n";
        let many =
            (0..7).map(|i| format!("function testCharges{i}() public {{ Fee.charges({i}); }}"));
        let many = format!("contract More {{ {} }}", many.collect::<String>());
        let tests = existing_tests([target], [test, many.as_str()]);
        assert_eq!(
            serde_json::to_string(&tests).unwrap(),
            r#"{"cap":[],"charges":["testCharges","testCharges0","testCharges1","testCharges2","testCharges3","+3 more"],"fee":["testCharges","testFee"]}"#
        );
    }
}
