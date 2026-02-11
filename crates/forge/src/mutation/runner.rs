//! Parallel mutation testing runner.
//!
//! This module provides high-performance parallel execution of mutation tests.
//! Each mutant is tested in an isolated temporary workspace to enable concurrent execution.

use std::{
    collections::BTreeMap,
    fs,
    panic::{self, AssertUnwindSafe},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use eyre::Result;
use foundry_common::{EmptyTestFilter, compile::ProjectCompiler, sh_eprintln, sh_println};
use foundry_compilers::compilers::multi::MultiCompiler;
use foundry_config::Config;
use foundry_evm::{Env, opts::EvmOpts};
use rayon::prelude::*;
use tempfile::TempDir;

use crate::{
    MultiContractRunnerBuilder,
    mutation::{
        SurvivedSpans,
        mutant::{Mutant, MutationResult},
        progress::MutationProgress,
    },
    result::SuiteResult,
    workspace,
};

/// Result of testing a single mutant.
#[derive(Debug, Clone)]
pub struct MutantTestResult {
    pub mutant: Mutant,
    pub result: MutationResult,
}

/// Tracks progress and adaptive span skipping across parallel workers.
#[derive(Default)]
pub struct SharedMutationState {
    /// Spans where mutations have survived - shared across workers for adaptive skipping.
    pub survived_spans: Mutex<SurvivedSpans>,
    /// Progress counter.
    pub completed: AtomicUsize,
    pub total: AtomicUsize,
    /// Cancellation flag (Ctrl+C)
    pub cancelled: AtomicBool,
    /// Optional progress display
    pub progress: Option<MutationProgress>,
    /// Whether to suppress all output (for JSON mode)
    pub silent: bool,
}

impl SharedMutationState {
    pub fn new() -> Self {
        Self {
            survived_spans: Mutex::new(SurvivedSpans::new()),
            completed: AtomicUsize::new(0),
            total: AtomicUsize::new(0),
            cancelled: AtomicBool::new(false),
            progress: None,
            silent: false,
        }
    }

    pub fn new_silent() -> Self {
        Self {
            survived_spans: Mutex::new(SurvivedSpans::new()),
            completed: AtomicUsize::new(0),
            total: AtomicUsize::new(0),
            cancelled: AtomicBool::new(false),
            progress: None,
            silent: true,
        }
    }

    pub fn with_progress(progress: MutationProgress) -> Self {
        Self {
            survived_spans: Mutex::new(SurvivedSpans::new()),
            completed: AtomicUsize::new(0),
            total: AtomicUsize::new(0),
            cancelled: AtomicBool::new(false),
            progress: Some(progress),
            silent: false,
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        if let Some(ref progress) = self.progress {
            progress.cancel();
        }
    }

    pub fn should_skip_span(&self, span: solar::ast::Span) -> bool {
        // Handle mutex poisoning gracefully - don't skip if we can't check
        self.survived_spans.lock().map(|guard| guard.should_skip(span)).unwrap_or(false)
    }

    pub fn mark_span_survived(&self, span: solar::ast::Span) {
        // Handle mutex poisoning gracefully - just skip marking if poisoned
        if let Ok(mut guard) = self.survived_spans.lock() {
            guard.mark_survived(span);
        }
    }

    pub fn increment_completed(&self) -> usize {
        self.completed.fetch_add(1, Ordering::SeqCst) + 1
    }
}

/// Run mutation tests in parallel with optional progress display.
#[allow(clippy::too_many_arguments)]
pub fn run_mutations_parallel_with_progress(
    mutants: Vec<Mutant>,
    source_path: PathBuf,
    original_source: Arc<String>,
    config: Arc<Config>,
    evm_opts: EvmOpts,
    env: Env,
    num_workers: usize,
    progress: Option<MutationProgress>,
    silent: bool,
) -> Result<Vec<MutantTestResult>> {
    let total = mutants.len();
    if total == 0 {
        return Ok(vec![]);
    }

    let shared_state = Arc::new(if let Some(p) = progress {
        SharedMutationState::with_progress(p)
    } else if silent {
        SharedMutationState::new_silent()
    } else {
        SharedMutationState::new()
    });
    shared_state.total.store(total, Ordering::SeqCst);

    // Only print if no progress bar and not silent
    if shared_state.progress.is_none() && !shared_state.silent {
        let _ = sh_println!("Running {} mutants in parallel with {} workers", total, num_workers);
    }

    // Get relative path of source within project - MUST be relative for safety
    // Canonicalize paths to handle relative vs absolute path comparisons
    let source_abs =
        if source_path.is_absolute() { source_path } else { config.root.join(&source_path) };

    let source_relative = source_abs
        .strip_prefix(&config.root)
        .map_err(|_| {
            eyre::eyre!(
                "Source path {} is not under project root {}",
                source_abs.display(),
                config.root.display()
            )
        })?
        .to_path_buf();

    workspace::ensure_safe_relative_path(&source_relative, "source", &source_abs)?;

    // Default to available parallelism if num_workers is 0
    let num_workers = if num_workers == 0 {
        std::thread::available_parallelism().map(|p| p.get()).unwrap_or(1)
    } else {
        num_workers
    };

    // Set up Ctrl+C handler using a background thread with tokio signal
    // This replaces ctrlc crate with tokio's built-in signal handling
    let ctrlc_handle = if shared_state.progress.is_some() {
        let state_for_ctrlc = Arc::clone(&shared_state);
        Some(std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("Failed to create tokio runtime for signal handler");
            rt.block_on(async {
                if tokio::signal::ctrl_c().await.is_ok() {
                    state_for_ctrlc.cancel();
                }
            });
        }))
    } else {
        None
    };

    // Configure rayon thread pool
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(num_workers)
        .stack_size(16 * 1024 * 1024) // 16MB stack to avoid overflow in deep call chains
        .build()
        .map_err(|e| eyre::eyre!("Failed to create thread pool: {}", e))?;

    // Use a thread-safe collection to store results as they complete
    let completed_results: Arc<Mutex<Vec<MutantTestResult>>> =
        Arc::new(Mutex::new(Vec::with_capacity(total)));

    pool.install(|| {
        mutants.into_par_iter().for_each(|mutant| {
            // Skip if cancelled
            if shared_state.is_cancelled() {
                return;
            }

            // Wrap in catch_unwind to prevent one panic from aborting the entire run
            let mutant_clone = mutant.clone();
            let result = panic::catch_unwind(AssertUnwindSafe(|| {
                test_single_mutant_isolated(
                    mutant,
                    &source_relative,
                    &original_source,
                    &config,
                    &evm_opts,
                    &env,
                    &shared_state,
                )
            }));

            let test_result = match result {
                Ok(r) => r,
                Err(_) => {
                    if shared_state.progress.is_none() {
                        let _ = sh_eprintln!("Panic while testing mutant: {}", mutant_clone);
                    }
                    MutantTestResult { mutant: mutant_clone, result: MutationResult::Invalid }
                }
            };

            // Store result immediately
            if let Ok(mut results) = completed_results.lock() {
                results.push(test_result);
            }
        });
    });

    // Extract results
    let results = Arc::try_unwrap(completed_results)
        .map(|m| m.into_inner().unwrap_or_default())
        .unwrap_or_default();

    // Clear progress and handle cancellation
    if let Some(ref progress) = shared_state.progress {
        progress.clear();
        if shared_state.is_cancelled() && !shared_state.silent {
            let _ = sh_println!(
                "\nMutation testing cancelled. Showing results for {} completed mutants.\n",
                results.len()
            );
        }
    }

    // The signal handler thread will exit when the program exits,
    // no need to join it since it's waiting on a signal that won't come after cancellation
    drop(ctrlc_handle);

    Ok(results)
}

/// Test a single mutant in an isolated temporary workspace.
fn test_single_mutant_isolated(
    mutant: Mutant,
    source_relative: &PathBuf,
    original_source: &Arc<String>,
    config: &Arc<Config>,
    evm_opts: &EvmOpts,
    env: &Env,
    shared_state: &Arc<SharedMutationState>,
) -> MutantTestResult {
    // Check if we should skip this mutant based on adaptive span tracking
    if shared_state.should_skip_span(mutant.span) {
        if let Some(ref progress) = shared_state.progress {
            progress.complete_mutant(&mutant, &MutationResult::Skipped);
        } else if !shared_state.silent {
            let completed = shared_state.increment_completed();
            let total = shared_state.total.load(Ordering::SeqCst);
            let _ = sh_println!(
                "[{}/{}] Skipping mutant (adaptive: span already has surviving mutation)",
                completed,
                total
            );
        }
        return MutantTestResult { mutant, result: MutationResult::Skipped };
    }

    // Show progress or log
    if let Some(ref progress) = shared_state.progress {
        progress.start_mutant(&mutant);
    } else if !shared_state.silent {
        let completed = shared_state.increment_completed();
        let total = shared_state.total.load(Ordering::SeqCst);
        let _ = sh_println!("[{}/{}] Testing mutant: {}", completed, total, mutant);
    }

    // Create isolated workspace using TempDir for automatic cleanup on drop
    let temp_dir = match TempDir::with_prefix("forge_mutation_") {
        Ok(dir) => dir,
        Err(e) => {
            let _ = sh_eprintln!("Failed to create temp directory: {}", e);
            return MutantTestResult { mutant, result: MutationResult::Invalid };
        }
    };

    // Copy project to temp directory
    if let Err(e) = workspace::copy_project(config, temp_dir.path()) {
        let _ = sh_eprintln!("Failed to copy project: {}", e);
        return MutantTestResult { mutant, result: MutationResult::Invalid };
    }

    // Apply mutation - source_relative is guaranteed to be relative at this point
    let mutated_source_path = temp_dir.path().join(source_relative);
    if let Err(e) = apply_mutation(&mutant, original_source, &mutated_source_path) {
        let _ = sh_eprintln!("Failed to apply mutation: {}", e);
        return MutantTestResult { mutant, result: MutationResult::Invalid };
    }

    // Create a config for the temp directory, preserving relative paths
    let temp_path = temp_dir.path().to_path_buf();
    let src_rel = workspace::relative_to_root(&config.root, &config.src);
    let test_rel = workspace::relative_to_root(&config.root, &config.test);

    let mut temp_config = Config::load_with_root(&temp_path).unwrap_or_else(|_| {
        let mut c = Config::clone(config.as_ref());
        c.root = temp_path.clone();
        c.src = temp_path.join(&src_rel);
        c.test = temp_path.join(&test_rel);
        c.out = temp_path.join("out");
        c.cache_path = temp_path.join("cache");
        c
    });
    temp_config.root = temp_path.clone();
    temp_config.src = temp_path.join(&src_rel);
    temp_config.test = temp_path.join(&test_rel);
    temp_config.out = temp_path.join("out");
    temp_config.cache_path = temp_path.join("cache");

    // Update libs paths to point to temp directory
    temp_config.libs = config
        .libs
        .iter()
        .map(|lib| {
            let lib_rel = workspace::relative_to_root(&config.root, lib);
            temp_path.join(lib_rel)
        })
        .collect();

    let temp_config = Arc::new(temp_config);

    // Compile and test
    let result = match compile_and_test(&temp_config, evm_opts, env) {
        Ok(killed) => {
            if killed {
                MutationResult::Dead
            } else {
                // Mark span as survived for adaptive skipping
                shared_state.mark_span_survived(mutant.span);
                MutationResult::Alive
            }
        }
        Err(_) => MutationResult::Invalid,
    };

    // Update progress
    if let Some(ref progress) = shared_state.progress {
        progress.complete_mutant(&mutant, &result);
    }

    // TempDir automatically cleans up on drop
    MutantTestResult { mutant, result }
}

/// Apply a mutation to a source file.
fn apply_mutation(mutant: &Mutant, original_source: &str, dest_path: &Path) -> Result<()> {
    let span = mutant.span;
    let replacement = mutant.mutation.to_string();
    let start_pos = span.lo().0 as usize;
    let end_pos = span.hi().0 as usize;

    // Use checked slicing to avoid panics on invalid spans or non-UTF8 boundaries
    let before = original_source.get(..start_pos).ok_or_else(|| {
        eyre::eyre!(
            "Invalid mutation span: start {} is out of bounds for source length {}",
            start_pos,
            original_source.len()
        )
    })?;

    let after = original_source.get(end_pos..).ok_or_else(|| {
        eyre::eyre!(
            "Invalid mutation span: end {} is out of bounds for source length {}",
            end_pos,
            original_source.len()
        )
    })?;

    let mut new_content = String::with_capacity(before.len() + replacement.len() + after.len());
    new_content.push_str(before);
    new_content.push_str(&replacement);
    new_content.push_str(after);

    // Ensure parent directory exists
    if let Some(parent) = dest_path.parent() {
        fs::create_dir_all(parent)?;
    }

    fs::write(dest_path, new_content)?;
    Ok(())
}

/// Compile the project and run tests, returning true if any test failed (mutant killed).
fn compile_and_test(config: &Arc<Config>, evm_opts: &EvmOpts, env: &Env) -> Result<bool> {
    // Compile
    let compiler =
        ProjectCompiler::new().dynamic_test_linking(config.dynamic_test_linking).quiet(true);

    let compile_output = compiler.compile(&config.project()?)?;

    // Build test runner with fail-fast enabled
    let mut runner = MultiContractRunnerBuilder::new(config.clone())
        .set_debug(false)
        .initial_balance(evm_opts.initial_balance)
        .evm_spec(config.evm_spec_id())
        .sender(evm_opts.sender)
        .with_fork(evm_opts.clone().get_fork(config, env.clone()))
        .fail_fast(true)
        .build::<MultiCompiler>(&compile_output, env.clone(), evm_opts.clone())?;

    // Run tests - need a multi-threaded Tokio runtime since test() uses rayon internally
    // with par_iter, and rayon workers need tokio handle access
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1) // Minimize overhead, tests use rayon for parallelism
        .enable_all()
        .build()
        .map_err(|e| eyre::eyre!("Failed to create tokio runtime: {}", e))?;

    // Use block_on to run within the runtime context
    let results: BTreeMap<String, SuiteResult> =
        rt.block_on(async { runner.test_collect(&EmptyTestFilter::default()) })?;

    // Check if any test failed (mutant killed)
    let killed = results.values().any(|suite| suite.failed() > 0);

    Ok(killed)
}
