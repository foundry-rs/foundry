use crate::{mutation::MutationJsonOutput, result::TestStatus, workspace};
use alloy_primitives::{U256, keccak256};
use clap::Parser;
use eyre::{Context, Result, ensure, eyre};
use foundry_common::sh_println;
use foundry_config::Config;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};
const CANDIDATE_SCHEMA: &str = "foundry/fuzz-improve-candidate-v1";
const PROMPT_SCHEMA: &str = "foundry/fuzz-improve-prompt-v1";
const MAX_CANDIDATE_FILES: usize = 8;
const MAX_CANDIDATE_BYTES: usize = 256 * 1024;

/// Generate fuzz properties and retain only reproducible mutation-coverage improvements.
#[derive(Clone, Debug, Parser)]
pub struct FuzzImproveArgs {
    /// Root of the Foundry project.
    #[arg(long, default_value = ".", value_name = "PATH")]
    root: PathBuf,

    /// Production source files passed to mutation testing.
    #[arg(long, required = true, num_args = 1.., value_name = "PATH")]
    mutate: Vec<PathBuf>,

    /// Markdown instructions supplied to the property generator.
    #[arg(long, value_name = "PATH")]
    brief: PathBuf,

    /// Executable that receives PROMPT_JSON and OUTPUT_JSON as its final arguments.
    #[arg(long, value_name = "PATH")]
    generator: PathBuf,

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
    tests: Vec<CandidateTest>,
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

#[derive(Clone, Debug, Deserialize, Serialize)]
struct CandidateTest {
    path: PathBuf,
    contract: String,
    name: String,
}

#[derive(Debug, Serialize)]
struct GeneratorPrompt<'a> {
    schema: &'static str,
    round: usize,
    project: &'a Path,
    brief: &'a str,
    guidance: &'static str,
    mutate: &'a [PathBuf],
    seeds: &'a [U256],
    baseline: &'a [PromptMutation<'a>],
    current_results: &'a [PromptMutation<'a>],
    mutation_gaps: &'a [MutationGap],
    #[serde(skip_serializing_if = "Option::is_none")]
    current_candidate: Option<&'a Candidate>,
    previous_feedback: &'a [ProposalFeedback],
    output_contract: OutputContract,
}

#[derive(Debug, Serialize)]
struct OutputContract {
    schema: &'static str,
    allowed_path_prefix: PathBuf,
    maximum_files: usize,
    maximum_total_bytes: usize,
    example: Candidate,
    note: &'static str,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    source_context: Option<String>,
}

#[derive(Debug, Serialize)]
struct ProposalFeedback {
    candidate_digest: String,
    accepted: bool,
    rationale: Option<String>,
    tests: Vec<CandidateTest>,
    reasons: Vec<String>,
    candidate_results: Vec<MutationResultSummary>,
    resolved_survivors: usize,
    newly_resolved_survivors: usize,
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
}

type MutationIdentity = (String, usize, usize, String, String);

impl FuzzImproveArgs {
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
        let brief = fs::read_to_string(&self.brief).wrap_err("failed to read campaign brief")?;
        let generator = self.generator.canonicalize().wrap_err("failed to resolve generator")?;
        let forge = std::env::current_exe().wrap_err("failed to resolve Forge executable")?;
        let contract_filter = self
            .match_contract
            .as_deref()
            .or_else(|| config.contract_pattern.as_ref().map(|pattern| pattern.as_str()));
        let baseline = self.run_mutations(&forge, &config.root, None, contract_filter)?;
        let prompt_baseline = baseline
            .iter()
            .map(|result| PromptMutation { seed: &result.seed, summary: &result.output.summary })
            .collect::<Vec<_>>();
        let cache_root = config.cache_path.join("fuzz-improve");
        fs::create_dir_all(&cache_root)?;

        let mut evaluations = Vec::new();
        let mut feedback = Vec::new();
        let mut current_candidate = None::<Candidate>;
        let mut current_results = baseline.clone();
        let mut resolved_survivors = BTreeSet::new();
        let mut accepted = None;
        for round in 1..=self.rounds {
            let round_dir = tempfile::Builder::new().prefix("forge-fuzz-improve-").tempdir()?;
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
                guidance: "Prioritize mutations that survive all seeds and inspect their numbered source context. Propose a property only when a concrete input or sequence can distinguish the original from the mutant; surviving mutants may be semantically equivalent. The current candidate is retained automatically, so return only new files with distinct paths.",
                mutate: &self.mutate,
                seeds: &self.seed,
                baseline: &prompt_baseline,
                current_results: &prompt_current_results,
                mutation_gaps: &mutation_gaps,
                current_candidate: current_candidate.as_ref(),
                previous_feedback: &feedback,
                output_contract: OutputContract {
                    schema: CANDIDATE_SCHEMA,
                    allowed_path_prefix: generated_tests.clone(),
                    maximum_files: MAX_CANDIDATE_FILES - retained_files,
                    maximum_total_bytes: MAX_CANDIDATE_BYTES - retained_bytes,
                    example: Candidate {
                        schema: CANDIDATE_SCHEMA.to_string(),
                        rationale: "Explain the concrete input or sequence that distinguishes the original from the mutant.".to_string(),
                        generator: Some(GeneratorMetadata {
                            agent: "generator name".to_string(),
                            model: "model name".to_string(),
                        }),
                        files: vec![CandidateFile {
                            path: example_path.clone(),
                            content: format!(
                                "pragma solidity ^0.8.0;\ncontract {example_contract} {{\n    function testProperty() external {{}}\n}}\n"
                            ),
                        }],
                        tests: vec![CandidateTest {
                            path: example_path,
                            contract: example_contract,
                            name: "testProperty".to_string(),
                        }],
                    },
                    note: "Return one JSON object with exactly this shape. Keep schema unchanged; replace the structural example's rationale, files, tests, and generator metadata with a useful property for this project. Generated assertions are candidates for human review, not proofs. Each file path must be new relative to current_candidate. The generator object is optional.",
                },
            };
            fs::write(&prompt_path, serde_json::to_vec_pretty(&prompt)?)?;

            let output = Command::new(&generator)
                .args(&self.generator_arg)
                .arg(&prompt_path)
                .arg(&candidate_path)
                .current_dir(&config.root)
                .output()
                .wrap_err("failed to invoke generator")?;
            let proposal = if output.status.success() {
                read_candidate(&candidate_path, &generated_tests)
            } else {
                Err(eyre!("generator failed: {}", stderr(&output)))
            };
            let evaluation = match proposal {
                Ok(proposal) => match accumulate_candidate(
                    current_candidate.as_ref(),
                    &proposal,
                    &generated_tests,
                ) {
                    Ok(candidate) => {
                        let digest = keccak256(serde_json::to_vec(&candidate)?).to_string();
                        let evaluated = self.evaluate(
                            &config,
                            &forge,
                            &baseline,
                            &current_results,
                            &resolved_survivors,
                            &candidate,
                        );
                        let (evaluation, candidate_results, newly_resolved) = evaluated
                            .unwrap_or_else(|error| {
                                (
                                    Evaluation {
                                        candidate_digest: digest,
                                        generator: candidate.generator.clone(),
                                        accepted: false,
                                        reasons: vec![error.to_string()],
                                        baseline: mutation_result_summaries(&baseline),
                                        candidate: vec![],
                                        resolved_survivors: 0,
                                        newly_resolved_survivors: 0,
                                    },
                                    vec![],
                                    BTreeSet::new(),
                                )
                            });
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
                            accepted = Some(candidate_cache);
                        }
                        feedback.push(proposal_feedback(Some(&proposal), &evaluation));
                        evaluation
                    }
                    Err(error) => {
                        let evaluation = Evaluation {
                            candidate_digest: format!("round-{round}"),
                            generator: proposal.generator.clone(),
                            accepted: false,
                            reasons: vec![error.to_string()],
                            baseline: mutation_result_summaries(&baseline),
                            candidate: vec![],
                            resolved_survivors: 0,
                            newly_resolved_survivors: 0,
                        };
                        feedback.push(proposal_feedback(Some(&proposal), &evaluation));
                        evaluation
                    }
                },
                Err(error) => {
                    let evaluation = Evaluation {
                        candidate_digest: format!("round-{round}"),
                        generator: None,
                        accepted: false,
                        reasons: vec![error.to_string()],
                        baseline: mutation_result_summaries(&baseline),
                        candidate: vec![],
                        resolved_survivors: 0,
                        newly_resolved_survivors: 0,
                    };
                    feedback.push(proposal_feedback(None, &evaluation));
                    evaluation
                }
            };
            evaluations.push(evaluation);
        }

        fs::write(cache_root.join("rounds.json"), serde_json::to_vec_pretty(&evaluations)?)?;
        if let Some(path) = accepted {
            let path = path.strip_prefix(&config.root).unwrap_or(&path);
            sh_println!(
                "accepted candidate: {} (reproducibly resolved {} survivor(s) across rounds)",
                path.display(),
                resolved_survivors.len(),
            )?;
        } else {
            sh_println!("no candidate reproducibly resolved a mutation survivor")?;
        }
        Ok(())
    }

    fn evaluate(
        &self,
        config: &Config,
        forge: &Path,
        baseline: &[SeedMutation],
        current_results: &[SeedMutation],
        previously_resolved: &BTreeSet<MutationIdentity>,
        candidate: &Candidate,
    ) -> Result<(Evaluation, Vec<SeedMutation>, BTreeSet<MutationIdentity>)> {
        let candidate_workspace =
            tempfile::Builder::new().prefix("forge-fuzz-improve-").tempdir()?;
        workspace::copy_project(config, candidate_workspace.path())?;
        // Mutation testing copies this workspace again. Materialize project-local library
        // symlinks so that nested copy cannot escape through links back to the source project.
        for lib in &config.libs {
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
                if entry.path().is_dir() {
                    if workspace::symlink_dir(&entry.path(), &destination).is_err() {
                        workspace::copy_dir_recursive(&entry.path(), &destination)?;
                    }
                } else {
                    fs::copy(entry.path(), destination)?;
                }
            }
        }
        for file in &candidate.files {
            let path = candidate_workspace.path().join(&file.path);
            ensure!(!path.exists(), "candidate would overwrite {}", file.path.display());
            fs::create_dir_all(path.parent().expect("candidate path has a parent"))?;
            fs::write(path, &file.content)?;
        }
        let mut reasons = Vec::new();
        for seed in &self.seed {
            for test in &candidate.tests {
                let output =
                    self.run_candidate_test(forge, candidate_workspace.path(), config, seed, test)?;
                if !output.status.success() && output.stdout.is_empty() {
                    reasons.push(format!(
                        "{}::{} failed on seed {seed}: {}",
                        test.contract,
                        test.name,
                        stderr(&output)
                    ));
                    continue;
                }
                match candidate_test_result(&output, test) {
                    Ok((TestStatus::Success, _)) if output.status.success() => {}
                    Ok((TestStatus::Success, _)) => reasons.push(format!(
                        "{}::{} passed but Forge exited unsuccessfully on seed {seed}: {}",
                        test.contract,
                        test.name,
                        stderr(&output)
                    )),
                    Ok((TestStatus::Failure, reason)) => reasons.push(format!(
                        "{}::{} failed on seed {seed}: {}",
                        test.contract,
                        test.name,
                        reason.unwrap_or_else(|| stderr(&output))
                    )),
                    Ok((TestStatus::Skipped, reason)) => reasons.push(format!(
                        "{}::{} was skipped on seed {seed}: {}",
                        test.contract,
                        test.name,
                        reason.unwrap_or_else(|| "no reason reported".to_string())
                    )),
                    Err(error) => reasons.push(format!(
                        "{}::{} could not be verified on seed {seed}: {error}",
                        test.contract, test.name
                    )),
                }
            }
        }

        let candidate_results = if reasons.is_empty() {
            let base_filter = self
                .match_contract
                .as_deref()
                .or_else(|| config.contract_pattern.as_ref().map(|pattern| pattern.as_str()));
            let contract_filter = candidate_contract_filter(base_filter, candidate);
            let mut results = Vec::with_capacity(self.seed.len());
            for before in current_results {
                let after = self.run_mutation(
                    forge,
                    candidate_workspace.path(),
                    Some(config),
                    contract_filter.as_deref(),
                    &before.seed,
                )?;
                let changed_population = before.output.summary.total != after.output.summary.total
                    || before.output.summary.invalid != after.output.summary.invalid;
                let introduced_timeout =
                    after.output.timed_out_mutants.iter().any(|(path, mutants)| {
                        before.output.timed_out_mutants.get(path).is_none_or(|baseline| {
                            mutants.iter().any(|mutant| !baseline.contains(mutant))
                        })
                    });
                results.push(after);

                if changed_population {
                    reasons.push(format!(
                        "candidate changed the mutant population on seed {}",
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
                candidate_digest: keccak256(serde_json::to_vec(candidate)?).to_string(),
                generator: candidate.generator.clone(),
                accepted: reasons.is_empty(),
                reasons,
                baseline: mutation_result_summaries(baseline),
                candidate: mutation_result_summaries(&candidate_results),
                resolved_survivors: resolved_survivor_identities(baseline, &candidate_results)
                    .len(),
                newly_resolved_survivors: newly_resolved.len(),
            },
            candidate_results,
            newly_resolved,
        ))
    }

    fn run_candidate_test(
        &self,
        forge: &Path,
        workspace: &Path,
        dependency_config: &Config,
        seed: &U256,
        test: &CandidateTest,
    ) -> Result<Output> {
        let mut command = forge_command(forge, workspace, seed);
        let contract = format!("^{}$", regex::escape(&test.contract));
        let test_name = format!(r"^{}(?:\(.*\))?$", regex::escape(&test.name));
        command.args([
            "test",
            "--json",
            "--match-path",
            test.path.to_str().expect("validated candidate path is UTF-8"),
            "--match-contract",
            &contract,
            "--match-test",
            &test_name,
        ]);
        add_dependency_args(&mut command, dependency_config, workspace);
        command.output().wrap_err("failed to run candidate test")
    }

    fn run_mutations(
        &self,
        forge: &Path,
        root: &Path,
        dependency_config: Option<&Config>,
        contract_filter: Option<&str>,
    ) -> Result<Vec<SeedMutation>> {
        self.seed
            .iter()
            .map(|seed| self.run_mutation(forge, root, dependency_config, contract_filter, seed))
            .collect()
    }

    fn run_mutation(
        &self,
        forge: &Path,
        root: &Path,
        dependency_config: Option<&Config>,
        contract_filter: Option<&str>,
        seed: &U256,
    ) -> Result<SeedMutation> {
        let mut command = forge_command(forge, root, seed);
        command.args(["test", "--json", "--mutate"]);
        command.args(&self.mutate);
        if let Some(timeout) = self.mutation_timeout {
            command.args(["--mutation-timeout", &timeout.to_string()]);
        }
        // Adaptive mutation skipping is concurrency-sensitive, so candidate comparisons must use
        // the same stable execution order.
        command.args(["--mutation-jobs", "1"]);
        if let Some(contract) = contract_filter {
            command.args(["--match-contract", contract]);
        }
        if let Some(config) = dependency_config {
            add_dependency_args(&mut command, config, root);
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

fn candidate_test_result(
    output: &Output,
    test: &CandidateTest,
) -> Result<(TestStatus, Option<String>)> {
    let suites = serde_json::from_slice::<serde_json::Value>(&output.stdout)
        .wrap_err_with(|| format!("invalid Forge JSON: {}", stderr(output)))?;
    let suites = suites.as_object().ok_or_else(|| eyre!("Forge JSON is not an object"))?;
    let suite_suffix = format!(":{}", test.contract);
    let mut matches = suites
        .iter()
        .filter(|(suite, _)| suite.ends_with(&suite_suffix))
        .filter_map(|(_, suite)| suite.get("test_results")?.as_object())
        .flat_map(|tests| tests.iter())
        .filter(|(signature, _)| {
            if test.name.contains('(') {
                signature.as_str() == test.name
            } else {
                signature.split_once('(').is_some_and(|(name, _)| name == test.name)
            }
        });
    let (_, result) = matches.next().ok_or_else(|| eyre!("test did not execute"))?;
    ensure!(matches.next().is_none(), "test name matched multiple results");
    let status = serde_json::from_value(
        result.get("status").ok_or_else(|| eyre!("test result has no status"))?.clone(),
    )?;
    let reason = result.get("reason").and_then(serde_json::Value::as_str).map(str::to_string);
    Ok((status, reason))
}

fn candidate_contract_filter(base: Option<&str>, candidate: &Candidate) -> Option<String> {
    let base = base?;
    let contracts = candidate
        .tests
        .iter()
        .map(|test| regex::escape(&test.contract))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
        .join("|");
    let generated = format!("^(?:{contracts})$");
    Some(format!("(?:{base})|(?:{generated})"))
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
            .then_with(|| a.path.cmp(&b.path))
            .then_with(|| a.line.cmp(&b.line))
            .then_with(|| a.column.cmp(&b.column))
            .then_with(|| a.original.cmp(&b.original))
            .then_with(|| a.mutant.cmp(&b.mutant))
    });
    gaps
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

fn resolved_survivor_identities(
    baseline: &[SeedMutation],
    candidate: &[SeedMutation],
) -> BTreeSet<MutationIdentity> {
    let baseline = baseline
        .iter()
        .map(|result| (result.seed, survivor_identities(&result.output)))
        .collect::<BTreeMap<_, _>>();
    let candidate = candidate
        .iter()
        .map(|result| (result.seed, survivor_identities(&result.output)))
        .collect::<BTreeMap<_, _>>();
    if baseline.keys().ne(candidate.keys()) {
        return BTreeSet::new();
    }
    let identities = baseline.values().flatten().cloned().collect::<BTreeSet<_>>();

    identities
        .into_iter()
        .filter(|identity| candidate.values().all(|survivors| !survivors.contains(identity)))
        .collect()
}

fn survivor_identities(output: &MutationJsonOutput) -> BTreeSet<MutationIdentity> {
    output
        .survived_mutants
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
        tests: current.tests.clone(),
    };
    candidate.files.extend(proposal.files.iter().cloned());
    candidate.tests.extend(proposal.tests.iter().cloned());
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
    ensure!(!candidate.tests.is_empty(), "candidate contains no tests");
    ensure!(candidate.files.len() <= MAX_CANDIDATE_FILES, "candidate contains too many files");
    ensure!(
        candidate.files.iter().map(|file| file.content.len()).sum::<usize>() <= MAX_CANDIDATE_BYTES,
        "candidate source is too large"
    );
    let mut paths = HashSet::new();
    for file in &candidate.files {
        validate_candidate_path(&file.path, generated_tests)?;
        ensure!(paths.insert(&file.path), "candidate contains duplicate file paths");
    }
    for test in &candidate.tests {
        validate_candidate_path(&test.path, generated_tests)?;
        ensure!(paths.contains(&test.path), "candidate test names a file absent from files");
        ensure!(!test.contract.is_empty(), "candidate test contract is empty");
        ensure!(!test.name.is_empty(), "candidate test name is empty");
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

fn proposal_feedback(candidate: Option<&Candidate>, evaluation: &Evaluation) -> ProposalFeedback {
    ProposalFeedback {
        candidate_digest: evaluation.candidate_digest.clone(),
        accepted: evaluation.accepted,
        rationale: candidate.map(|candidate| candidate.rationale.clone()),
        tests: candidate.map(|candidate| candidate.tests.clone()).unwrap_or_default(),
        reasons: evaluation.reasons.clone(),
        candidate_results: evaluation.candidate.clone(),
        resolved_survivors: evaluation.resolved_survivors,
        newly_resolved_survivors: evaluation.newly_resolved_survivors,
    }
}

fn forge_command(forge: &Path, workspace: &Path, seed: &U256) -> Command {
    let mut command = Command::new(forge);
    command.current_dir(workspace).env("FOUNDRY_FUZZ_SEED", format!("{seed:#x}"));
    command
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

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).trim().chars().take(2_000).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mutation::{MutationSummaryJson, SurvivedMutantJson};

    fn mutation_result(seed: u64, mutants: &[(usize, &str, &str)]) -> SeedMutation {
        let survived_mutants = BTreeMap::from([(
            "src/Example.sol".to_string(),
            mutants
                .iter()
                .map(|(line, original, mutant)| SurvivedMutantJson {
                    line: *line,
                    column: 1,
                    original: (*original).to_string(),
                    mutant: (*mutant).to_string(),
                })
                .collect(),
        )]);
        SeedMutation {
            seed: U256::from(seed),
            output: MutationJsonOutput {
                summary: MutationSummaryJson {
                    total: mutants.len(),
                    killed: 0,
                    survived: mutants.len(),
                    invalid: 0,
                    skipped: 0,
                    timed_out: 0,
                    mutation_score: 0.0,
                    duration_secs: 0.0,
                },
                survived_mutants,
                timed_out_mutants: BTreeMap::new(),
            },
        }
    }

    #[test]
    fn counts_survivors_resolved_across_all_seeds() {
        let baseline = [
            mutation_result(1, &[(1, "a", "b"), (2, "c", "d")]),
            mutation_result(2, &[(1, "a", "b")]),
        ];
        let candidate = [mutation_result(1, &[(2, "c", "d")]), mutation_result(2, &[])];

        assert_eq!(resolved_survivor_identities(&baseline, &candidate).len(), 1);

        let shifted = [mutation_result(1, &[(2, "c", "d")]), mutation_result(2, &[(1, "a", "b")])];
        assert!(resolved_survivor_identities(&baseline, &shifted).is_empty());

        let partial = [mutation_result(1, &[])];
        assert!(resolved_survivor_identities(&baseline, &partial).is_empty());
    }
}
