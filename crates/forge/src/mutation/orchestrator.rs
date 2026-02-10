//! Mutation testing orchestrator.
//!
//! This module coordinates the mutation testing workflow, including:
//! - Filtering source files for mutation
//! - Managing mutation handlers per file
//! - Running mutations in parallel with caching
//! - Aggregating results and reporting

use std::{path::PathBuf, sync::Arc, time::Instant};

use eyre::Result;
use foundry_cli::utils::FoundryPathExt;
use foundry_common::sh_println;
use foundry_compilers::{
    Language, ProjectCompileOutput,
    compilers::multi::{MultiCompiler, MultiCompilerLanguage},
    utils::source_files_iter,
};
use foundry_config::{Config, filter::GlobMatcher};
use foundry_evm::{Env, opts::EvmOpts};

use crate::mutation::{
    MutationHandler, MutationProgress, MutationReporter, MutationsSummary, mutant::MutationResult,
    runner::run_mutations_parallel_with_progress,
};

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
    env: Env,
    mutation_config: MutationRunConfig,
) -> Result<MutationRunResult> {
    let num_workers = mutation_config.effective_workers();
    let json_output = mutation_config.json_output;

    // Determine which paths to mutate
    let mutate_paths = resolve_mutate_paths(&config, output, &mutation_config)?;

    if !mutation_config.show_progress && !json_output {
        sh_println!("Running mutation tests with {} parallel workers...", num_workers)?;
    }

    let mut mutation_summary = MutationsSummary::new();
    let mut cancelled = false;
    let start_time = Instant::now();

    for path in mutate_paths {
        if !mutation_config.show_progress && !json_output {
            sh_println!("Running mutation tests for {}", path.display())?;
        }

        // Create handler for this file
        let mut handler = MutationHandler::new(path.clone(), config.clone());
        handler.read_source_contract()?;

        // Get build ID for caching
        let build_id = output
            .artifact_ids()
            .find_map(|(id, _)| if id.source == path { Some(id.build_id) } else { None })
            .unwrap_or_default();

        // Check for cached results
        if let Some(prior) = handler.retrieve_cached_mutant_results(&build_id) {
            if !mutation_config.show_progress && !json_output {
                sh_println!("  Using cached results for {} mutants", prior.len())?;
            }
            for (mutant, status) in prior {
                match status {
                    MutationResult::Dead => handler.add_dead_mutant(mutant),
                    MutationResult::Alive => handler.add_survived_mutant(mutant),
                    MutationResult::Invalid => handler.add_invalid_mutant(mutant),
                    MutationResult::Skipped => handler.add_skipped_mutant(mutant),
                }
            }
            mutation_summary.merge(handler.get_report());
            continue;
        }

        // Load survived spans for adaptive mutation testing
        handler.retrieve_survived_spans(&build_id);

        // Generate or load cached mutants
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

        // Sort mutations by span for optimal adaptive testing
        mutants.sort_by(|a, b| {
            let lo_cmp = a.span.lo().0.cmp(&b.span.lo().0);
            if lo_cmp != std::cmp::Ordering::Equal {
                lo_cmp
            } else {
                b.span.hi().0.cmp(&a.span.hi().0)
            }
        });

        // Create progress display if enabled (not in JSON mode)
        let progress = if mutation_config.show_progress && !json_output {
            let p = MutationProgress::new(mutants.len(), num_workers);
            // Show relative path from project root
            let display_path =
                path.strip_prefix(&config.root).unwrap_or(&path).display().to_string();
            p.set_current_file(&display_path);
            Some(p)
        } else if !json_output {
            sh_println!("  Generated {} mutants, testing in parallel...", mutants.len())?;
            None
        } else {
            None
        };

        // Run mutations in parallel using isolated workspaces
        let results = run_mutations_parallel_with_progress(
            mutants.clone(),
            path.clone(),
            handler.src.clone(),
            config.clone(),
            evm_opts.clone(),
            env.clone(),
            num_workers,
            progress.clone(),
            json_output,
        )?;

        // Collect results for caching
        let mut results_vec = Vec::with_capacity(results.len());
        for result in results {
            results_vec.push((result.mutant.clone(), result.result.clone()));
            match result.result {
                MutationResult::Dead => handler.add_dead_mutant(result.mutant),
                MutationResult::Alive => {
                    handler.mark_span_survived(result.mutant.span);
                    handler.add_survived_mutant(result.mutant);
                }
                MutationResult::Invalid => handler.add_invalid_mutant(result.mutant),
                MutationResult::Skipped => handler.add_skipped_mutant(result.mutant),
            }
        }

        // Persist results for caching
        if !mutants.is_empty() && !build_id.is_empty() {
            let _ = handler.persist_cached_mutants(&build_id, &mutants);
            let _ = handler.persist_cached_results(&build_id, &results_vec);
            let _ = handler.persist_survived_spans(&build_id);
        }

        mutation_summary.merge(handler.get_report());

        // If cancelled, break out of the loop
        if let Some(ref p) = progress
            && p.is_cancelled()
        {
            cancelled = true;
            break;
        }
    }

    // Report results
    let duration = start_time.elapsed();
    let duration_secs = duration.as_secs_f64();

    // Only show human-readable report if not in JSON mode
    if !json_output {
        MutationReporter::new().report(&mutation_summary, duration);
    }

    Ok(MutationRunResult { summary: mutation_summary, cancelled, duration_secs })
}

/// Resolve which paths to mutate based on configuration.
fn resolve_mutate_paths(
    config: &Config,
    output: &ProjectCompileOutput<MultiCompiler>,
    mutation_config: &MutationRunConfig,
) -> Result<Vec<PathBuf>> {
    let paths = if let Some(pattern) = &mutation_config.mutate_path_pattern {
        // If --mutate-path is provided, use it to filter paths
        source_files_iter(&config.src, MultiCompilerLanguage::FILE_EXTENSIONS)
            .filter(|entry| entry.is_sol() && !entry.is_sol_test() && pattern.is_match(entry))
            .collect()
    } else if let Some(contract_pattern) = &mutation_config.mutate_contract_pattern {
        // If --mutate-contract is provided, use it to filter contracts
        source_files_iter(&config.src, MultiCompilerLanguage::FILE_EXTENSIONS)
            .filter(|entry| {
                entry.is_sol()
                    && !entry.is_sol_test()
                    && output
                        .artifact_ids()
                        .find(|(id, _)| id.source == *entry)
                        .is_some_and(|(id, _)| contract_pattern.is_match(&id.name))
            })
            .collect()
    } else if mutation_config.mutate_paths.is_empty() {
        // If --mutate is passed without arguments, use all Solidity files
        source_files_iter(&config.src, MultiCompilerLanguage::FILE_EXTENSIONS)
            .filter(|entry| entry.is_sol() && !entry.is_sol_test())
            .collect()
    } else {
        // If --mutate is passed with arguments, use those paths
        mutation_config.mutate_paths.clone()
    };

    Ok(paths)
}
