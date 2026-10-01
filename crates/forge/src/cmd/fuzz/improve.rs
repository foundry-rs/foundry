use crate::{mutation::MutationJsonOutput, workspace};
use alloy_primitives::keccak256;
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
    seed: Vec<String>,

    /// Maximum number of proposal rounds.
    #[arg(long, default_value_t = 1, value_name = "N")]
    rounds: usize,

    /// Best-effort timeout for each mutant.
    #[arg(long, value_name = "SECONDS")]
    mutation_timeout: Option<u32>,

    /// Parallel mutation workers.
    #[arg(long, value_name = "JOBS")]
    mutation_jobs: Option<usize>,

    /// Restrict mutation evaluation to matching test contracts.
    #[arg(long, value_name = "REGEX")]
    match_contract: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Candidate {
    schema: String,
    rationale: String,
    files: Vec<CandidateFile>,
    tests: Vec<CandidateTest>,
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
    mutate: &'a [PathBuf],
    seeds: &'a [String],
    baseline: &'a [PromptMutation<'a>],
    mutation_gaps: &'a [MutationGap],
    previous_feedback: &'a [ProposalFeedback],
    output_contract: OutputContract,
}

#[derive(Debug, Serialize)]
struct OutputContract {
    schema: &'static str,
    allowed_path_prefix: &'static str,
    maximum_files: usize,
    maximum_total_bytes: usize,
    note: &'static str,
}

#[derive(Clone, Debug)]
struct SeedMutation {
    seed: String,
    output: MutationJsonOutput,
}

#[derive(Debug, Serialize)]
struct PromptMutation<'a> {
    seed: &'a str,
    summary: &'a crate::mutation::MutationSummaryJson,
}

#[derive(Debug, Serialize)]
struct MutationGap {
    path: PathBuf,
    line: usize,
    column: usize,
    seeds: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ProposalFeedback {
    candidate_digest: String,
    accepted: bool,
    rationale: Option<String>,
    tests: Vec<CandidateTest>,
    reasons: Vec<String>,
    candidate_results: Vec<MutationResultSummary>,
    minimum_new_kills: i64,
}

#[derive(Clone, Debug, Serialize)]
struct MutationResultSummary {
    seed: String,
    summary: crate::mutation::MutationSummaryJson,
}

#[derive(Debug, Serialize)]
struct Evaluation {
    candidate_digest: String,
    accepted: bool,
    reasons: Vec<String>,
    baseline: Vec<MutationResultSummary>,
    candidate: Vec<MutationResultSummary>,
    minimum_new_kills: i64,
}

impl FuzzImproveArgs {
    pub fn run(self) -> Result<()> {
        ensure!(self.seed.len() >= 2, "at least two --seed values are required");
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
        if let Some(jobs) = self.mutation_jobs {
            ensure!(jobs > 0, "--mutation-jobs must be greater than zero");
        }

        let root = self.root.canonicalize().wrap_err("failed to resolve project root")?;
        let config = Config::load_with_root(&root)?.sanitized();
        let brief = fs::read_to_string(&self.brief).wrap_err("failed to read campaign brief")?;
        let generator = self.generator.canonicalize().wrap_err("failed to resolve generator")?;
        let forge = std::env::current_exe().wrap_err("failed to resolve Forge executable")?;
        let baseline =
            self.run_mutations(&forge, &config.root, None, self.match_contract.as_deref())?;
        let prompt_baseline = baseline
            .iter()
            .map(|result| PromptMutation { seed: &result.seed, summary: &result.output.summary })
            .collect::<Vec<_>>();
        let mutation_gaps = mutation_gaps(&baseline);
        let cache_root = config.cache_path.join("fuzz-improve");
        fs::create_dir_all(&cache_root)?;

        let mut evaluations = Vec::new();
        let mut feedback = Vec::new();
        let mut best = None;
        for round in 1..=self.rounds {
            let round_dir = tempfile::Builder::new().prefix("forge-fuzz-improve-").tempdir()?;
            let prompt_path = round_dir.path().join("prompt.json");
            let candidate_path = round_dir.path().join("candidate.json");
            let prompt = GeneratorPrompt {
                schema: PROMPT_SCHEMA,
                round,
                project: &config.root,
                brief: &brief,
                mutate: &self.mutate,
                seeds: &self.seed,
                baseline: &prompt_baseline,
                mutation_gaps: &mutation_gaps,
                previous_feedback: &feedback,
                output_contract: OutputContract {
                    schema: CANDIDATE_SCHEMA,
                    allowed_path_prefix: "test/generated/",
                    maximum_files: MAX_CANDIDATE_FILES,
                    maximum_total_bytes: MAX_CANDIDATE_BYTES,
                    note: "Generated assertions are candidates for human review, not proofs.",
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
            let candidate = if output.status.success() {
                read_candidate(&candidate_path)
            } else {
                Err(eyre!("generator failed: {}", stderr(&output)))
            };
            let evaluation = match candidate {
                Ok(candidate) => {
                    let evaluation = self.evaluate(&config, &forge, &baseline, &candidate)?;
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
                    if evaluation.accepted
                        && best.as_ref().is_none_or(|(score, _): &(i64, PathBuf)| {
                            evaluation.minimum_new_kills > *score
                        })
                    {
                        best = Some((evaluation.minimum_new_kills, candidate_cache));
                    }
                    feedback.push(proposal_feedback(Some(&candidate), &evaluation));
                    evaluation
                }
                Err(error) => {
                    let evaluation = Evaluation {
                        candidate_digest: format!("round-{round}"),
                        accepted: false,
                        reasons: vec![error.to_string()],
                        baseline: mutation_result_summaries(&baseline),
                        candidate: vec![],
                        minimum_new_kills: 0,
                    };
                    feedback.push(proposal_feedback(None, &evaluation));
                    evaluation
                }
            };
            evaluations.push(evaluation);
        }

        fs::write(cache_root.join("rounds.json"), serde_json::to_vec_pretty(&evaluations)?)?;
        if let Some((new_kills, path)) = best {
            let path = path.strip_prefix(&config.root).unwrap_or(&path);
            sh_println!(
                "accepted candidate: {} (+{new_kills} kills on every seed)",
                path.display()
            )?;
        } else {
            sh_println!("no candidate improved mutation kills on every seed")?;
        }
        Ok(())
    }

    fn evaluate(
        &self,
        config: &Config,
        forge: &Path,
        baseline: &[SeedMutation],
        candidate: &Candidate,
    ) -> Result<Evaluation> {
        let digest = keccak256(serde_json::to_vec(candidate)?).to_string();
        let candidate_workspace =
            tempfile::Builder::new().prefix("forge-fuzz-improve-").tempdir()?;
        workspace::copy_project(config, candidate_workspace.path())?;
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
                if !output.status.success() {
                    reasons.push(format!(
                        "{}::{} failed on seed {seed}: {}",
                        test.contract,
                        test.name,
                        stderr(&output)
                    ));
                } else if !String::from_utf8_lossy(&output.stdout).contains(&test.name) {
                    reasons.push(format!(
                        "{}::{} did not execute on seed {seed}",
                        test.contract, test.name
                    ));
                }
            }
        }

        let candidate_results = if reasons.is_empty() {
            let contract_filter =
                candidate_contract_filter(self.match_contract.as_deref(), candidate);
            self.run_mutations(
                forge,
                candidate_workspace.path(),
                Some(config),
                Some(&contract_filter),
            )?
        } else {
            vec![]
        };
        let minimum_new_kills = baseline
            .iter()
            .zip(&candidate_results)
            .map(|(before, after)| {
                after.output.summary.killed as i64 - before.output.summary.killed as i64
            })
            .min()
            .unwrap_or_default();
        if reasons.is_empty() && candidate_results.len() != baseline.len() {
            reasons.push("candidate mutation results did not cover every seed".to_string());
        }
        if reasons.is_empty()
            && baseline
                .iter()
                .chain(&candidate_results)
                .any(|result| result.output.summary.timed_out > 0)
        {
            reasons.push("mutation evaluation contained timed-out mutants".to_string());
        }
        if reasons.is_empty()
            && baseline.iter().zip(&candidate_results).any(|(before, after)| {
                before.output.summary.total != after.output.summary.total
                    || before.output.summary.invalid != after.output.summary.invalid
                    || before.output.summary.timed_out != after.output.summary.timed_out
            })
        {
            reasons.push("candidate changed the mutant population".to_string());
        }
        if reasons.is_empty() && minimum_new_kills <= 0 {
            reasons.push("candidate did not add a mutation kill on every seed".to_string());
        }

        Ok(Evaluation {
            candidate_digest: digest,
            accepted: reasons.is_empty(),
            reasons,
            baseline: mutation_result_summaries(baseline),
            candidate: mutation_result_summaries(&candidate_results),
            minimum_new_kills,
        })
    }

    fn run_candidate_test(
        &self,
        forge: &Path,
        workspace: &Path,
        dependency_config: &Config,
        seed: &str,
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
            .map(|seed| {
                let mut command = forge_command(forge, root, seed);
                command.args(["test", "--json", "--mutate"]);
                command.args(&self.mutate);
                if let Some(timeout) = self.mutation_timeout {
                    command.args(["--mutation-timeout", &timeout.to_string()]);
                }
                if let Some(jobs) = self.mutation_jobs {
                    command.args(["--mutation-jobs", &jobs.to_string()]);
                }
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
                    seed: seed.clone(),
                    output: serde_json::from_slice(&output.stdout)
                        .wrap_err_with(|| format!("invalid mutation JSON for seed {seed}"))?,
                })
            })
            .collect()
    }
}

fn candidate_contract_filter(base: Option<&str>, candidate: &Candidate) -> String {
    let contracts = candidate
        .tests
        .iter()
        .map(|test| regex::escape(&test.contract))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
        .join("|");
    let generated = format!("^(?:{contracts})$");
    match base {
        Some(base) => format!("(?:{base})|(?:{generated})"),
        None => generated,
    }
}

fn mutation_gaps(results: &[SeedMutation]) -> Vec<MutationGap> {
    let mut gaps = BTreeMap::<(PathBuf, usize, usize), BTreeSet<String>>::new();
    for result in results {
        for (path, mutants) in &result.output.survived_mutants {
            for mutant in mutants {
                gaps.entry((PathBuf::from(path), mutant.line, mutant.column))
                    .or_default()
                    .insert(result.seed.clone());
            }
        }
    }
    gaps.into_iter()
        .map(|((path, line, column), seeds)| MutationGap {
            path,
            line,
            column,
            seeds: seeds.into_iter().collect(),
        })
        .collect()
}

fn mutation_result_summaries(results: &[SeedMutation]) -> Vec<MutationResultSummary> {
    results
        .iter()
        .map(|result| MutationResultSummary {
            seed: result.seed.clone(),
            summary: result.output.summary.clone(),
        })
        .collect()
}

fn read_candidate(path: &Path) -> Result<Candidate> {
    let candidate: Candidate = serde_json::from_slice(
        &fs::read(path).wrap_err_with(|| format!("failed to read {}", path.display()))?,
    )
    .wrap_err("invalid candidate JSON")?;
    ensure!(candidate.schema == CANDIDATE_SCHEMA, "unsupported candidate schema");
    ensure!(!candidate.rationale.trim().is_empty(), "candidate rationale is empty");
    ensure!(!candidate.files.is_empty(), "candidate contains no files");
    ensure!(!candidate.tests.is_empty(), "candidate contains no tests");
    ensure!(candidate.files.len() <= MAX_CANDIDATE_FILES, "candidate contains too many files");
    ensure!(
        candidate.files.iter().map(|file| file.content.len()).sum::<usize>() <= MAX_CANDIDATE_BYTES,
        "candidate source is too large"
    );
    let mut paths = HashSet::new();
    for file in &candidate.files {
        validate_candidate_path(&file.path)?;
        ensure!(paths.insert(&file.path), "candidate contains duplicate file paths");
    }
    for test in &candidate.tests {
        validate_candidate_path(&test.path)?;
        ensure!(paths.contains(&test.path), "candidate test names a file absent from files");
        ensure!(!test.contract.is_empty(), "candidate test contract is empty");
        ensure!(!test.name.is_empty(), "candidate test name is empty");
    }
    Ok(candidate)
}

fn validate_candidate_path(path: &Path) -> Result<()> {
    ensure!(
        path.extension().is_some_and(|extension| extension == "sol"),
        "candidate files must be Solidity sources"
    );
    ensure!(path.starts_with("test/generated"), "candidate files must be under test/generated/");
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
        minimum_new_kills: evaluation.minimum_new_kills,
    }
}

fn forge_command(forge: &Path, workspace: &Path, seed: &str) -> Command {
    let mut command = Command::new(forge);
    command.current_dir(workspace).env("FOUNDRY_FUZZ_SEED", seed);
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
