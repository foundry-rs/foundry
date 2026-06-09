//! Mutation testing orchestrator.
//!
//! This module coordinates the mutation testing workflow, including:
//! - Filtering source files for mutation
//! - Managing mutation handlers per file
//! - Running mutations in parallel with caching
//! - Aggregating results and reporting

use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use alloy_primitives::keccak256;
use eyre::{Result, WrapErr};
use foundry_cli::utils::FoundryPathExt;
use foundry_common::{compile::ProjectCompiler, sh_println};
use foundry_compilers::{
    Language, ProjectCompileOutput,
    compilers::multi::{MultiCompiler, MultiCompilerLanguage},
    utils::source_files_iter,
};
use foundry_config::{Config, filter::GlobMatcher};
use foundry_evm::opts::EvmOpts;

use crate::{
    cmd::test::FilterArgs,
    mutation::{
        MutationHandler, MutationProgress, MutationReporter, MutationsSummary,
        mutant::{Mutant, MutationResult},
        runner::run_mutations_parallel_with_progress,
    },
};

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, serde::Serialize)]
struct ArtifactCacheFingerprint {
    source: String,
    name: String,
    version: String,
    build_id: String,
    profile: String,
}

#[derive(serde::Serialize)]
struct ExecutionCacheFingerprint<'a> {
    schema: &'static str,
    config: &'a Config,
    evm_opts: &'a EvmOpts,
    filter_args: FilterArgsFingerprint<'a>,
    num_workers: usize,
    artifacts: &'a [ArtifactCacheFingerprint],
}

#[derive(serde::Serialize)]
struct FilterArgsFingerprint<'a> {
    test_pattern: Option<&'a str>,
    test_pattern_inverse: Option<&'a str>,
    contract_pattern: Option<&'a str>,
    contract_pattern_inverse: Option<&'a str>,
    path_pattern: Option<&'a str>,
    path_pattern_inverse: Option<&'a str>,
}

/// Configuration for mutation testing run.
pub struct MutationRunConfig {
    /// Paths to mutate (if empty, use all source files).
    pub mutate_paths: Vec<PathBuf>,
    /// Optional glob pattern to filter paths.
    pub mutate_path_pattern: Option<GlobMatcher>,
    /// Optional contract regex pattern to filter contracts.
    pub mutate_contract_pattern: Option<regex::Regex>,
    /// Number of parallel workers (0 = auto-detect).
    pub num_workers: usize,
    /// Whether to show progress display.
    pub show_progress: bool,
    /// Whether to output JSON (suppress all other output).
    pub json_output: bool,
    /// Test filter (`--match-test`, `--match-contract`, `--match-path`, ...)
    /// applied identically to baseline and every mutant run so they exercise
    /// the same test set.
    pub filter_args: FilterArgs,
    /// Project-relative source files selected for the baseline compile.
    /// Re-rooted into each per-mutant workspace so compilation and execution
    /// honor the same filtered test universe.
    pub selected_sources_relative: Vec<PathBuf>,
    /// EVM isolation flag — mirrors the canonical `forge test` runner so
    /// baseline and mutant runs use the same execution model.
    pub isolate: bool,
}

impl MutationRunConfig {
    /// Determine number of workers, using auto-detection if 0.
    pub fn effective_workers(&self) -> usize {
        if self.num_workers == 0 {
            std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1)
        } else {
            self.num_workers
        }
    }
}

/// Result of a mutation testing run.
pub struct MutationRunResult {
    /// Summary of all mutations across all files.
    pub summary: MutationsSummary,
    /// Whether the run was cancelled (e.g., Ctrl+C).
    pub cancelled: bool,
    /// Duration of the mutation testing run in seconds.
    pub duration_secs: f64,
}

/// Run mutation testing on the project.
///
/// This function encapsulates the mutation testing logic that was previously
/// in the test command. It handles:
/// - Filtering source files based on patterns
/// - Per-file mutation handling with caching
/// - Parallel mutation execution
/// - Result aggregation and reporting
pub async fn run_mutation_testing(
    config: Arc<Config>,
    output: &ProjectCompileOutput<MultiCompiler>,
    evm_opts: EvmOpts,
    mutation_config: MutationRunConfig,
) -> Result<MutationRunResult> {
    let num_workers = mutation_config.effective_workers();
    let json_output = mutation_config.json_output;
    let artifact_link_references = output.artifact_ids().filter_map(|(id, artifact)| {
        let source = project_relative_path(&config.root, &id.source)?;
        let links = artifact
            .all_link_references()
            .into_keys()
            .filter_map(|file| project_relative_path(&config.root, Path::new(&file)))
            .collect::<BTreeSet<_>>();
        Some((source, links))
    });
    let selected_sources_relative = mutation_compile_sources(
        mutation_config.selected_sources_relative.iter().cloned(),
        artifact_link_references,
    );

    // Determine which paths to mutate
    let mutate_paths = resolve_mutate_paths(&config, output, &mutation_config)?;
    let execution_cache_output = ProjectCompiler::new()
        .dynamic_test_linking(config.dynamic_test_linking)
        .quiet(json_output)
        .files(
            selected_sources_relative
                .iter()
                .map(|path| config.root.join(path))
                .filter(|path| path.exists())
                .collect::<Vec<_>>(),
        )
        .compile(&config.project()?)?;
    let execution_cache_key = mutation_execution_cache_key(
        &config,
        &execution_cache_output,
        &evm_opts,
        &mutation_config.filter_args,
        num_workers,
    )?;

    if !mutation_config.show_progress && !json_output {
        sh_println!("Running mutation tests with {} parallel workers...", num_workers)?;
    }

    let mut mutation_summary = MutationsSummary::new();
    let mut cancelled = false;
    let start_time = Instant::now();
    let cancellation_requested = Arc::new(AtomicBool::new(false));
    let ctrlc_handle = {
        let cancellation_requested = Arc::clone(&cancellation_requested);
        tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                cancellation_requested.store(true, Ordering::SeqCst);
            }
        })
    };

    for path in mutate_paths {
        if cancellation_requested.load(Ordering::SeqCst) {
            cancelled = true;
            break;
        }

        if !mutation_config.show_progress && !json_output {
            sh_println!("Running mutation tests for {}", path.display())?;
        }

        // Create handler for this file, optionally restricting to a subset of
        // contracts by name when --mutate-contract is provided.
        let mut handler = MutationHandler::new(path.clone(), config.clone());
        if let Some(filter) = &mutation_config.mutate_contract_pattern {
            handler = handler.with_contract_filter(filter.clone());
        }
        handler.read_source_contract()?;

        // Get build ID for caching
        let build_id = output
            .artifact_ids()
            .find_map(|(id, _)| (id.source == path).then_some(id.build_id))
            .unwrap_or_default();

        // Load persisted survived spans before generating/loading mutants so
        // resumed runs can retain adaptively skipped points as Skipped results
        // while only executing mutants whose spans still need coverage.
        handler.retrieve_survived_spans(&build_id, &execution_cache_key);

        // Generate or load cached mutants. Adaptive resume happens after the
        // full mutant set is known so skipped points are still counted and
        // reported as Skipped instead of disappearing from totals.
        let mut mutants = if let Some(ms) = handler.retrieve_cached_mutants(&build_id) {
            ms
        } else {
            handler.generate_ast().await?;
            handler.mutations.clone()
        };

        if mutants.is_empty() {
            if !mutation_config.show_progress && !json_output {
                sh_println!("  No mutants generated for {}", path.display())?;
            }
            continue;
        }

        // Check for cached results only after the current mutant set is known.
        // The result cache carries a count/hash of that set so stale or partial
        // caches cannot suppress newly generated mutants.
        if let Some(prior) =
            handler.retrieve_cached_mutant_results(&build_id, &execution_cache_key, &mutants)
        {
            if !mutation_config.show_progress && !json_output {
                sh_println!("  Using cached results for {} mutants", prior.len())?;
            }
            for (mutant, status) in prior {
                match status {
                    MutationResult::Dead => handler.add_dead_mutant(mutant),
                    MutationResult::Alive => handler.add_survived_mutant(mutant),
                    MutationResult::Invalid => handler.add_invalid_mutant(mutant),
                    MutationResult::Skipped => handler.add_skipped_mutant(mutant),
                    MutationResult::TimedOut => handler.add_timed_out_mutant(mutant),
                }
            }
            mutation_summary.merge(handler.get_report());
            continue;
        }

        // Sort mutations by span for optimal adaptive testing
        mutants.sort_by(|a, b| {
            a.span.lo().0.cmp(&b.span.lo().0).then_with(|| b.span.hi().0.cmp(&a.span.hi().0))
        });

        let (mutants_to_test, skipped_results) =
            partition_adaptively_skipped_mutants(&mut handler, &mutants);

        // Create progress display if enabled (not in JSON mode)
        let progress = if mutation_config.show_progress && !json_output {
            let p = MutationProgress::with_timeout(
                mutants_to_test.len(),
                num_workers,
                config.mutation.timeout,
            );
            // Show relative path from project root
            let display_path =
                path.strip_prefix(&config.root).unwrap_or(&path).display().to_string();
            p.set_current_file(&display_path);
            Some(p)
        } else if !json_output {
            sh_println!(
                "  Generated {} mutants; testing {}, adaptively skipped {}",
                mutants.len(),
                mutants_to_test.len(),
                skipped_results.len()
            )?;
            None
        } else {
            None
        };

        // Run mutations in parallel using isolated workspaces
        let batch = run_mutations_parallel_with_progress(
            mutants_to_test.clone(),
            path.clone(),
            handler.src.clone(),
            config.clone(),
            evm_opts.clone(),
            num_workers,
            progress.clone(),
            json_output,
            mutation_config.filter_args.clone(),
            Arc::new(selected_sources_relative.clone()),
            mutation_config.isolate,
            Arc::clone(&cancellation_requested),
        )?;
        let file_cancelled = batch.cancelled;

        // Collect results for caching
        let mut results_vec = Vec::with_capacity(skipped_results.len() + batch.results.len());
        results_vec.extend(skipped_results);
        for result in batch.results {
            results_vec.push((result.mutant.clone(), result.result.clone()));
            match result.result {
                MutationResult::Dead => handler.add_dead_mutant(result.mutant),
                MutationResult::Alive => {
                    handler.mark_span_survived(result.mutant.span);
                    handler.add_survived_mutant(result.mutant);
                }
                MutationResult::Invalid => handler.add_invalid_mutant(result.mutant),
                MutationResult::Skipped => handler.add_skipped_mutant(result.mutant),
                MutationResult::TimedOut => handler.add_timed_out_mutant(result.mutant),
            }
        }

        // Detect cancellation early so we can decide whether the result set is
        // complete before persisting it. Without this guard a Ctrl+C mid-run
        // would write a *partial* results vector to the cache and the next run
        // would treat that subset as the full answer for this file.
        let complete_run = !file_cancelled && results_vec.len() == mutants.len();

        // Persist results for caching only when the run for this file is
        // complete. Partial caches are silent correctness bugs:
        //   - cancelled runs would be reloaded as authoritative
        //   - non-cancelled-but-short result vectors indicate a bug, not a hit
        // The mutants list itself is fine to persist (it's deterministic from
        // the AST + operator set) and so are survived spans (best-effort hint).
        //
        // Sort the persisted result vector by mutant span so the on-disk
        // cache is independent of rayon worker completion order; otherwise
        // the cache file changes content-hash run-to-run even when the
        // outcomes are identical, defeating diffing and reproducibility.
        results_vec.sort_by(|(a, _), (b, _)| {
            a.span.lo().0.cmp(&b.span.lo().0).then_with(|| a.span.hi().0.cmp(&b.span.hi().0))
        });
        if !mutants.is_empty() && !build_id.is_empty() {
            let _ = handler.persist_cached_mutants(&build_id, &mutants);
            if complete_run {
                let _ = handler.persist_cached_results(
                    &build_id,
                    &execution_cache_key,
                    &mutants,
                    &results_vec,
                );
            }
            let _ = handler.persist_survived_spans(&build_id, &execution_cache_key);
        }

        mutation_summary.merge(handler.get_report());

        // If cancelled, break out of the loop
        if file_cancelled {
            cancelled = true;
            break;
        }
    }
    cancelled |= cancellation_requested.load(Ordering::SeqCst);

    // Report results
    let duration = start_time.elapsed();
    let duration_secs = duration.as_secs_f64();

    // Only show human-readable report if not in JSON mode
    if !json_output {
        MutationReporter::new().report(&mutation_summary, duration);
    }

    ctrlc_handle.abort();

    Ok(MutationRunResult { summary: mutation_summary, cancelled, duration_secs })
}

/// Build the cache discriminator for mutation *results*.
///
/// Mutant generation only depends on the source build + selected mutators, but
/// result correctness depends on the compiled test universe and execution
/// settings. Hashing the full serialized config intentionally includes fuzz /
/// invariant settings, test filters, fs permissions, sender/balance/env values,
/// and future config fields unless explicitly skipped by `Config` itself. The
/// artifact fingerprint covers the same filter-selected source and test build
/// IDs that baseline and mutant runs compile. Worker count is included because
/// adaptive span skipping is concurrency-sensitive.
fn mutation_execution_cache_key(
    config: &Config,
    output: &ProjectCompileOutput<MultiCompiler>,
    evm_opts: &EvmOpts,
    filter_args: &FilterArgs,
    num_workers: usize,
) -> Result<String> {
    let artifacts = output
        .artifact_ids()
        .map(|(id, _)| ArtifactCacheFingerprint {
            source: id.source.display().to_string(),
            name: id.name,
            version: id.version.to_string(),
            build_id: id.build_id,
            profile: id.profile,
        })
        .collect::<Vec<_>>();
    mutation_execution_cache_key_from_parts(config, evm_opts, filter_args, num_workers, artifacts)
}

fn mutation_execution_cache_key_from_parts(
    config: &Config,
    evm_opts: &EvmOpts,
    filter_args: &FilterArgs,
    num_workers: usize,
    mut artifacts: Vec<ArtifactCacheFingerprint>,
) -> Result<String> {
    artifacts.sort();
    let fingerprint = ExecutionCacheFingerprint {
        schema: "mutation-results-v1",
        config,
        evm_opts,
        filter_args: filter_args_fingerprint(filter_args),
        num_workers,
        artifacts: &artifacts,
    };
    let encoded = serde_json::to_vec(&fingerprint)
        .wrap_err("failed to encode mutation execution cache key")?;

    Ok(keccak256(encoded).to_string())
}

fn filter_args_fingerprint(filter_args: &FilterArgs) -> FilterArgsFingerprint<'_> {
    FilterArgsFingerprint {
        test_pattern: filter_args.test_pattern.as_ref().map(|re| re.as_str()),
        test_pattern_inverse: filter_args.test_pattern_inverse.as_ref().map(|re| re.as_str()),
        contract_pattern: filter_args.contract_pattern.as_ref().map(|re| re.as_str()),
        contract_pattern_inverse: filter_args
            .contract_pattern_inverse
            .as_ref()
            .map(|re| re.as_str()),
        path_pattern: filter_args.path_pattern.as_ref().map(|glob| glob.as_str()),
        path_pattern_inverse: filter_args.path_pattern_inverse.as_ref().map(|glob| glob.as_str()),
    }
}

fn project_relative_path(root: &Path, path: &Path) -> Option<PathBuf> {
    if path.is_relative() {
        return Some(path.to_path_buf());
    }

    if let Ok(stripped) = path.strip_prefix(root) {
        return Some(stripped.to_path_buf());
    }

    path.canonicalize().ok()?.strip_prefix(root.canonicalize().ok()?).ok().map(PathBuf::from)
}

fn mutation_compile_sources(
    selected_sources: impl IntoIterator<Item = PathBuf>,
    artifact_link_references: impl IntoIterator<Item = (PathBuf, BTreeSet<PathBuf>)>,
) -> Vec<PathBuf> {
    let link_edges = artifact_link_references.into_iter().collect::<BTreeMap<_, _>>();
    let mut selected_sources_relative = selected_sources.into_iter().collect::<BTreeSet<_>>();
    let mut queue = selected_sources_relative.iter().cloned().collect::<Vec<_>>();

    while let Some(source) = queue.pop() {
        if let Some(links) = link_edges.get(&source) {
            for link in links {
                if selected_sources_relative.insert(link.clone()) {
                    queue.push(link.clone());
                }
            }
        }
    }

    selected_sources_relative.into_iter().collect()
}

fn partition_adaptively_skipped_mutants(
    handler: &mut MutationHandler,
    mutants: &[Mutant],
) -> (Vec<Mutant>, Vec<(Mutant, MutationResult)>) {
    let mut skipped_results = Vec::new();
    let mutants_to_test = mutants
        .iter()
        .filter_map(|mutant| {
            if handler.should_skip_span(mutant.span) {
                handler.add_skipped_mutant(mutant.clone());
                skipped_results.push((mutant.clone(), MutationResult::Skipped));
                None
            } else {
                Some(mutant.clone())
            }
        })
        .collect();

    (mutants_to_test, skipped_results)
}

/// Resolve which paths to mutate based on configuration.
///
/// Resolution order:
/// 1. Pick the *base* set of candidate files:
///    - `--mutate-path <GLOB>` → all source files matching the glob, OR
///    - explicit `--mutate PATH...` → those validated files, OR
///    - default → every Solidity file under `config.src`.
/// 2. If `--mutate-contract <REGEX>` is set, intersect the base set with files that contain at
///    least one contract whose name matches the regex. The per-file contract filter still
///    re-applies inside the handler.
fn resolve_mutate_paths(
    config: &Config,
    output: &ProjectCompileOutput<MultiCompiler>,
    mutation_config: &MutationRunConfig,
) -> Result<Vec<PathBuf>> {
    // 1. Base path set.
    let base: Vec<PathBuf> = if let Some(pattern) = &mutation_config.mutate_path_pattern {
        let paths: Vec<_> = source_files_iter(&config.src, MultiCompilerLanguage::FILE_EXTENSIONS)
            .filter(|entry| entry.is_sol() && !entry.is_sol_test() && pattern.is_match(entry))
            .collect();
        if paths.is_empty() {
            eyre::bail!("no source matched --mutate-path pattern `{pattern}`");
        }
        paths
    } else if !mutation_config.mutate_paths.is_empty() {
        let root_canon =
            config.root.canonicalize().wrap_err("failed to canonicalize project root")?;
        let mut validated = Vec::with_capacity(mutation_config.mutate_paths.len());
        for path in &mutation_config.mutate_paths {
            let resolved = if path.is_relative() { config.root.join(path) } else { path.clone() };
            if !resolved.exists() {
                eyre::bail!("mutate path does not exist: {}", resolved.display());
            }
            if !resolved.is_file() {
                eyre::bail!("mutate path is not a file: {}", resolved.display());
            }
            let canon = resolved
                .canonicalize()
                .wrap_err_with(|| format!("failed to canonicalize: {}", resolved.display()))?;
            if !canon.starts_with(&root_canon) {
                eyre::bail!("mutate path is outside the project root: {}", resolved.display());
            }
            if !canon.is_sol() {
                eyre::bail!("mutate path is not a Solidity file: {}", resolved.display());
            }
            if canon.is_sol_test() {
                eyre::bail!(
                    "mutate path is a test file, not a source file: {}",
                    resolved.display()
                );
            }
            validated.push(canon);
        }
        validated
    } else {
        source_files_iter(&config.src, MultiCompilerLanguage::FILE_EXTENSIONS)
            .filter(|entry| entry.is_sol() && !entry.is_sol_test())
            .collect()
    };

    // 2. Intersect with `--mutate-contract` if set, so explicit `--mutate <paths>` combined with
    //    `--mutate-contract <regex>` does the principled thing (the listed files, restricted to
    //    those containing a matching contract) instead of silently expanding to every source file.
    let paths = if let Some(contract_pattern) = &mutation_config.mutate_contract_pattern {
        let matching_sources: HashSet<PathBuf> = output
            .artifact_ids()
            .filter_map(|(id, _)| contract_pattern.is_match(&id.name).then_some(id.source.clone()))
            .collect();
        let paths: Vec<_> =
            base.into_iter().filter(|entry| matching_sources.contains(entry)).collect();
        if paths.is_empty() {
            if mutation_config.mutate_paths.is_empty()
                && mutation_config.mutate_path_pattern.is_none()
            {
                eyre::bail!("no source matched --mutate-contract pattern `{contract_pattern}`");
            }
            eyre::bail!("no source matched --mutate-contract within the selected mutation paths");
        }
        paths
    } else {
        base
    };

    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    use crate::mutation::mutant::MutationType;
    use solar::{ast::Span, interface::BytePos};

    fn artifact(build_id: &str) -> ArtifactCacheFingerprint {
        ArtifactCacheFingerprint {
            source: "src/Counter.sol".to_string(),
            name: "Counter".to_string(),
            version: "0.8.30".to_string(),
            build_id: build_id.to_string(),
            profile: "default".to_string(),
        }
    }

    fn filter_args() -> FilterArgs {
        FilterArgs {
            test_pattern: None,
            test_pattern_inverse: None,
            contract_pattern: None,
            contract_pattern_inverse: None,
            path_pattern: None,
            path_pattern_inverse: None,
            coverage_pattern_inverse: None,
        }
    }

    fn mutant(lo: u32, hi: u32) -> Mutant {
        Mutant {
            path: PathBuf::from("src/Counter.sol"),
            span: Span::new(BytePos(lo), BytePos(hi)),
            mutation: MutationType::DeleteExpression,
            original: "number++".to_string(),
            source_line: "number++;".to_string(),
            line_number: 1,
            column_number: 1,
        }
    }

    #[test]
    fn execution_cache_key_changes_when_fuzz_config_changes() {
        let first = Config::default();
        let mut second = first.clone();
        second.fuzz.runs += 1;

        let evm_opts = EvmOpts::default();
        let filter_args = filter_args();
        let artifacts = vec![artifact("build-a")];

        let first_key = mutation_execution_cache_key_from_parts(
            &first,
            &evm_opts,
            &filter_args,
            1,
            artifacts.clone(),
        )
        .unwrap();
        let second_key =
            mutation_execution_cache_key_from_parts(&second, &evm_opts, &filter_args, 1, artifacts)
                .unwrap();

        assert_ne!(first_key, second_key);
    }

    #[test]
    fn execution_cache_key_changes_when_evm_options_change() {
        let config = Config::default();
        let first = EvmOpts::default();
        let mut second = first.clone();
        second.memory_limit = first.memory_limit + 1;

        let filter_args = filter_args();
        let artifacts = vec![artifact("build-a")];

        let first_key = mutation_execution_cache_key_from_parts(
            &config,
            &first,
            &filter_args,
            1,
            artifacts.clone(),
        )
        .unwrap();
        let second_key =
            mutation_execution_cache_key_from_parts(&config, &second, &filter_args, 1, artifacts)
                .unwrap();

        assert_ne!(first_key, second_key);
    }

    #[test]
    fn execution_cache_key_changes_when_compiled_artifacts_change() {
        let config = Config::default();
        let evm_opts = EvmOpts::default();
        let filter_args = filter_args();

        let first_key = mutation_execution_cache_key_from_parts(
            &config,
            &evm_opts,
            &filter_args,
            1,
            vec![artifact("build-a")],
        )
        .unwrap();
        let second_key = mutation_execution_cache_key_from_parts(
            &config,
            &evm_opts,
            &filter_args,
            1,
            vec![artifact("build-b")],
        )
        .unwrap();

        assert_ne!(first_key, second_key);
    }

    #[test]
    fn execution_cache_key_sorts_artifacts_before_hashing() {
        let config = Config::default();
        let evm_opts = EvmOpts::default();
        let filter_args = filter_args();

        let first = vec![artifact("build-a"), artifact("build-b")];
        let second = vec![artifact("build-b"), artifact("build-a")];

        let first_key =
            mutation_execution_cache_key_from_parts(&config, &evm_opts, &filter_args, 1, first)
                .unwrap();
        let second_key =
            mutation_execution_cache_key_from_parts(&config, &evm_opts, &filter_args, 1, second)
                .unwrap();

        assert_eq!(first_key, second_key);
    }

    #[test]
    fn execution_cache_key_changes_when_worker_count_changes() {
        let config = Config::default();
        let evm_opts = EvmOpts::default();
        let filter_args = filter_args();
        let artifacts = vec![artifact("build-a")];

        let first_key = mutation_execution_cache_key_from_parts(
            &config,
            &evm_opts,
            &filter_args,
            1,
            artifacts.clone(),
        )
        .unwrap();
        let second_key =
            mutation_execution_cache_key_from_parts(&config, &evm_opts, &filter_args, 4, artifacts)
                .unwrap();

        assert_ne!(first_key, second_key);
    }

    #[test]
    fn execution_cache_key_changes_when_match_test_filter_changes() {
        let config = Config::default();
        let evm_opts = EvmOpts::default();
        let mut first_filter = filter_args();
        let mut second_filter = filter_args();
        first_filter.test_pattern = Some(regex::Regex::new("testA|testAlpha").unwrap());
        second_filter.test_pattern = Some(regex::Regex::new("testB|testBeta").unwrap());
        let artifacts = vec![artifact("build-a")];

        let first_key = mutation_execution_cache_key_from_parts(
            &config,
            &evm_opts,
            &first_filter,
            1,
            artifacts.clone(),
        )
        .unwrap();
        let second_key = mutation_execution_cache_key_from_parts(
            &config,
            &evm_opts,
            &second_filter,
            1,
            artifacts,
        )
        .unwrap();

        assert_ne!(first_key, second_key);
    }

    #[test]
    fn execution_cache_key_changes_when_match_path_filter_changes() {
        let config = Config::default();
        let evm_opts = EvmOpts::default();
        let mut first_filter = filter_args();
        let mut second_filter = filter_args();
        first_filter.path_pattern = Some(GlobMatcher::from_str("test/A.t.sol").unwrap());
        second_filter.path_pattern = Some(GlobMatcher::from_str("test/B.t.sol").unwrap());
        let artifacts = vec![artifact("build-a")];

        let first_key = mutation_execution_cache_key_from_parts(
            &config,
            &evm_opts,
            &first_filter,
            1,
            artifacts.clone(),
        )
        .unwrap();
        let second_key = mutation_execution_cache_key_from_parts(
            &config,
            &evm_opts,
            &second_filter,
            1,
            artifacts,
        )
        .unwrap();

        assert_ne!(first_key, second_key);
    }

    #[test]
    fn mutation_compile_sources_only_include_selected_link_reference_closure() {
        let sources = mutation_compile_sources(
            [PathBuf::from("test/Selected.t.sol")],
            [
                (
                    PathBuf::from("test/Selected.t.sol"),
                    BTreeSet::from([PathBuf::from("test/SelectedLinkedHelper.sol")]),
                ),
                (
                    PathBuf::from("test/SelectedLinkedHelper.sol"),
                    BTreeSet::from([PathBuf::from("test/TransitiveLinkedHelper.sol")]),
                ),
                (
                    PathBuf::from("test/Unrelated.t.sol"),
                    BTreeSet::from([PathBuf::from("test/UnusedLinkedHelper.sol")]),
                ),
            ],
        );

        assert_eq!(
            sources,
            vec![
                PathBuf::from("test/Selected.t.sol"),
                PathBuf::from("test/SelectedLinkedHelper.sol"),
                PathBuf::from("test/TransitiveLinkedHelper.sol"),
            ]
        );
    }

    #[test]
    fn resumed_adaptive_skips_are_reported_as_skipped_results() {
        let mut handler =
            MutationHandler::new(PathBuf::from("src/Counter.sol"), Arc::new(Config::default()));
        handler.mark_span_survived(Span::new(BytePos(10), BytePos(20)));

        let exact_survivor = mutant(10, 20);
        let skipped_child = mutant(12, 18);
        let unrelated = mutant(30, 40);
        let (mutants_to_test, skipped_results) = partition_adaptively_skipped_mutants(
            &mut handler,
            &[exact_survivor.clone(), skipped_child.clone(), unrelated.clone()],
        );

        assert_eq!(mutants_to_test.len(), 2);
        assert_eq!(mutants_to_test[0].span, exact_survivor.span);
        assert_eq!(mutants_to_test[1].span, unrelated.span);
        assert_eq!(skipped_results.len(), 1);
        assert!(matches!(skipped_results[0].1, MutationResult::Skipped));
        assert_eq!(skipped_results[0].0.span, skipped_child.span);
        assert_eq!(handler.get_report().total_skipped(), 1);
        assert_eq!(handler.get_report().total_mutants(), 1);
    }
}
