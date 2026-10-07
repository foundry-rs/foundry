//! The `forge properties` command: generate test properties and keep only verified ones.

use crate::{
    cmd::test::RerunFailure,
    mutation::MutationJsonOutput,
    result::{TestResult, TestStatus},
    workspace,
};
use alloy_primitives::{U256, keccak256};
use clap::Parser;
use eyre::{Context, Result, ensure, eyre};
use foundry_common::sh_println;
use foundry_compilers::{
    Graph,
    compilers::multi::{MultiCompilerLanguage, MultiCompilerParser},
};
use foundry_config::Config;
use foundry_evm::fuzz::CounterExample;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    ffi::OsString,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::LazyLock,
};
use tempfile::TempDir;
const CANDIDATE_SCHEMA: &str = "foundry/properties-candidate-v1";
const PROMPT_SCHEMA: &str = "foundry/properties-prompt-v1";
const MAX_CANDIDATE_FILES: usize = 8;
const MAX_CANDIDATE_BYTES: usize = 256 * 1024;
// Bound each source-context section so it cannot dominate the prompt.
const MAX_PROMPT_SOURCE_BYTES: usize = 16 * 1024;
const MAX_PROJECT_CONTEXT_FILES: usize = 2;
/// Overrides configured test filters, so they cannot hide generated tests. `$^` matches no
/// contract or test name, and no project file is under the absolute path
/// `/forge-properties-none`.
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

/// Rules for the property generator, one per bullet in `guidance.md`.
static GUIDANCE: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    include_str!("properties/guidance.md")
        .lines()
        .filter_map(|line| line.strip_prefix("- "))
        .collect()
});

/// Generate test properties and keep only reproducible mutation-coverage improvements.
#[derive(Clone, Debug, Parser)]
pub struct PropertiesArgs {
    /// Root of the Foundry project.
    #[arg(long, default_value = ".", value_name = "PATH")]
    root: PathBuf,

    /// Production source files passed to mutation testing.
    #[arg(long, required_unless_present = "check", num_args = 1.., value_name = "PATH")]
    mutate: Vec<PathBuf>,

    /// Markdown instructions supplied to the property generator.
    #[arg(long, required_unless_present = "check", value_name = "PATH")]
    brief: Option<PathBuf>,

    /// Executable that receives PROMPT_JSON and OUTPUT_JSON as its final arguments.
    #[arg(long, required_unless_present = "check", value_name = "PATH")]
    generator: Option<PathBuf>,

    /// Compile and run the tests of a candidate JSON file on every seed, without mutation testing,
    /// and print the result as JSON. Generators use this to check a candidate before returning it.
    #[arg(long, value_name = "CANDIDATE_JSON", conflicts_with_all = ["mutate", "brief", "generator"])]
    check: Option<PathBuf>,

    /// Argument passed to the generator before PROMPT_JSON and OUTPUT_JSON.
    #[arg(long, allow_hyphen_values = true, value_name = "ARG")]
    generator_arg: Vec<OsString>,

    /// Deterministic seeds used to evaluate every candidate. Supply at least two.
    #[arg(long, required = true, action = clap::ArgAction::Append, value_name = "SEED")]
    seed: Vec<U256>,

    /// Maximum number of proposal rounds.
    #[arg(long, default_value_t = 1, value_name = "N")]
    rounds: usize,

    /// Best-effort timeout for each mutant.
    #[arg(long, value_name = "SECONDS")]
    mutation_timeout: Option<u32>,

    /// Restrict mutation evaluation to matching test contracts.
    #[arg(long, value_name = "REGEX")]
    match_contract: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Candidate {
    schema: String,
    rationale: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    generator: Option<GeneratorMetadata>,
    files: Vec<CandidateFile>,
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
    project: &'a Path,
    brief: &'a str,
    guidance: &'static [&'static str],
    mutate: &'a [PathBuf],
    seeds: &'a [U256],
    baseline: &'a [PromptMutation<'a>],
    current_results: &'a [PromptMutation<'a>],
    mutation_gaps: &'a [MutationGap],
    project_context: &'a ProjectContext,
    #[serde(skip_serializing_if = "Option::is_none")]
    current_candidate: Option<&'a Candidate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_rejected_sources: Option<&'a [PromptSource]>,
    previous_feedback: &'a [ProposalFeedback],
    output_contract: OutputContract,
}

#[derive(Debug, Serialize)]
struct ProjectContext {
    remappings: Vec<String>,
    /// Whether each external call from a test contract runs as its own transaction.
    isolate: bool,
    evm_version: String,
    target_sources: Vec<PromptSource>,
    test_sources: Vec<PromptSource>,
}

#[derive(Debug, Serialize)]
struct PromptSource {
    path: PathBuf,
    content: String,
    truncated: bool,
}

#[derive(Debug, Serialize)]
struct OutputContract {
    schema: &'static str,
    allowed_path_prefix: PathBuf,
    maximum_files: usize,
    maximum_total_bytes: usize,
    example: Candidate,
    note: &'static str,
    /// Command that compiles and runs a candidate file without mutation testing.
    check_command: Vec<String>,
}

#[derive(Clone, Debug)]
struct SeedMutation {
    seed: U256,
    output: MutationJsonOutput,
}

#[derive(Debug, Serialize)]
struct PromptMutation<'a> {
    seed: &'a U256,
    summary: &'a crate::mutation::MutationSummaryJson,
}

#[derive(Debug, Serialize)]
struct MutationGap {
    path: PathBuf,
    line: usize,
    column: usize,
    original: String,
    mutant: String,
    seeds: Vec<U256>,
    survives_all_seeds: bool,
    /// Why the mutant is likely equivalent to the original, if a known pattern matches.
    #[serde(skip_serializing_if = "Option::is_none")]
    likely_equivalent: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_context: Option<String>,
}

#[derive(Debug, Serialize)]
struct ProposalFeedback {
    candidate_digest: String,
    accepted: bool,
    rationale: Option<String>,
    reasons: Vec<String>,
    candidate_results: Vec<MutationResultSummary>,
    resolved_survivors: usize,
    newly_resolved_survivors: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    possible_bugs: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
struct MutationResultSummary {
    seed: U256,
    summary: crate::mutation::MutationSummaryJson,
}

#[derive(Debug, Serialize)]
struct Evaluation {
    candidate_digest: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    generator: Option<GeneratorMetadata>,
    accepted: bool,
    reasons: Vec<String>,
    baseline: Vec<MutationResultSummary>,
    candidate: Vec<MutationResultSummary>,
    resolved_survivors: usize,
    newly_resolved_survivors: usize,
    /// Candidate tests that fail on every seed against the current implementation. Such a
    /// property is either incorrect or exposes a bug, so it is reported for review.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    possible_bugs: Vec<String>,
    /// Whether this candidate was kept after an earlier round reported a possible bug. It can
    /// encode the reported behavior, so it needs review together with that report.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    after_possible_bug: bool,
}

impl Evaluation {
    fn rejected(
        candidate_digest: String,
        generator: Option<GeneratorMetadata>,
        baseline: &[SeedMutation],
        reason: impl std::fmt::Display,
    ) -> Self {
        Self {
            candidate_digest,
            generator,
            accepted: false,
            reasons: vec![reason.to_string()],
            baseline: mutation_result_summaries(baseline),
            candidate: vec![],
            resolved_survivors: 0,
            newly_resolved_survivors: 0,
            possible_bugs: vec![],
            after_possible_bug: false,
        }
    }

    fn feedback(&self, candidate: Option<&Candidate>) -> ProposalFeedback {
        ProposalFeedback {
            candidate_digest: self.candidate_digest.clone(),
            accepted: self.accepted,
            rationale: candidate.map(|candidate| candidate.rationale.clone()),
            reasons: self.reasons.clone(),
            candidate_results: self.candidate.clone(),
            resolved_survivors: self.resolved_survivors,
            newly_resolved_survivors: self.newly_resolved_survivors,
            possible_bugs: self.possible_bugs.clone(),
        }
    }
}

type MutationIdentity = (String, usize, usize, String, String);

impl PropertiesArgs {
    pub fn run(self) -> Result<()> {
        ensure!(self.seed.len() >= 2, "at least two --seed values are required");
        ensure!(
            self.seed.iter().collect::<HashSet<_>>().len() == self.seed.len(),
            "--seed values must be distinct"
        );
        ensure!(self.rounds > 0, "--rounds must be greater than zero");
        for path in &self.mutate {
            ensure!(
                workspace::is_safe_relative_path(path),
                "mutation paths must be project-relative paths"
            );
        }
        if let Some(timeout) = self.mutation_timeout {
            ensure!(timeout > 0, "--mutation-timeout must be greater than zero");
        }
        let root = self.root.canonicalize().wrap_err("failed to resolve project root")?;
        let config = Config::load_with_root(&root)?.sanitized();
        let generated_tests =
            workspace::relative_to_root(&config.root, &config.test).join("generated");
        workspace::ensure_safe_relative_path(&generated_tests, "generated test", &config.test)?;
        let forge = std::env::current_exe().wrap_err("failed to resolve Forge executable")?;
        if let Some(path) = &self.check {
            return self.check(&config, &forge, &generated_tests, path);
        }
        let (Some(brief), Some(generator)) = (&self.brief, &self.generator) else {
            eyre::bail!("--brief and --generator are required");
        };
        let brief = fs::read_to_string(brief).wrap_err("failed to read campaign brief")?;
        let generator = generator.canonicalize().wrap_err("failed to resolve generator")?;
        let contract_filter = self
            .match_contract
            .as_deref()
            .or_else(|| config.contract_pattern.as_ref().map(|pattern| pattern.as_str()));
        let contract_filter_args = contract_filter
            .map(|contract| vec!["--match-contract".to_string(), contract.to_string()])
            .unwrap_or_default();
        let baseline =
            self.run_mutations(&forge, &config, &config.root, &contract_filter_args, None)?;
        let baseline_tests = list_tests(&forge, &config.root, &contract_filter_args)?;
        let project_context = project_context(&config, &self.mutate);
        let prompt_baseline = baseline
            .iter()
            .map(|result| PromptMutation { seed: &result.seed, summary: &result.output.summary })
            .collect::<Vec<_>>();
        let cache_root = config.cache_path.join("properties");
        fs::create_dir_all(&cache_root)?;

        let check_command = [forge.as_os_str(), "properties".as_ref(), "--root".as_ref()]
            .into_iter()
            .chain([config.root.as_os_str(), "--check".as_ref(), "CANDIDATE_JSON".as_ref()])
            .map(|arg| arg.to_string_lossy().into_owned())
            .chain(self.seed.iter().flat_map(|seed| ["--seed".to_string(), format!("{seed:#x}")]))
            .collect::<Vec<_>>();
        let mut evaluations = Vec::new();
        let mut review_paths = HashSet::new();
        let mut feedback = Vec::new();
        let mut current_candidate = None::<Candidate>;
        let mut last_rejected_sources = None::<Vec<PromptSource>>;
        let mut current_results = baseline.clone();
        let mut resolved_survivors = BTreeSet::new();
        let mut generator_error = None;
        for round in 1..=self.rounds {
            let round_dir = tempfile::Builder::new().prefix("forge-properties-").tempdir()?;
            let prompt_path = round_dir.path().join("prompt.json");
            let candidate_path = round_dir.path().join("candidate.json");
            let prompt_current_results = current_results
                .iter()
                .map(|result| PromptMutation {
                    seed: &result.seed,
                    summary: &result.output.summary,
                })
                .collect::<Vec<_>>();
            let mutation_gaps = mutation_gaps(&config.root, &current_results);
            let retained_files =
                current_candidate.as_ref().map_or(0, |candidate| candidate.files.len());
            let retained_bytes = current_candidate
                .as_ref()
                .map_or(0, |candidate| candidate.files.iter().map(|file| file.content.len()).sum());
            let example_contract = format!("GeneratedRound{round}Test");
            let example_path = generated_tests.join(format!("GeneratedRound{round}.t.sol"));
            let prompt = GeneratorPrompt {
                schema: PROMPT_SCHEMA,
                round,
                project: &config.root,
                brief: &brief,
                guidance: &GUIDANCE,
                mutate: &self.mutate,
                seeds: &self.seed,
                baseline: &prompt_baseline,
                current_results: &prompt_current_results,
                mutation_gaps: &mutation_gaps,
                project_context: &project_context,
                current_candidate: current_candidate.as_ref(),
                last_rejected_sources: last_rejected_sources.as_deref(),
                previous_feedback: &feedback,
                output_contract: OutputContract {
                    schema: CANDIDATE_SCHEMA,
                    allowed_path_prefix: generated_tests.clone(),
                    maximum_files: MAX_CANDIDATE_FILES - retained_files,
                    maximum_total_bytes: MAX_CANDIDATE_BYTES - retained_bytes,
                    check_command: check_command.clone(),
                    example: Candidate {
                        schema: CANDIDATE_SCHEMA.to_string(),
                        rationale: "Explain the concrete input or sequence that distinguishes the original from the mutant.".to_string(),
                        generator: Some(GeneratorMetadata {
                            agent: "generator name".to_string(),
                            model: "model name".to_string(),
                        }),
                        files: vec![CandidateFile {
                            path: example_path,
                            content: format!(
                                "pragma solidity ^0.8.0;\ncontract {example_contract} {{\n    function testProperty() external {{}}\n}}\n"
                            ),
                        }],
                    },
                    note: "Return one JSON object with exactly this shape. Keep schema unchanged; replace the structural example's rationale, files, and generator metadata with a useful property for this project. Forge runs every test in the files. Generated assertions are candidates for human review, not proofs. Each file path must be new relative to current_candidate. The generator object is optional.",
                },
            };
            fs::write(&prompt_path, serde_json::to_vec_pretty(&prompt)?)?;
            last_rejected_sources = None;

            let output = Command::new(&generator)
                .args(&self.generator_arg)
                .arg(&prompt_path)
                .arg(&candidate_path)
                .current_dir(&config.root)
                .output()
                .wrap_err("failed to invoke generator")?;
            if !output.status.success() {
                generator_error = Some(eyre!(
                    "generator failed in round {round} ({}): {}",
                    output.status,
                    stderr(&output)
                ));
                break;
            }
            let evaluation = match read_candidate(&candidate_path, &generated_tests) {
                Ok(proposal) => match accumulate_candidate(
                    current_candidate.as_ref(),
                    &proposal,
                    &generated_tests,
                ) {
                    Ok(candidate) => {
                        let (mut evaluation, candidate_results, newly_resolved) = self.evaluate(
                            &config,
                            &forge,
                            &baseline,
                            &baseline_tests,
                            &current_results,
                            &resolved_survivors,
                            &candidate,
                        )?;
                        evaluation.after_possible_bug = evaluation.accepted
                            && evaluations
                                .iter()
                                .any(|earlier: &Evaluation| !earlier.possible_bugs.is_empty());
                        if evaluation.after_possible_bug {
                            review_paths
                                .extend(proposal.files.iter().map(|file| file.path.clone()));
                        }
                        let candidate_cache = cache_root.join(&evaluation.candidate_digest);
                        fs::create_dir_all(&candidate_cache)?;
                        fs::write(
                            candidate_cache.join("candidate.json"),
                            serde_json::to_vec_pretty(&candidate)?,
                        )?;
                        fs::write(
                            candidate_cache.join("evaluation.json"),
                            serde_json::to_vec_pretty(&evaluation)?,
                        )?;
                        if evaluation.accepted {
                            current_candidate = Some(candidate);
                            current_results = candidate_results;
                            resolved_survivors.extend(newly_resolved);
                        } else {
                            last_rejected_sources = Some(prompt_sources(
                                proposal
                                    .files
                                    .iter()
                                    .map(|file| (file.path.as_path(), file.content.as_str())),
                            ));
                        }
                        feedback.push(evaluation.feedback(Some(&proposal)));
                        evaluation
                    }
                    Err(error) => {
                        let evaluation = Evaluation::rejected(
                            format!("round-{round}"),
                            proposal.generator.clone(),
                            &baseline,
                            error,
                        );
                        last_rejected_sources = Some(prompt_sources(
                            proposal
                                .files
                                .iter()
                                .map(|file| (file.path.as_path(), file.content.as_str())),
                        ));
                        feedback.push(evaluation.feedback(Some(&proposal)));
                        evaluation
                    }
                },
                Err(error) => {
                    let evaluation =
                        Evaluation::rejected(format!("round-{round}"), None, &baseline, error);
                    feedback.push(evaluation.feedback(None));
                    evaluation
                }
            };
            evaluations.push(evaluation);
        }

        fs::write(cache_root.join("rounds.json"), serde_json::to_vec_pretty(&evaluations)?)?;
        for evaluation in &evaluations {
            let path = cache_root.join(&evaluation.candidate_digest);
            let path = path.strip_prefix(&config.root).unwrap_or(&path);
            for possible_bug in &evaluation.possible_bugs {
                sh_println!(
                    "possible bug: {possible_bug}\n  the property fails on every seed against the current implementation; candidate: {}",
                    path.display()
                )?;
            }
        }
        if let Some(candidate) = current_candidate {
            // Evaluation rejects paths that already exist or leave the project, so this adds files
            // without overwriting.
            for file in &candidate.files {
                let path = config.root.join(&file.path);
                fs::create_dir_all(path.parent().expect("candidate path has a parent"))?;
                fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)
                    .and_then(|mut target| target.write_all(file.content.as_bytes()))
                    .wrap_err_with(|| format!("failed to write {}", file.path.display()))?;
            }
            sh_println!(
                "added {} generated test file(s) that reproducibly resolve {} mutation survivor(s):",
                candidate.files.len(),
                resolved_survivors.len(),
            )?;
            for file in &candidate.files {
                if review_paths.contains(&file.path) {
                    sh_println!(
                        "  {} (kept after a possible bug was reported; review it together with that report)",
                        file.path.display()
                    )?;
                } else {
                    sh_println!("  {}", file.path.display())?;
                }
            }
        } else {
            sh_println!("no candidate reproducibly resolved a mutation survivor")?;
        }
        generator_error.map_or(Ok(()), Err)
    }

    /// Evaluates a candidate. Problems with the candidate become rejection reasons; failures of
    /// Forge itself are errors.
    #[allow(clippy::too_many_arguments)]
    fn evaluate(
        &self,
        config: &Config,
        forge: &Path,
        baseline: &[SeedMutation],
        baseline_tests: &[RerunFailure],
        current_results: &[SeedMutation],
        previously_resolved: &BTreeSet<MutationIdentity>,
        candidate: &Candidate,
    ) -> Result<(Evaluation, Vec<SeedMutation>, BTreeSet<MutationIdentity>)> {
        let digest = keccak256(serde_json::to_vec(candidate)?).to_string();
        let conflict = candidate.files.iter().find_map(|file| {
            let path = config.root.join(&file.path);
            if path.exists() {
                return Some(format!("candidate would overwrite {}", file.path.display()));
            }
            // A symlinked directory on the path must not lead outside the project.
            let existing = path.ancestors().find(|ancestor| ancestor.exists())?;
            workspace::ensure_within_root(&config.root, existing, "generated test", &file.path)
                .err()
                .map(|err| err.to_string())
        });
        if let Some(reason) = conflict {
            let evaluation =
                Evaluation::rejected(digest, candidate.generator.clone(), baseline, reason);
            return Ok((evaluation, vec![], BTreeSet::new()));
        }
        let candidate_workspace = candidate_workspace(config, candidate, &self.mutate)?;
        let (mut reasons, possible_bugs, tests) =
            self.check_candidate_tests(forge, candidate_workspace.path(), config, candidate)?;

        let candidate_results = if reasons.is_empty() {
            // Run exactly the baseline tests and the generated tests. Widening filters instead
            // could also select existing tests that the baseline excluded.
            let selection = baseline_tests.iter().chain(&tests).cloned().collect::<Vec<_>>();
            let results = self.run_mutations(
                forge,
                config,
                candidate_workspace.path(),
                &[],
                Some(&selection),
            )?;
            for (before, after) in current_results.iter().zip(&results) {
                // Kills are inferred from absence, so both runs must name mutants the same way.
                let population = not_killed_identities(&before.output);
                ensure!(
                    not_killed_identities(&after.output).is_subset(&population),
                    "mutation results on seed {} name mutants that the baseline does not have",
                    before.seed
                );
                // Invalid is an execution outcome, not part of the population: a stronger test can
                // expose a previously skipped mutant that does not compile. Only a survivor that
                // became invalid would be miscounted as resolved.
                let changed_population = before.output.summary.total != after.output.summary.total;
                let invalidated_survivor = !survivor_identities(&before.output)
                    .is_disjoint(&mutant_identities(&after.output.invalid_mutants));
                let introduced_timeout =
                    after.output.timed_out_mutants.iter().any(|(path, mutants)| {
                        before.output.timed_out_mutants.get(path).is_none_or(|baseline| {
                            mutants.iter().any(|mutant| !baseline.contains(mutant))
                        })
                    });
                if changed_population {
                    reasons.push(format!(
                        "candidate changed the mutant population on seed {}",
                        before.seed
                    ));
                    break;
                }
                if invalidated_survivor {
                    reasons.push(format!(
                        "candidate made a surviving mutant invalid on seed {}",
                        before.seed
                    ));
                    break;
                }
                if introduced_timeout {
                    reasons.push(format!(
                        "candidate introduced a timed-out mutant on seed {}",
                        before.seed
                    ));
                    break;
                }
            }
            results
        } else {
            vec![]
        };
        if reasons.is_empty() && candidate_results.len() != baseline.len() {
            reasons.push("candidate mutation results did not cover every seed".to_string());
        }
        // Adaptive span skipping can expose a new surviving sibling and skip mutants that the
        // baseline killed, so aggregate kill counts are not monotonic. Compare stable survivor
        // identities instead: candidate files cannot alter the mutated sources. A useful property
        // must eliminate an identity on every seed without reviving it on another seed; it need not
        // add a new kill on seeds where the baseline already killed that mutant.
        let newly_resolved = if reasons.is_empty() {
            resolved_survivor_identities(current_results, &candidate_results)
        } else {
            BTreeSet::new()
        };
        if reasons.is_empty() && newly_resolved.is_empty() {
            reasons.push(
                "candidate did not reproducibly resolve a current mutation survivor".to_string(),
            );
        }
        if reasons.is_empty()
            && candidate_results.iter().any(|result| {
                !previously_resolved.is_disjoint(&survivor_identities(&result.output))
            })
        {
            reasons.push("candidate revived a survivor resolved by an earlier round".to_string());
        }
        Ok((
            Evaluation {
                candidate_digest: digest,
                generator: candidate.generator.clone(),
                accepted: reasons.is_empty(),
                reasons,
                baseline: mutation_result_summaries(baseline),
                candidate: mutation_result_summaries(&candidate_results),
                resolved_survivors: resolved_survivor_identities(baseline, &candidate_results)
                    .len(),
                newly_resolved_survivors: newly_resolved.len(),
                possible_bugs,
                after_possible_bug: false,
            },
            candidate_results,
            newly_resolved,
        ))
    }

    /// Runs every test in the candidate files on every seed. Returns the rejection reasons, the
    /// tests that fail on every seed (possible bugs), and the tests that ran.
    fn check_candidate_tests(
        &self,
        forge: &Path,
        workspace: &Path,
        config: &Config,
        candidate: &Candidate,
    ) -> Result<(Vec<String>, Vec<String>, Vec<RerunFailure>)> {
        let paths = candidate
            .files
            .iter()
            .map(|file| file.path.display().to_string())
            .collect::<Vec<_>>()
            .join(",");
        let mut reasons = Vec::new();
        let mut failures = BTreeMap::<(String, String), Vec<String>>::new();
        for seed in &self.seed {
            let mut command = forge_command(forge, config, workspace, seed);
            // Configured test filters must not hide generated tests.
            command.args(["test", "--json", "--match-path", &format!("{{{paths}}}")]);
            command.args(SELECT_ALL_TESTS);
            add_dependency_args(&mut command, config, workspace);
            let output = command.output().wrap_err("failed to run candidate tests")?;
            if !output.status.success() && output.stdout.is_empty() {
                reasons.push(format!("candidate tests failed on seed {seed}: {}", stderr(&output)));
                continue;
            }
            let results = candidate_test_results(&output)?;
            if results.is_empty() {
                reasons.push(format!("candidate files contain no runnable tests on seed {seed}"));
                continue;
            }
            for (test, status, reason) in results {
                let name = test_display_name(&test);
                let seed_failures = failures.entry((test.contract, test.test)).or_default();
                match status {
                    TestStatus::Success => {}
                    TestStatus::Failure => {
                        let mut reason = reason.unwrap_or_else(|| stderr(&output));
                        if reason.is_empty() {
                            reason = "the test failed without a reason".to_string();
                        }
                        reasons.push(format!("{name} failed on seed {seed}: {reason}"));
                        seed_failures.push(format!("seed {seed}: {reason}"));
                    }
                    TestStatus::Skipped => reasons.push(format!(
                        "{name} was skipped on seed {seed}: {}",
                        reason.unwrap_or_else(|| "no reason reported".to_string())
                    )),
                }
            }
        }
        let possible_bugs = failures
            .iter()
            .filter(|(_, seed_failures)| seed_failures.len() == self.seed.len())
            .map(|((contract, test), seed_failures)| {
                let test = RerunFailure { contract: contract.clone(), test: test.clone() };
                format!("{} ({})", test_display_name(&test), seed_failures.join("; "))
            })
            .collect();
        let tests =
            failures.into_keys().map(|(contract, test)| RerunFailure { contract, test }).collect();
        Ok((reasons, possible_bugs, tests))
    }

    fn check(
        &self,
        config: &Config,
        forge: &Path,
        generated_tests: &Path,
        path: &Path,
    ) -> Result<()> {
        let candidate = read_candidate(path, generated_tests)?;
        let workspace = candidate_workspace(config, &candidate, &[])?;
        let (reasons, possible_bugs, _) =
            self.check_candidate_tests(forge, workspace.path(), config, &candidate)?;
        let passed = reasons.is_empty();
        sh_println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "passed": passed,
                "reasons": reasons,
                "possible_bugs": possible_bugs,
            }))?
        )?;
        ensure!(passed, "candidate check failed");
        Ok(())
    }

    /// Runs mutation testing for every seed concurrently. Each seed builds into its own
    /// directories. With `selection`, exactly those tests run.
    fn run_mutations(
        &self,
        forge: &Path,
        config: &Config,
        workspace: &Path,
        filter_args: &[String],
        selection: Option<&[RerunFailure]>,
    ) -> Result<Vec<SeedMutation>> {
        std::thread::scope(|scope| {
            let runs = self
                .seed
                .iter()
                .map(|seed| {
                    scope.spawn(move || {
                        self.run_mutation(forge, config, workspace, filter_args, selection, seed)
                    })
                })
                .collect::<Vec<_>>();
            runs.into_iter().map(|run| run.join().expect("mutation thread panicked")).collect()
        })
    }

    fn run_mutation(
        &self,
        forge: &Path,
        config: &Config,
        workspace: &Path,
        filter_args: &[String],
        selection: Option<&[RerunFailure]>,
        seed: &U256,
    ) -> Result<SeedMutation> {
        let seed_dir = seed_dir(config, workspace, seed);
        let mut command = forge_command(forge, config, workspace, seed);
        command
            .env("FOUNDRY_OUT", seed_dir.join("out"))
            .env("FOUNDRY_CACHE_PATH", seed_dir.join("cache"));
        if let Some(selection) = selection {
            // `--rerun` selects exact contract and test pairs; the broad patterns keep configured
            // filters from removing any of them.
            fs::create_dir_all(&seed_dir)?;
            fs::write(
                seed_dir.join("test-failures"),
                serde_json::to_vec(&serde_json::json!({ "version": 1, "failures": selection }))?,
            )?;
        }
        command.args(["test", "--json", "--mutate"]);
        command.args(&self.mutate);
        if selection.is_some() {
            command.args(["--rerun", "--match-path", "**"]);
            command.args(SELECT_ALL_TESTS);
        }
        if let Some(timeout) = self.mutation_timeout {
            command.args(["--mutation-timeout", &timeout.to_string()]);
        }
        // Seeds run concurrently, so each seed gets an equal share of the cores. Mutation results
        // do not depend on the worker count.
        let jobs = std::thread::available_parallelism()
            .map_or(1, |cores| (cores.get() / self.seed.len()).max(1));
        command.args(["--mutation-jobs", &jobs.to_string()]);
        command.args(filter_args);
        // Candidate workspaces materialize the project's libraries; point remappings at them.
        if workspace != config.root {
            add_dependency_args(&mut command, config, workspace);
        }
        let output = command.output().wrap_err("failed to run mutation testing")?;
        ensure!(
            output.status.success(),
            "mutation testing failed for seed {seed}: {}",
            stderr(&output)
        );
        Ok(SeedMutation {
            seed: *seed,
            output: serde_json::from_slice(&output.stdout)
                .wrap_err_with(|| format!("invalid mutation JSON for seed {seed}"))?,
        })
    }
}

/// Copies the project into a temporary workspace and adds the candidate files.
fn candidate_workspace(
    config: &Config,
    candidate: &Candidate,
    mutate: &[PathBuf],
) -> Result<TempDir> {
    let candidate_workspace = tempfile::Builder::new().prefix("forge-properties-").tempdir()?;
    workspace::copy_project(config, candidate_workspace.path())?;
    // Mutation testing copies this workspace again. Materialize project-local library and
    // dependency symlinks (`copy_project` links `node_modules` and `dependencies` even when
    // they are not in `libs`) so that nested copy cannot escape back to the source project.
    let dependency_dirs = ["node_modules", "dependencies"].map(PathBuf::from);
    for lib in config.libs.iter().chain(&dependency_dirs) {
        let source = if lib.is_absolute() { lib.clone() } else { config.root.join(lib) };
        let Ok(relative) = source.strip_prefix(&config.root) else { continue };
        if !workspace::is_safe_relative_path(relative) || !source.is_dir() {
            continue;
        }

        let target = candidate_workspace.path().join(relative);
        if target.exists() {
            let metadata = fs::symlink_metadata(&target)?;
            if !metadata.file_type().is_symlink() {
                continue;
            }
            #[cfg(unix)]
            fs::remove_file(&target)?;
            #[cfg(windows)]
            fs::remove_dir(&target)?;
        }
        fs::create_dir_all(&target)?;
        for entry in fs::read_dir(&source)? {
            let entry = entry?;
            let destination = target.join(entry.file_name());
            // Mutation testing only accepts targets inside its workspace, so copy dependency
            // directories that contain a mutation target instead of linking them.
            let contains_target = || {
                let relative = relative.join(entry.file_name());
                mutate.iter().any(|target| target.starts_with(&relative))
            };
            if entry.path().is_dir() {
                if contains_target() || workspace::symlink_dir(&entry.path(), &destination).is_err()
                {
                    workspace::copy_dir_recursive(&entry.path(), &destination)?;
                }
            } else {
                fs::copy(entry.path(), destination)?;
            }
        }
    }
    for file in &candidate.files {
        let path = candidate_workspace.path().join(&file.path);
        fs::create_dir_all(path.parent().expect("candidate path has a parent"))?;
        fs::write(path, &file.content)?;
    }
    Ok(candidate_workspace)
}

/// Returns every test result in Forge's JSON output, with one result per invariant predicate of a
/// merged invariant campaign.
fn candidate_test_results(
    output: &Output,
) -> Result<Vec<(RerunFailure, TestStatus, Option<String>)>> {
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
            if result.invariant_predicate_results.is_empty() {
                let reason = result.reason.clone().or_else(|| {
                    let reasons = result
                        .invariant_failures
                        .iter()
                        .map(|failure| failure_reason(failure.reason(), failure.counterexample()))
                        .collect::<Vec<_>>();
                    (!reasons.is_empty()).then(|| reasons.join("; "))
                });
                let reason = match (reason, result.counterexample.as_ref()) {
                    (Some(reason), counterexample) => Some(failure_reason(&reason, counterexample)),
                    (None, Some(counterexample)) => Some(failure_reason("", Some(counterexample))),
                    (None, None) => None,
                };
                results.push((key(test), result.status, reason));
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
            // Handler assertion failures belong to the campaign, not to one predicate.
            for failure in result.invariant_failures.iter().filter(|f| f.predicate_name().is_none())
            {
                let reason = format!(
                    "handler {}: {}",
                    failure.name(),
                    failure_reason(failure.reason(), failure.counterexample())
                );
                results.push((key(test), TestStatus::Failure, Some(reason)));
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

/// Lists the tests that the filter arguments select in a project.
fn list_tests(forge: &Path, root: &Path, filter_args: &[String]) -> Result<Vec<RerunFailure>> {
    let output = Command::new(forge)
        .current_dir(root)
        .args(["test", "--list", "--json"])
        .args(filter_args)
        .output()
        .wrap_err("failed to list tests")?;
    ensure!(output.status.success(), "failed to list tests: {}", stderr(&output));
    let files =
        serde_json::from_slice::<BTreeMap<String, BTreeMap<String, Vec<String>>>>(&output.stdout)
            .wrap_err("invalid test list JSON")?;
    Ok(files
        .into_iter()
        .flat_map(|(path, contracts)| {
            contracts.into_iter().flat_map(move |(contract, tests)| {
                let contract = format!("{path}:{contract}");
                tests.into_iter().map(move |test| RerunFailure { contract: contract.clone(), test })
            })
        })
        .collect())
}

/// Returns `Contract::test` for a test identity.
fn test_display_name(test: &RerunFailure) -> String {
    let contract = test.contract.rsplit_once(':').map_or(test.contract.as_str(), |(_, name)| name);
    let name = test.test.split_once('(').map_or(test.test.as_str(), |(name, _)| name);
    format!("{contract}::{name}")
}

fn project_context(config: &Config, mutate: &[PathBuf]) -> ProjectContext {
    let mut context = ProjectContext {
        remappings: config
            .remappings
            .iter()
            .cloned()
            .map(|remapping| remapping.to_relative_remapping())
            .map(|remapping| remapping.to_string())
            .collect(),
        isolate: config.isolate,
        evm_version: config.evm_version.to_string(),
        target_sources: Vec::new(),
        test_sources: Vec::new(),
    };
    let targets = mutate
        .iter()
        .filter_map(|path| Some((path, fs::read_to_string(config.root.join(path)).ok()?)))
        .collect::<Vec<_>>();
    context.target_sources =
        prompt_sources(targets.iter().map(|(path, source)| (path.as_path(), source.as_str())));
    let Ok(graph) =
        Graph::<MultiCompilerParser>::resolve(&config.project_paths::<MultiCompilerLanguage>())
    else {
        return context;
    };
    let mutation_targets = graph
        .files()
        .keys()
        .filter(|path| {
            let relative = workspace::relative_to_root(&config.root, path);
            mutate.iter().any(|target| target == &relative)
        })
        .cloned()
        .collect::<BTreeSet<_>>();
    let generated_tests = config.test.join("generated");
    let mut relevant_tests = graph
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| {
            node.path().starts_with(&config.test)
                && !node.path().starts_with(&generated_tests)
                && node.path().extension().is_some_and(|extension| extension == "sol")
        })
        .filter(|(_, node)| {
            graph.imports(node.path()).iter().any(|import| mutation_targets.contains(*import))
        })
        .collect::<Vec<_>>();
    let directly_imports_target = |index| {
        graph
            .imported_nodes(index)
            .iter()
            .any(|import| mutation_targets.contains(graph.node(*import).path()))
    };
    // A test file named after a target, such as `Math.t.sol` for `Math.sol`, usually holds its
    // primary suite even when it reaches the target through a wrapper.
    let target_stems = mutate.iter().filter_map(|path| path.file_stem()).collect::<Vec<_>>();
    let names_target = |path: &Path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.split('.').next())
            .is_some_and(|stem| target_stems.iter().any(|target| target.to_str() == Some(stem)))
    };
    relevant_tests.sort_unstable_by(|(a_index, a), (b_index, b)| {
        names_target(b.path())
            .cmp(&names_target(a.path()))
            .then_with(|| directly_imports_target(*b_index).cmp(&directly_imports_target(*a_index)))
            .then_with(|| a.content().len().cmp(&b.content().len()))
            .then_with(|| a.path().cmp(b.path()))
    });
    let mut total_bytes = 0;
    for (_, node) in relevant_tests {
        if context.test_sources.len() == MAX_PROJECT_CONTEXT_FILES {
            break;
        }
        let path = node.path();
        let Ok(canonical) = path.canonicalize() else { continue };
        let Ok(relative) = canonical.strip_prefix(&config.root) else { continue };
        let source = node.content();
        let available = MAX_PROMPT_SOURCE_BYTES - total_bytes;
        let end = source.floor_char_boundary(available);
        if end == 0 {
            continue;
        }
        total_bytes += end;
        context.test_sources.push(PromptSource {
            path: relative.to_path_buf(),
            content: source[..end].to_string(),
            truncated: end < source.len(),
        });
    }
    context
}

/// Bounds sources to one shared byte budget so a single section cannot dominate the prompt.
fn prompt_sources<'a>(sources: impl IntoIterator<Item = (&'a Path, &'a str)>) -> Vec<PromptSource> {
    let mut prompt_sources = Vec::new();
    let mut total_bytes = 0;
    for (path, source) in sources {
        let available = MAX_PROMPT_SOURCE_BYTES - total_bytes;
        if available == 0 {
            break;
        }
        let end = source.floor_char_boundary(available);
        total_bytes += end;
        prompt_sources.push(PromptSource {
            path: path.to_path_buf(),
            content: source[..end].to_string(),
            truncated: end < source.len(),
        });
    }
    prompt_sources
}

fn mutation_gaps(root: &Path, results: &[SeedMutation]) -> Vec<MutationGap> {
    let mut gaps = BTreeMap::<(PathBuf, usize, usize, String, String), BTreeSet<U256>>::new();
    for result in results {
        for (path, mutants) in &result.output.survived_mutants {
            for mutant in mutants {
                gaps.entry((
                    PathBuf::from(path),
                    mutant.line,
                    mutant.column,
                    mutant.original.clone(),
                    mutant.mutant.clone(),
                ))
                .or_default()
                .insert(result.seed);
            }
        }
    }
    let mut source_files = BTreeMap::<PathBuf, Option<Vec<String>>>::new();
    let mut gaps = gaps
        .into_iter()
        .map(|((path, line, column, original, mutant), seeds)| MutationGap {
            likely_equivalent: likely_equivalent(&original, &mutant),
            source_context: None,
            path,
            line,
            column,
            original,
            mutant,
            survives_all_seeds: seeds.len() == results.len(),
            seeds: seeds.into_iter().collect(),
        })
        .collect::<Vec<_>>();
    for gap in &mut gaps {
        let lines = source_files.entry(gap.path.clone()).or_insert_with(|| {
            fs::read_to_string(root.join(&gap.path))
                .ok()
                .map(|source| source.lines().map(str::to_string).collect())
        });
        let Some(lines) = lines else { continue };
        let start = gap.line.saturating_sub(4);
        let end = gap.line.saturating_add(3).min(lines.len());
        gap.source_context = Some(
            (start..end)
                .map(|index| format!("{:>4}: {}", index + 1, lines[index]))
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }
    gaps.sort_by(|a, b| {
        b.survives_all_seeds
            .cmp(&a.survives_all_seeds)
            .then_with(|| a.likely_equivalent.is_some().cmp(&b.likely_equivalent.is_some()))
            .then_with(|| a.path.cmp(&b.path))
            .then_with(|| a.line.cmp(&b.line))
            .then_with(|| a.column.cmp(&b.column))
            .then_with(|| a.original.cmp(&b.original))
            .then_with(|| a.mutant.cmp(&b.mutant))
    });
    gaps
}

/// Returns why a surviving mutant is likely equivalent to the original, for common patterns that
/// no test can distinguish.
fn likely_equivalent(original: &str, mutant: &str) -> Option<&'static str> {
    const SIGNED_FORMS: [(&str, &str); 5] =
        [("lt(", "slt("), ("gt(", "sgt("), ("shr(", "sar("), ("div(", "sdiv("), ("mod(", "smod(")];
    const UNSIGNED_ZERO_FORMS: [(&str, &str); 2] = [(">", "!="), ("==", "<=")];
    if SIGNED_FORMS.iter().any(|(unsigned, signed)| {
        original
            .strip_prefix(unsigned)
            .is_some_and(|args| mutant.strip_prefix(signed) == Some(args))
    }) {
        return Some("signed and unsigned forms agree while the operands are below 2^255");
    }
    if UNSIGNED_ZERO_FORMS.iter().any(|(from, to)| {
        ["0", "uint256(0)"].iter().any(|zero| {
            let operand = original.strip_suffix(&format!(" {from} {zero}"));
            operand.is_some() && operand == mutant.strip_suffix(&format!(" {to} {zero}"))
        })
    }) {
        return Some("the comparisons agree when the operand is unsigned");
    }
    None
}

fn mutation_result_summaries(results: &[SeedMutation]) -> Vec<MutationResultSummary> {
    results
        .iter()
        .map(|result| MutationResultSummary {
            seed: result.seed,
            summary: result.output.summary.clone(),
        })
        .collect()
}

/// Returns the baseline survivors that the candidate kills on every seed.
///
/// The mutant population does not change, so a mutant is killed when it is not survived, timed
/// out, invalid, or skipped. Being absent from the survivors alone is not enough.
fn resolved_survivor_identities(
    baseline: &[SeedMutation],
    candidate: &[SeedMutation],
) -> BTreeSet<MutationIdentity> {
    if baseline.iter().map(|result| result.seed).ne(candidate.iter().map(|result| result.seed)) {
        return BTreeSet::new();
    }
    let not_killed =
        candidate.iter().map(|result| not_killed_identities(&result.output)).collect::<Vec<_>>();
    baseline
        .iter()
        .flat_map(|result| survivor_identities(&result.output))
        .filter(|identity| not_killed.iter().all(|not_killed| !not_killed.contains(identity)))
        .collect()
}

/// Returns the mutants that the run did not kill: survived, timed out, invalid, or skipped.
fn not_killed_identities(output: &MutationJsonOutput) -> BTreeSet<MutationIdentity> {
    [
        &output.survived_mutants,
        &output.timed_out_mutants,
        &output.invalid_mutants,
        &output.skipped_mutants,
    ]
    .into_iter()
    .flat_map(mutant_identities)
    .collect()
}

fn survivor_identities(output: &MutationJsonOutput) -> BTreeSet<MutationIdentity> {
    mutant_identities(&output.survived_mutants)
}

fn mutant_identities(
    mutants: &BTreeMap<String, Vec<crate::mutation::SurvivedMutantJson>>,
) -> BTreeSet<MutationIdentity> {
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

fn accumulate_candidate(
    current: Option<&Candidate>,
    proposal: &Candidate,
    generated_tests: &Path,
) -> Result<Candidate> {
    let Some(current) = current else { return Ok(proposal.clone()) };
    let existing_paths = current.files.iter().map(|file| &file.path).collect::<HashSet<_>>();
    for file in &proposal.files {
        ensure!(
            !existing_paths.contains(&file.path),
            "candidate path {} was retained by an earlier round",
            file.path.display()
        );
    }

    let mut candidate = Candidate {
        schema: CANDIDATE_SCHEMA.to_string(),
        rationale: format!("{}\n\n{}", current.rationale, proposal.rationale),
        generator: if current.generator == proposal.generator {
            proposal.generator.clone()
        } else {
            None
        },
        files: current.files.clone(),
    };
    candidate.files.extend(proposal.files.iter().cloned());
    validate_candidate(&candidate, generated_tests)?;
    Ok(candidate)
}

fn read_candidate(path: &Path, generated_tests: &Path) -> Result<Candidate> {
    let candidate: Candidate = serde_json::from_slice(
        &fs::read(path).wrap_err_with(|| format!("failed to read {}", path.display()))?,
    )
    .wrap_err("invalid candidate JSON")?;
    validate_candidate(&candidate, generated_tests)?;
    Ok(candidate)
}

fn validate_candidate(candidate: &Candidate, generated_tests: &Path) -> Result<()> {
    ensure!(candidate.schema == CANDIDATE_SCHEMA, "unsupported candidate schema");
    ensure!(!candidate.rationale.trim().is_empty(), "candidate rationale is empty");
    if let Some(generator) = &candidate.generator {
        ensure!(!generator.agent.trim().is_empty(), "candidate generator agent is empty");
        ensure!(!generator.model.trim().is_empty(), "candidate generator model is empty");
    }
    ensure!(!candidate.files.is_empty(), "candidate contains no files");
    ensure!(candidate.files.len() <= MAX_CANDIDATE_FILES, "candidate contains too many files");
    ensure!(
        candidate.files.iter().map(|file| file.content.len()).sum::<usize>() <= MAX_CANDIDATE_BYTES,
        "candidate source is too large"
    );
    let mut paths = HashSet::new();
    for file in &candidate.files {
        validate_candidate_path(&file.path, generated_tests)?;
        ensure!(paths.insert(&file.path), "candidate contains duplicate file paths");
        // The campaign seeds must decide every run, so a generated test cannot pin its own seed.
        let sets_seed = file.content.lines().any(|line| {
            line.split_once("forge-config:")
                .and_then(|(_, setting)| setting.split_once('='))
                .is_some_and(|(key, _)| key.trim().ends_with(".seed"))
        });
        ensure!(!sets_seed, "candidate {} sets a seed in inline config", file.path.display());
    }
    Ok(())
}

fn validate_candidate_path(path: &Path, generated_tests: &Path) -> Result<()> {
    ensure!(
        path.extension().is_some_and(|extension| extension == "sol"),
        "candidate files must be Solidity sources"
    );
    ensure!(
        path.starts_with(generated_tests),
        "candidate files must be under {}/",
        generated_tests.display()
    );
    ensure!(
        workspace::is_safe_relative_path(path),
        "candidate path contains a non-normal component"
    );
    Ok(())
}

/// Runs Forge with one seed. Each seed persists fuzz and invariant failures in its own
/// directory, so a counterexample found with one seed is not replayed with another.
fn forge_command(forge: &Path, config: &Config, workspace: &Path, seed: &U256) -> Command {
    let seed_dir = seed_dir(config, workspace, seed);
    let mut command = Command::new(forge);
    command
        .current_dir(workspace)
        .env("FOUNDRY_FUZZ_SEED", format!("{seed:#x}"))
        .env("FOUNDRY_FUZZ_FAILURE_PERSIST_DIR", seed_dir.join("fuzz"))
        .env("FOUNDRY_INVARIANT_FAILURE_PERSIST_DIR", seed_dir.join("invariant"))
        // Internal runs must not replace the project's own `--rerun` state.
        .env("FOUNDRY_TEST_FAILURES_FILE", seed_dir.join("test-failures"));
    // Read the project's own config file, so `extends` resolves next to it, not in the copy.
    let config_file = config.root.join(Config::FILE_NAME);
    if workspace != config.root
        && std::env::var_os("FOUNDRY_CONFIG").is_none()
        && config_file.is_file()
    {
        command.env("FOUNDRY_CONFIG", config_file);
    }
    command
}

/// Returns the directory for one seed's builds and persisted state in a workspace.
fn seed_dir(config: &Config, workspace: &Path, seed: &U256) -> PathBuf {
    workspace
        .join(workspace::relative_to_root(&config.root, &config.cache_path))
        .join("properties/seeds")
        .join(format!("{seed:#x}"))
}

fn add_dependency_args(command: &mut Command, config: &Config, workspace: &Path) {
    for lib in &config.libs {
        let lib = if lib.is_absolute() { lib.clone() } else { config.root.join(lib) };
        let lib =
            lib.strip_prefix(&config.root).map_or(lib.clone(), |relative| workspace.join(relative));
        command.arg("--lib-paths");
        command.arg(lib);
    }
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
