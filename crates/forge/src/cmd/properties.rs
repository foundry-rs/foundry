//! The `forge properties` command: an external generator audits the targets and writes their
//! documented properties as tests. Forge reports the tests that fail on every seed as possible bugs
//! and keeps the tests that pass. With `--mutation`, a second phase sends the mutants that survive
//! the kept tests to the generator and keeps only new tests that kill one.

use crate::{
    cmd::test::RerunFailure,
    mutation::{MutationJsonOutput, SurvivedMutantJson},
    result::{TestResult, TestStatus},
    workspace,
};
use alloy_primitives::{U256, keccak256};
use clap::Parser;
use eyre::{Context, Result, ensure, eyre};
use foundry_common::sh_println;
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
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    ffi::OsString,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::LazyLock,
};
use tempfile::TempDir;

const CANDIDATE_SCHEMA: &str = "foundry/properties-candidate-v1";
const PROMPT_SCHEMA: &str = "foundry/properties-prompt-v2";
// Bound each source section so it cannot dominate the prompt. Targets hold the documentation that
// defines correct behavior, so they get the largest share; 64 KiB holds most contracts in full.
const MAX_PROMPT_TARGET_BYTES: usize = 64 * 1024;
// Interfaces and base contracts often hold the NatSpec of the functions that a target implements.
const MAX_PROMPT_INHERITED_BYTES: usize = 32 * 1024;
const MAX_PROMPT_SOURCE_BYTES: usize = 16 * 1024;
const MAX_PROMPT_TEST_FILES: usize = 2;
/// Records every test that fails on every seed in a `--check` run, one JSON object per line.
const CHECK_JOURNAL: &str = "check-failures.jsonl";
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
/// The protocol steps, one per bullet under `## Protocol` in `guidance.md`.
static PROTOCOL: LazyLock<Vec<&'static str>> = LazyLock::new(|| guidance_section("## Protocol"));
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

    /// Executable that receives PROMPT_JSON and OUTPUT_JSON as its final arguments.
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
        conflicts_with_all = ["target", "brief", "generator", "mutation"]
    )]
    check: Option<PathBuf>,

    /// Deterministic seeds used to evaluate every candidate. Supply at least two.
    #[arg(long, required = true, action = clap::ArgAction::Append, value_name = "SEED")]
    seed: Vec<U256>,

    /// Maximum number of generator rounds in each phase.
    #[arg(long, default_value_t = 1, value_name = "N")]
    rounds: usize,

    /// After the audit, mutate the targets and ask the generator for tests that kill the mutants
    /// that survive. Forge keeps a new test file only if it kills a survivor on every seed.
    #[arg(long)]
    mutation: bool,

    /// Best-effort timeout for each mutant.
    #[arg(long, requires = "mutation", value_name = "SECONDS")]
    mutation_timeout: Option<u32>,

    /// Run only matching existing test contracts against the mutants. Generated tests always run.
    #[arg(long, requires = "mutation", value_name = "REGEX")]
    match_contract: Option<String>,
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
}

/// One claim of the audit: what the documentation demands and how a test proves it.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct Hypothesis {
    /// Short identifier that names the test which proves the hypothesis.
    id: String,
    /// The audit lens that produced the hypothesis: `spec`, `state`, or `code`.
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

/// The phase of a round.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
enum Phase {
    /// Audit the targets and prove each hypothesis with a test.
    #[default]
    Audit,
    /// Kill the mutants that survive the kept tests.
    Mutation,
}

#[derive(Debug, Serialize)]
struct GeneratorPrompt<'a> {
    schema: &'static str,
    phase: Phase,
    round: usize,
    project: &'a Path,
    brief: &'a str,
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    context: &'a [PromptSource],
    protocol: &'static [&'static str],
    guidance: &'static [&'static str],
    targets: &'a [PathBuf],
    seeds: &'a [U256],
    /// The mutants that the tests do not kill. Only in the mutation phase.
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    survivors: &'a [Survivor],
    project_context: &'a ProjectContext,
    #[serde(skip_serializing_if = "Option::is_none")]
    current_candidate: Option<&'a Candidate>,
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    last_rejected_sources: &'a [PromptSource],
    previous_feedback: &'a [Round],
    output_contract: OutputContract,
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

/// A mutant that a mutation run did not kill.
#[derive(Debug, Serialize)]
struct Survivor {
    path: PathBuf,
    line: usize,
    column: usize,
    original: String,
    mutant: String,
    seeds: Vec<U256>,
    survives_all_seeds: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_context: Option<String>,
}

/// The mutation testing result of one seed.
struct SeedMutation {
    seed: U256,
    output: MutationJsonOutput,
}

/// The result of one round, in `rounds.json` and in the feedback of later prompts.
#[derive(Debug, Default, Serialize)]
struct Round {
    phase: Phase,
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
    /// Survivors that the kept files kill on every seed.
    #[serde(skip_serializing_if = "Option::is_none")]
    killed_survivors: Option<usize>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<String>,
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

impl std::fmt::Display for CheckFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Some(evidence) = &self.evidence else {
            return write!(f, "possible bug (no evidence): {} ({})", self.test, self.reason);
        };
        let basis = match evidence.basis {
            EvidenceBasis::Documented => "documented",
            EvidenceBasis::Inferred => "inferred",
        };
        write!(
            f,
            "possible bug ({basis}): {} ({})\n  claim: {}\n  quote: {:?} ({})\n  code path: {}",
            self.test,
            self.reason,
            evidence.claim,
            evidence.quote,
            evidence.source,
            evidence.code_path
        )
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
}

type MutantId = (String, usize, usize, String, String);

impl PropertiesArgs {
    pub fn run(self) -> Result<()> {
        ensure!(self.seed.len() >= 2, "at least two --seed values are required");
        ensure!(
            self.seed.iter().collect::<HashSet<_>>().len() == self.seed.len(),
            "--seed values must be distinct"
        );
        ensure!(self.rounds > 0, "--rounds must be greater than zero");
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
        let mut campaign = Campaign {
            args: &self,
            config: &config,
            forge,
            generated_tests,
            cache_root,
            targets: vec![],
            rounds: vec![],
            current: None,
            current_tests: vec![],
            rejected_sources: vec![],
            mutants: vec![],
            baseline_tests: vec![],
            killed: 0,
        };
        if let Some(path) = &self.check {
            return campaign.check(path);
        }
        campaign.run()
    }
}

/// The state of one `forge properties` run.
struct Campaign<'a> {
    args: &'a PropertiesArgs,
    config: &'a Config,
    forge: PathBuf,
    generated_tests: PathBuf,
    cache_root: PathBuf,
    /// The target files, relative to the project root.
    targets: Vec<PathBuf>,
    rounds: Vec<Round>,
    /// The kept files.
    current: Option<Candidate>,
    /// The tests in the kept files.
    current_tests: Vec<RerunFailure>,
    /// The rejected files of the latest round.
    rejected_sources: Vec<PromptSource>,
    /// The latest accepted mutation results, one per seed.
    mutants: Vec<SeedMutation>,
    /// The existing tests that run against the mutants.
    baseline_tests: Vec<RerunFailure>,
    /// The survivors that the mutation phase killed.
    killed: usize,
}

impl Campaign<'_> {
    fn run(&mut self) -> Result<()> {
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
        self.targets = if self.args.target.is_empty() {
            source_files_iter(&self.config.src, MultiCompilerLanguage::FILE_EXTENSIONS)
                .map(|path| workspace::relative_to_root(&self.config.root, &path))
                .collect()
        } else {
            self.args.target.clone()
        };
        fs::create_dir_all(&self.cache_root)?;
        // Generator checks of this run start a new journal.
        let journal = self.cache_root.join(CHECK_JOURNAL);
        if let Err(error) = fs::remove_file(&journal)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            return Err(error).wrap_err("failed to reset the check journal");
        }
        let project_context = project_context(self.config, &self.targets);
        let check_command = [self.forge.as_os_str(), "properties".as_ref(), "--root".as_ref()]
            .into_iter()
            .chain([self.config.root.as_os_str(), "--check".as_ref(), "CANDIDATE_JSON".as_ref()])
            .map(|arg| arg.to_string_lossy().into_owned())
            .chain(
                self.args.seed.iter().flat_map(|seed| ["--seed".to_string(), format!("{seed:#x}")]),
            )
            .collect::<Vec<_>>();

        let mut phases = vec![Phase::Audit];
        if self.args.mutation {
            phases.push(Phase::Mutation);
        }
        // An error ends the run after the report, so the kept files are not lost.
        let mut error = None;
        'phases: for phase in phases {
            if phase == Phase::Mutation
                && let Err(start_error) = self.start_mutation_phase()
            {
                error = Some(start_error);
                break;
            }
            for _ in 0..self.args.rounds {
                let survivors = survivors(&self.config.root, &self.mutants);
                if phase == Phase::Mutation && survivors.is_empty() {
                    break;
                }
                let round = self.rounds.len() + 1;
                let round_dir = tempfile::Builder::new().prefix("forge-properties-").tempdir()?;
                let prompt_path = round_dir.path().join("prompt.json");
                let candidate_path = round_dir.path().join("candidate.json");
                let example_contract = format!("GeneratedRound{round}Test");
                let prompt = GeneratorPrompt {
                    schema: PROMPT_SCHEMA,
                    phase,
                    round,
                    project: &self.config.root,
                    brief: &brief,
                    context: &context,
                    protocol: &PROTOCOL,
                    guidance: &GUIDANCE,
                    targets: &self.targets,
                    seeds: &self.args.seed,
                    survivors: &survivors,
                    project_context: &project_context,
                    current_candidate: self.current.as_ref(),
                    last_rejected_sources: &self.rejected_sources,
                    previous_feedback: &self.rounds,
                    output_contract: OutputContract {
                        schema: CANDIDATE_SCHEMA,
                        allowed_path_prefix: self.generated_tests.clone(),
                        check_command: check_command.clone(),
                        example: Candidate {
                            schema: CANDIDATE_SCHEMA.to_string(),
                            rationale: "One line per hypothesis: its id, finding, refuted, or not testable, and why.".to_string(),
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
                            hypotheses: vec![Hypothesis {
                                id: "testProperty".to_string(),
                                lens: "spec".to_string(),
                                function: "Contract.function".to_string(),
                                claim: "The documented behavior and its source (path:line, or a standard and its clause).".to_string(),
                                recipe: "The concrete setup, the call, and the result the documentation demands.".to_string(),
                                result: HypothesisResult::Finding,
                            }],
                            evidence: BTreeMap::from([(
                                format!("{example_contract}::testProperty"),
                                Evidence {
                                    claim: "The documented behavior that the failing test asserts.".to_string(),
                                    quote: "The documentation sentence, verbatim.".to_string(),
                                    source: "src/Contract.sol:12, or the standard and its clause".to_string(),
                                    basis: EvidenceBasis::Documented,
                                    code_path: "src/Contract.sol:40 and what the code does there instead.".to_string(),
                                },
                            )]),
                        },
                        note: "Return one JSON object with exactly this shape. Keep schema unchanged and replace the example's rationale, files, generator metadata, hypotheses, and evidence. Forge runs every test in the files on every seed. It keeps a file only if all of its tests pass on every seed, and it reports the tests that fail on every seed as possible bugs. Each file path must be new relative to current_candidate. The generator object is optional. List every hypothesis of the audit in hypotheses, with its result. Add one evidence entry, keyed by Contract::test, for every test that fails against the current implementation.",
                    },
                };
                fs::write(&prompt_path, serde_json::to_vec_pretty(&prompt)?)?;
                self.rejected_sources.clear();

                let output = Command::new(&generator)
                    .args(&self.args.generator_arg)
                    .arg(&prompt_path)
                    .arg(&candidate_path)
                    .current_dir(&self.config.root)
                    .output()
                    .wrap_err("failed to invoke generator")?;
                if !output.status.success() {
                    error = Some(eyre!(
                        "generator failed in round {round} ({}): {}",
                        output.status,
                        stderr(&output)
                    ));
                    break 'phases;
                }
                let mut record = Round { phase, round, ..Default::default() };
                match read_candidate(&candidate_path, &self.generated_tests) {
                    Ok(proposal) => {
                        record.generator = proposal.generator.clone();
                        record.rationale = Some(proposal.rationale.clone());
                        record.audit = AuditSummary::new(&proposal.hypotheses);
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
                self.rounds.push(record);
            }
        }
        fs::write(self.cache_root.join("rounds.json"), serde_json::to_vec_pretty(&self.rounds)?)?;
        self.report(&journal)?;
        error.map_or(Ok(()), Err)
    }

    /// Evaluates a proposal and keeps its files whose tests pass on every seed. In the mutation
    /// phase, the kept files must also kill a survivor on every seed.
    ///
    /// Problems with the proposal are errors that become rejection reasons; failures of Forge
    /// itself also stop only this round.
    fn evaluate(&mut self, proposal: &Candidate, record: &mut Round) -> Result<()> {
        let candidate = merge_candidate(self.current.as_ref(), proposal, &self.generated_tests)?;
        for file in &proposal.files {
            let path = self.config.root.join(&file.path);
            ensure!(!path.exists(), "candidate would overwrite {}", file.path.display());
            // A symlinked directory on the path must not lead outside the project.
            if let Some(existing) = path.ancestors().find(|ancestor| ancestor.exists()) {
                workspace::ensure_within_root(
                    &self.config.root,
                    existing,
                    "generated test",
                    &file.path,
                )?;
            }
        }
        let digest = keccak256(serde_json::to_vec(&candidate)?).to_string();
        let cache = self.cache_root.join(&digest);
        fs::create_dir_all(&cache)?;
        fs::write(cache.join("candidate.json"), serde_json::to_vec_pretty(&candidate)?)?;
        record.candidate_digest = Some(digest);

        let workspace = candidate_workspace(self.config, &candidate.files, &self.targets)?;
        let run = self.run_tests(workspace.path(), &candidate)?;
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
        let mut kept = required_candidate_files(
            self.config,
            workspace.path(),
            &candidate.files,
            &passing_files,
        )?;
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

        if record.phase == Phase::Mutation && !kept.is_empty() {
            let selection = self.baseline_tests.iter().chain(&tests).cloned().collect::<Vec<_>>();
            let results = self.run_mutations(workspace.path(), &selection, &cache)?;
            let killed = killed_survivors(&self.mutants, &results);
            for (before, after) in self.mutants.iter().zip(&results) {
                // Generated files only add tests, so this comes from timeouts, adaptive span
                // skipping, or timing-dependent tests.
                let revived =
                    not_killed(&after.output).difference(&not_killed(&before.output)).count();
                if revived > 0 {
                    record.warnings.push(format!(
                        "the candidate run did not kill {revived} mutant(s) that the previous run killed on seed {}",
                        before.seed
                    ));
                }
            }
            if killed.is_empty() {
                record
                    .reasons
                    .push("the passing files kill no surviving mutant on every seed".to_string());
                kept.clear();
            } else {
                record.killed_survivors = Some(killed.len());
                self.killed += killed.len();
                self.mutants = results;
            }
        }
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

    /// Runs mutation testing on the targets with the existing tests and the kept tests.
    fn start_mutation_phase(&mut self) -> Result<()> {
        let filter = self
            .args
            .match_contract
            .iter()
            .flat_map(|contract| ["--match-contract", contract.as_str()]);
        let output = Command::new(&self.forge)
            .current_dir(&self.config.root)
            .args(["test", "--list", "--json"])
            .args(filter)
            .output()
            .wrap_err("failed to list tests")?;
        ensure!(output.status.success(), "failed to list tests: {}", stderr(&output));
        let files = serde_json::from_slice::<BTreeMap<String, BTreeMap<String, Vec<String>>>>(
            &output.stdout,
        )
        .wrap_err("invalid test list JSON")?;
        self.baseline_tests = files
            .into_iter()
            .flat_map(|(path, contracts)| {
                contracts.into_iter().flat_map(move |(contract, tests)| {
                    let contract = format!("{path}:{contract}");
                    tests
                        .into_iter()
                        .map(move |test| RerunFailure { contract: contract.clone(), test })
                })
            })
            .collect();
        for path in self.baseline_tests.iter().map(test_path).collect::<HashSet<_>>() {
            let source = fs::read_to_string(self.config.root.join(&path))?;
            ensure!(
                !has_inline_fuzz_seed(&source),
                "test {} sets a seed in inline config",
                path.display()
            );
        }
        let files = self.current.as_ref().map_or(&[][..], |current| &current.files);
        let workspace = candidate_workspace(self.config, files, &self.targets)?;
        let selection =
            self.baseline_tests.iter().chain(&self.current_tests).cloned().collect::<Vec<_>>();
        self.mutants =
            self.run_mutations(workspace.path(), &selection, &self.cache_root.join("baseline"))?;
        Ok(())
    }

    /// Prints the possible bugs and writes the kept files to the project.
    fn report(&self, journal: &Path) -> Result<()> {
        let mut possible_bugs = self
            .rounds
            .iter()
            .flat_map(|round| {
                let digest = round.candidate_digest.as_deref().unwrap_or_default();
                let path = self.cache_root.join(digest);
                let path = path.strip_prefix(&self.config.root).unwrap_or(&path).display();
                let note = format!("fails on every seed; candidate: {path}");
                round.possible_bugs.iter().map(move |failure| (failure, note.clone()))
            })
            .collect::<Vec<_>>();
        // A generator can remove a failing test after a check. Report the failures that no round
        // reported and that the kept files do not hold.
        let journal_failures = read_check_journal(journal)?;
        let journal = journal.strip_prefix(&self.config.root).unwrap_or(journal);
        let note =
            format!("seen in a generator check, then removed; journal: {}", journal.display());
        for failure in &journal_failures {
            let reported = possible_bugs.iter().any(|(reported, _)| reported.test == failure.test);
            let name =
                failure.test.rsplit_once("::").map_or(failure.test.as_str(), |(_, name)| name);
            let kept = self.current.as_ref().is_some_and(|current| {
                current.files.iter().any(|file| {
                    file.path == failure.path && file.content.contains(&format!("function {name}("))
                })
            });
            if !reported && !kept {
                possible_bugs.push((failure, note.clone()));
            }
        }
        possible_bugs.sort_by(|(a, _), (b, _)| a.report_key().cmp(&b.report_key()));
        for (failure, note) in &possible_bugs {
            sh_println!("{failure}\n  {note}")?;
        }
        for round in &self.rounds {
            if let Some(audit) = &round.audit {
                let phase = match round.phase {
                    Phase::Audit => "audit",
                    Phase::Mutation => "mutation",
                };
                sh_println!("{phase} round {}: {audit}", round.round)?;
            }
        }

        let Some(current) = &self.current else {
            sh_println!("no generated test file was kept")?;
            return Ok(());
        };
        // Evaluation rejects paths that exist or leave the project, so this only adds files.
        for file in &current.files {
            let path = self.config.root.join(&file.path);
            fs::create_dir_all(path.parent().expect("candidate path has a parent"))?;
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .and_then(|mut target| target.write_all(file.content.as_bytes()))
                .wrap_err_with(|| format!("failed to write {}", file.path.display()))?;
        }
        sh_println!("added {} generated test file(s):", current.files.len())?;
        for file in &current.files {
            sh_println!("  {}", file.path.display())?;
        }
        if self.args.mutation {
            let remaining = survivors(&self.config.root, &self.mutants).len();
            sh_println!(
                "the mutation phase killed {} survivor(s); {remaining} remain",
                self.killed
            )?;
        }
        Ok(())
    }

    /// Runs the `--check` command.
    fn check(&self, path: &Path) -> Result<()> {
        let candidate = read_candidate(path, &self.generated_tests)?;
        let workspace = candidate_workspace(self.config, &candidate.files, &[])?;
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
            fs::create_dir_all(&self.cache_root)?;
            // One write per check keeps the lines of concurrent checks apart.
            fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.cache_root.join(CHECK_JOURNAL))
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
        for seed in &self.args.seed {
            let mut command = self.forge_command(workspace, seed);
            command.args(["test", "--json", "--match-path", &format!("{{{paths}}}")]);
            command.args(SELECT_ALL_TESTS);
            let output = command.output().wrap_err("failed to run candidate tests")?;
            if !output.status.success() && output.stdout.is_empty() {
                reasons.push(format!("candidate tests failed on seed {seed}: {}", stderr(&output)));
                continue;
            }
            let results = test_results(&output)?;
            if results.is_empty() {
                reasons.push(format!("candidate files contain no runnable tests on seed {seed}"));
            }
            for (test, status, reason) in results {
                let name = test_display_name(&test);
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
        let mut run = TestRun { reasons, ..Default::default() };
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
        Ok(run)
    }

    /// Runs mutation testing for every seed concurrently with exactly the selected tests. Each
    /// seed builds into its own directories and writes its stderr to `log_dir`.
    fn run_mutations(
        &self,
        workspace: &Path,
        selection: &[RerunFailure],
        log_dir: &Path,
    ) -> Result<Vec<SeedMutation>> {
        fs::create_dir_all(log_dir)?;
        // Seeds run concurrently, so each seed gets an equal share of the cores. Mutation results
        // do not depend on the worker count.
        let jobs = std::thread::available_parallelism()
            .map_or(1, |cores| (cores.get() / self.args.seed.len()).max(1));
        std::thread::scope(|scope| {
            let runs = self
                .args
                .seed
                .iter()
                .map(|seed| {
                    scope.spawn(move || {
                        let seed_dir = seed_dir(self.config, workspace, seed);
                        fs::create_dir_all(&seed_dir)?;
                        // `--rerun` selects exact contract and test pairs; the broad patterns keep
                        // configured filters from removing any of them.
                        fs::write(
                            seed_dir.join("test-failures"),
                            serde_json::to_vec(
                                &serde_json::json!({ "version": 1, "failures": selection }),
                            )?,
                        )?;
                        let mut command = self.forge_command(workspace, seed);
                        command
                            .env("FOUNDRY_OUT", seed_dir.join("out"))
                            .env("FOUNDRY_CACHE_PATH", seed_dir.join("cache"))
                            .args(["test", "--json", "--mutate"])
                            .args(&self.targets)
                            .args(["--rerun", "--match-path", "**"])
                            .args(SELECT_ALL_TESTS)
                            .args(["--mutation-jobs", &jobs.to_string()]);
                        if let Some(timeout) = self.args.mutation_timeout {
                            command.args(["--mutation-timeout", &timeout.to_string()]);
                        }
                        let run = command.output().wrap_err("failed to run mutation testing")?;
                        fs::write(log_dir.join(format!("mutation-{seed:#x}.log")), &run.stderr)?;
                        fs::write(log_dir.join(format!("mutation-{seed:#x}.json")), &run.stdout)?;
                        ensure!(
                            run.status.success(),
                            "mutation testing failed for seed {seed}: {}",
                            stderr(&run)
                        );
                        Ok(SeedMutation {
                            seed: *seed,
                            output: serde_json::from_slice(&run.stdout).wrap_err_with(|| {
                                format!("invalid mutation JSON for seed {seed}")
                            })?,
                        })
                    })
                })
                .collect::<Vec<_>>();
            runs.into_iter().map(|run| run.join().expect("mutation thread panicked")).collect()
        })
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
            .env_remove("FOUNDRY_TEST");
        command
    }
}

/// Copies the project into a temporary workspace and adds the files.
fn candidate_workspace(
    config: &Config,
    files: &[CandidateFile],
    targets: &[PathBuf],
) -> Result<TempDir> {
    let workspace = tempfile::Builder::new().prefix("forge-properties-").tempdir()?;
    workspace::copy_project(config, workspace.path())?;
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
    // Mutation testing copies this workspace again. Its library links must point into this
    // workspace, or the nested copy reaches the project through two paths. `copy_project` also
    // links `node_modules` and `dependencies` when they are not in `libs`.
    for lib in config
        .libs
        .iter()
        .map(PathBuf::as_path)
        .chain(["node_modules", "dependencies"].map(Path::new))
    {
        let source = config.root.join(lib);
        let Ok(relative) = source.strip_prefix(&config.root) else { continue };
        let target = workspace.path().join(relative);
        if !workspace::is_safe_relative_path(relative)
            || !source.is_dir()
            || !fs::symlink_metadata(&target).is_ok_and(|metadata| metadata.is_symlink())
        {
            continue;
        }
        #[cfg(unix)]
        fs::remove_file(&target)?;
        #[cfg(windows)]
        fs::remove_dir(&target)?;
        fs::create_dir(&target)?;
        for entry in fs::read_dir(&source)? {
            let entry = entry?;
            let destination = target.join(entry.file_name());
            if !entry.path().is_dir() {
                fs::copy(entry.path(), destination)?;
            } else if targets.iter().any(|path| path.starts_with(relative.join(entry.file_name())))
                || workspace::symlink_dir(&entry.path(), &destination).is_err()
            {
                // Mutation testing accepts only targets inside its workspace.
                workspace::copy_dir_recursive(&entry.path(), &destination)?;
            }
        }
    }
    for file in files {
        let path = workspace.path().join(&file.path);
        fs::create_dir_all(path.parent().expect("candidate path has a parent"))?;
        fs::write(path, &file.content)?;
    }
    Ok(workspace)
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
fn test_results(output: &Output) -> Result<Vec<(RerunFailure, TestStatus, Option<String>)>> {
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
                results.push((key(test), status, reason));
                continue;
            }
            for predicate in &result.invariant_predicate_results {
                let failure = result
                    .invariant_failures
                    .iter()
                    .find(|failure| failure.predicate_name() == Some(predicate.name.as_str()));
                let reason =
                    predicate.reason.as_deref().or(failure.map(|failure| failure.reason()));
                let reason = reason.map(|reason| {
                    failure_reason(reason, failure.and_then(|failure| failure.counterexample()))
                });
                results.push((key(&predicate.name), predicate.status, reason));
            }
            // One campaign result per seed, so the count of failing seeds stays correct.
            if !handler_failures.is_empty() {
                results.push((key(test), TestStatus::Failure, Some(handler_failures.join("; "))));
            }
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
    let generated_tests = config.test.join("generated");
    let target_stems = targets.iter().filter_map(|path| path.file_stem()).collect::<Vec<_>>();
    let mut tests = graph
        .nodes
        .iter()
        .filter(|node| {
            node.path().starts_with(&config.test)
                && !node.path().starts_with(&generated_tests)
                && node.path().extension().is_some_and(|extension| extension == "sol")
                && graph.imports(node.path()).iter().any(|import| target_files.contains(*import))
        })
        .map(|node| {
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
    context
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

/// Returns the surviving mutants of all seeds, the ones that survive every seed first, with the
/// source lines around each.
fn survivors(root: &Path, results: &[SeedMutation]) -> Vec<Survivor> {
    let mut seeds = BTreeMap::<MutantId, Vec<U256>>::new();
    for result in results {
        for id in mutant_ids(&result.output.survived_mutants) {
            seeds.entry(id).or_default().push(result.seed);
        }
    }
    let mut sources = HashMap::<String, Option<String>>::new();
    let mut survivors = seeds
        .into_iter()
        .map(|((path, line, column, original, mutant), seeds)| {
            let source = sources
                .entry(path.clone())
                .or_insert_with(|| fs::read_to_string(root.join(&path)).ok());
            let source_context = source.as_ref().map(|source| {
                source
                    .lines()
                    .enumerate()
                    .skip(line.saturating_sub(4))
                    .take(7)
                    .map(|(index, text)| format!("{:>4}: {text}", index + 1))
                    .collect::<Vec<_>>()
                    .join("\n")
            });
            Survivor {
                path: path.into(),
                line,
                column,
                original,
                mutant,
                survives_all_seeds: seeds.len() == results.len(),
                seeds,
                source_context,
            }
        })
        .collect::<Vec<_>>();
    survivors.sort_by_key(|survivor| !survivor.survives_all_seeds);
    survivors
}

/// Returns the survivors of `before` that `after` kills on every seed.
///
/// A mutant is killed only if it did not survive, time out, become invalid, or get skipped:
/// adaptive span skipping can remove a survivor without a kill.
fn killed_survivors(before: &[SeedMutation], after: &[SeedMutation]) -> BTreeSet<MutantId> {
    let not_killed = after.iter().map(|result| not_killed(&result.output)).collect::<Vec<_>>();
    before
        .iter()
        .flat_map(|result| mutant_ids(&result.output.survived_mutants))
        .filter(|id| not_killed.iter().all(|not_killed| !not_killed.contains(id)))
        .collect()
}

/// Returns the mutants that a run did not kill.
fn not_killed(output: &MutationJsonOutput) -> BTreeSet<MutantId> {
    [
        &output.survived_mutants,
        &output.timed_out_mutants,
        &output.invalid_mutants,
        &output.skipped_mutants,
    ]
    .into_iter()
    .flat_map(mutant_ids)
    .collect()
}

fn mutant_ids(mutants: &BTreeMap<String, Vec<SurvivedMutantJson>>) -> BTreeSet<MutantId> {
    mutants
        .iter()
        .flat_map(|(path, mutants)| {
            mutants.iter().map(|mutant| {
                (
                    path.clone(),
                    mutant.line,
                    mutant.column,
                    mutant.original.clone(),
                    mutant.mutant.clone(),
                )
            })
        })
        .collect()
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
