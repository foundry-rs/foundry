//! Parallel mutation testing runner.
//!
//! This module provides high-performance parallel execution of mutation tests.
//! Each mutant is tested in an isolated temporary workspace to enable concurrent execution.

use crate::{
    MultiContractRunnerBuilder,
    cmd::test::{FilterArgs, RerunFailure},
    mutation::{
        mutant::{Mutant, MutationResult},
        progress::MutationProgress,
    },
    result::SuiteResult,
    workspace,
};
use eyre::Result;
use foundry_common::{compile::ProjectCompiler, fs::canonicalize_path, sh_eprintln, sh_println};
use foundry_compilers::compilers::multi::MultiCompiler;
use foundry_config::{Config, InlineConfig};
use foundry_evm::{
    backend::Backend,
    core::evm::{EthEvmNetwork, FoundryEvmNetwork},
    executors::ExecutorBuilder,
    opts::EvmOpts,
};
use std::{
    collections::BTreeMap,
    fs,
    panic::{self, AssertUnwindSafe},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use tempfile::TempDir;

const MUTATION_STACK_SIZE: usize = 16 * 1024 * 1024;

#[cfg(test)]
const MUTATION_STACK_PROBE_ENV: &str = "FOUNDRY_MUTATION_STACK_PROBE";
#[cfg(test)]
const MUTATION_STACK_PROBE_MARKER_ENV: &str = "FOUNDRY_MUTATION_STACK_PROBE_MARKER";
#[cfg(test)]
static MUTATION_STACK_PROBE_RAN: AtomicBool = AtomicBool::new(false);

/// Result of testing a single mutant.
#[derive(Debug, Clone)]
pub struct MutantTestResult {
    pub mutant: Mutant,
    pub result: MutationResult,
}

/// Result of a parallel mutation batch.
#[derive(Debug, Clone)]
pub struct MutationBatchResult {
    pub results: Vec<MutantTestResult>,
    pub cancelled: bool,
}

/// Immutable EVM inputs shared by the baseline and every mutation worker.
#[derive(Clone)]
pub struct MutationEvmConfig<FEN: FoundryEvmNetwork = EthEvmNetwork> {
    pub opts: EvmOpts,
    pub backend: Backend<FEN>,
    pub executor_builder: ExecutorBuilder<FEN>,
    pub create2_deployer_available: bool,
}

/// Tracks progress, cancellation, and timed-out workers across parallel workers.
pub struct SharedMutationState {
    /// Progress counter.
    pub completed: AtomicUsize,
    pub total: AtomicUsize,
    /// Cancellation flag (Ctrl+C)
    pub cancelled: Arc<AtomicBool>,
    /// Optional progress display
    pub progress: Option<MutationProgress>,
    /// Whether to suppress all output (for JSON mode)
    pub silent: bool,
    /// Worker threads spawned for timed-out mutants. We keep these handles
    /// alive (and the `TempDir` they own) so that:
    ///   1. The `TempDir` is *not* dropped while the worker is still touching it.
    ///   2. We can join the threads at the end of the run and surface leaks.
    pub pending_workers: Mutex<Vec<JoinHandle<()>>>,
    /// Maximum number of timed-out worker handles to keep pending at once.
    /// Older handles are joined before parking more, bounding cleanup backlog.
    max_pending_workers: AtomicUsize,
}

impl SharedMutationState {
    pub const fn new(
        cancelled: Arc<AtomicBool>,
        silent: bool,
        progress: Option<MutationProgress>,
    ) -> Self {
        Self {
            completed: AtomicUsize::new(0),
            total: AtomicUsize::new(0),
            cancelled,
            progress,
            silent,
            pending_workers: Mutex::new(Vec::new()),
            max_pending_workers: AtomicUsize::new(usize::MAX),
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

    pub fn increment_completed(&self) -> usize {
        self.completed.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn set_max_pending_workers(&self, max: usize) {
        self.max_pending_workers.store(max.max(1), Ordering::SeqCst);
    }

    fn park_timed_out_worker(&self, handle: JoinHandle<()>) {
        let mut pending = match self.pending_workers.lock() {
            Ok(pending) => pending,
            Err(_) => {
                let _ = handle.join();
                return;
            }
        };

        let max_pending = self.max_pending_workers.load(Ordering::SeqCst).max(1);
        while pending.len() >= max_pending {
            let old_handle = pending.remove(0);
            drop(pending);
            let _ = old_handle.join();
            pending = match self.pending_workers.lock() {
                Ok(pending) => pending,
                Err(_) => {
                    let _ = handle.join();
                    return;
                }
            };
        }

        pending.push(handle);
    }
}

impl Default for SharedMutationState {
    fn default() -> Self {
        Self::new(Arc::new(AtomicBool::new(false)), false, None)
    }
}

/// Run mutation tests in parallel with optional progress display.
#[allow(clippy::too_many_arguments)]
pub fn run_mutations_parallel_with_progress<FEN: FoundryEvmNetwork>(
    mutants: Vec<Mutant>,
    source_path: PathBuf,
    original_source: Arc<String>,
    config: Arc<Config>,
    evm: MutationEvmConfig<FEN>,
    num_workers: usize,
    progress: Option<MutationProgress>,
    silent: bool,
    filter_args: FilterArgs,
    rerun_failures: Option<Vec<RerunFailure>>,
    selected_sources_relative: Arc<Vec<PathBuf>>,
    isolate: bool,
    cancellation_requested: Arc<AtomicBool>,
) -> Result<MutationBatchResult> {
    let total = mutants.len();
    if total == 0 {
        return Ok(MutationBatchResult { results: vec![], cancelled: false });
    }

    // Default to available parallelism if num_workers is 0
    let num_workers = if num_workers == 0 {
        std::thread::available_parallelism().map(|p| p.get()).unwrap_or(1)
    } else {
        num_workers
    };

    let shared_state = Arc::new(SharedMutationState::new(cancellation_requested, silent, progress));
    shared_state.total.store(total, Ordering::SeqCst);
    shared_state.set_max_pending_workers(num_workers);

    // Only print if no progress bar and not silent
    if shared_state.progress.is_none() && !shared_state.silent {
        let _ = sh_println!("Running {} mutants in parallel with {} workers", total, num_workers);
    }

    // Get relative path of source within project - MUST be relative for safety
    // Canonicalize paths to handle relative vs absolute path comparisons
    let source_abs =
        if source_path.is_absolute() { source_path } else { config.root.join(&source_path) };

    let root_abs = config.root.canonicalize().unwrap_or_else(|_| config.root.clone());
    let source_abs = source_abs.canonicalize().unwrap_or(source_abs);

    let source_relative = source_abs
        .strip_prefix(&root_abs)
        .map_err(|_| {
            eyre::eyre!(
                "Source path {} is not under project root {}",
                source_abs.display(),
                root_abs.display()
            )
        })?
        .to_path_buf();

    workspace::ensure_safe_relative_path(&source_relative, "source", &source_abs)?;

    // `ProjectPathsConfig` canonicalizes its root. Create mutant workspaces beneath the canonical
    // temp root as well so explicit compiler inputs, project-local remappings, and the project
    // root all use the same path spelling (notably `/private/var` rather than `/var` on macOS).
    let temp_root = std::env::temp_dir();
    let temp_root = canonicalize_path(&temp_root).map_err(|err| {
        eyre::eyre!("failed to canonicalize mutation temp root {}: {err}", temp_root.display())
    })?;

    // Configure rayon thread pool
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(num_workers)
        .stack_size(MUTATION_STACK_SIZE)
        .build()
        .map_err(|e| eyre::eyre!("Failed to create thread pool: {}", e))?;

    // Use a thread-safe collection to store results as they complete
    let completed_results: Arc<Mutex<Vec<MutantTestResult>>> =
        Arc::new(Mutex::new(Vec::with_capacity(total)));

    let filter_args = Arc::new(filter_args);
    let rerun_failures = Arc::new(rerun_failures);

    // Each worker pulls mutants from a shared queue. Do not use a parallel iterator here: a worker
    // that waits on another pool (for example solar's parser pool in `Session::enter`) steals
    // queued jobs from this pool. If those jobs are mutants, they nest on one stack, and each
    // waiting mutant keeps its compiler session and thread pool alive until the run ends.
    let queue = Mutex::new(mutants.into_iter());
    pool.broadcast(|_| {
        loop {
            if shared_state.is_cancelled() {
                break;
            }
            let Some(mutant) = queue.lock().ok().and_then(|mut queue| queue.next()) else {
                break;
            };

            // Wrap in catch_unwind to prevent one panic from aborting the entire run
            let mutant_clone = mutant.clone();
            let result = panic::catch_unwind(AssertUnwindSafe(|| {
                test_single_mutant_isolated(
                    mutant,
                    &source_relative,
                    &original_source,
                    &config,
                    &evm,
                    &shared_state,
                    &temp_root,
                    &filter_args,
                    &rerun_failures,
                    &selected_sources_relative,
                    isolate,
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
        }
    });

    // Extract results
    let results = Arc::try_unwrap(completed_results)
        .map(|m| m.into_inner().unwrap_or_default())
        .unwrap_or_default();

    // Drain and join any worker threads that were left running by a
    // wall-clock `TimedOut`. Each worker owns its own `TempDir`, so joining
    // here is what actually deletes the per-mutant workspace from disk. This
    // is the difference between a clean shutdown and stale `forge_mutation_*`
    // directories piling up under `$TMPDIR`.
    //
    // We intentionally block: by this point all rayon work is done, the
    // wall-clock budget has already been spent, and the only thing left to do
    // is reclaim cleanup. Each worker stops its test run at the same deadline
    // (see `compile_and_test`), which bounds how long any individual worker can
    // actually run.
    let pending = shared_state
        .pending_workers
        .lock()
        .map(|mut g| std::mem::take(&mut *g))
        .unwrap_or_default();
    let pending_count = pending.len();
    if pending_count > 0 && !shared_state.silent && shared_state.progress.is_none() {
        let _ = sh_println!("Waiting for {pending_count} timed-out worker(s) to finish cleanup...");
    }
    for handle in pending {
        let _ = handle.join();
    }

    let cancelled = shared_state.is_cancelled();

    // Clear progress and handle cancellation
    if let Some(ref progress) = shared_state.progress {
        progress.clear();
    }
    if cancelled && !shared_state.silent {
        let _ = sh_println!(
            "\nMutation testing cancelled. Showing results for {} completed mutants.\n",
            results.len()
        );
    }

    Ok(MutationBatchResult { results, cancelled })
}

/// Test a single mutant in an isolated temporary workspace.
#[allow(clippy::too_many_arguments)]
fn test_single_mutant_isolated<FEN: FoundryEvmNetwork>(
    mutant: Mutant,
    source_relative: &PathBuf,
    original_source: &Arc<String>,
    config: &Arc<Config>,
    evm: &MutationEvmConfig<FEN>,
    shared_state: &Arc<SharedMutationState>,
    temp_root: &Path,
    filter_args: &Arc<FilterArgs>,
    rerun_failures: &Arc<Option<Vec<RerunFailure>>>,
    selected_sources_relative: &Arc<Vec<PathBuf>>,
    isolate: bool,
) -> MutantTestResult {
    // Show progress or log
    if let Some(ref progress) = shared_state.progress {
        progress.start_mutant(&mutant);
    } else if !shared_state.silent {
        let completed = shared_state.increment_completed();
        let total = shared_state.total.load(Ordering::SeqCst);
        let _ = sh_println!("[{}/{}] Testing mutant: {}", completed, total, mutant);
    }

    // Create isolated workspace using TempDir for automatic cleanup on drop
    let temp_dir = match TempDir::with_prefix_in("forge_mutation_", temp_root) {
        Ok(dir) => dir,
        Err(e) => {
            let _ =
                sh_eprintln!("Failed to create temp directory in {}: {}", temp_root.display(), e);
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

    // Start from the already materialized baseline config instead of reloading
    // `foundry.toml`, so CLI overrides and runtime normalization stay identical
    // between the baseline run and every mutant run.
    let temp_config = Arc::new(workspace::rebase_config_paths(config, temp_dir.path()));

    // Compile and test, optionally bounded by a wall-clock timeout.
    //
    // Lifetime contract: `temp_dir` (the `TempDir`) must live *at least* as
    // long as the worker thread that reads from `temp_path`. Dropping the
    // `TempDir` early would delete the workspace while a worker still touches
    // it, which is a real correctness bug (random compile/test failures and
    // dangling fs handles on Windows).
    //
    // To satisfy that contract we move `temp_dir` ownership into the worker
    // thread. If the wall-clock budget fires the outer call returns
    // `TimedOut`, but the `TempDir` only drops when the worker thread itself
    // exits. The `JoinHandle` is stored in `shared_state.pending_workers` and
    // joined at the end of the parallel run.
    let timeout = config.mutation.timeout.map(|s| Duration::from_secs(s as u64));

    let result = match timeout {
        Some(budget) => run_compile_and_test_with_timeout(
            temp_config,
            evm,
            budget,
            temp_dir,
            shared_state,
            filter_args.clone(),
            rerun_failures.clone(),
            selected_sources_relative.clone(),
            isolate,
        ),
        None => {
            let res = match compile_and_test(
                &temp_config,
                evm,
                filter_args,
                rerun_failures.as_ref().as_deref(),
                selected_sources_relative,
                isolate,
                None,
            ) {
                Ok(true) => MutationResult::Dead,
                Ok(false) => MutationResult::Alive,
                Err(_) => MutationResult::Invalid,
            };
            drop(temp_dir); // explicit: workspace is only safe to remove now
            res
        }
    };

    // Update progress
    if let Some(ref progress) = shared_state.progress {
        progress.complete_mutant(&mutant, &result);
    }

    MutantTestResult { mutant, result }
}

/// Run `compile_and_test` on a worker thread and wait at most `budget` for it
/// to complete. Returns `TimedOut` on overrun and `Invalid` on infrastructure
/// errors / panics.
///
/// The worker stops its test run at the same deadline, so a timed-out worker
/// does not keep running fuzz or invariant campaigns in the background.
///
/// The worker takes ownership of `temp_dir` so the underlying workspace
/// directory is only dropped when the worker thread actually exits. On
/// timeout the `JoinHandle` is parked in `shared_state.pending_workers`
/// and joined at the end of the parallel run.
#[allow(clippy::too_many_arguments)]
fn run_compile_and_test_with_timeout<FEN: FoundryEvmNetwork>(
    config: Arc<Config>,
    evm: &MutationEvmConfig<FEN>,
    budget: Duration,
    temp_dir: TempDir,
    shared_state: &Arc<SharedMutationState>,
    filter_args: Arc<FilterArgs>,
    rerun_failures: Arc<Option<Vec<RerunFailure>>>,
    selected_sources_relative: Arc<Vec<PathBuf>>,
    isolate: bool,
) -> MutationResult {
    let deadline = Instant::now() + budget;
    let (tx, rx) = mpsc::channel::<Result<bool>>();
    let evm = evm.clone();
    // Move `temp_dir` into the worker so its `Drop` only runs after the worker
    // thread exits. Do NOT capture by reference — the worker may outlive this
    // function on timeout.
    let cfg = Arc::clone(&config);
    let filter_for_worker = Arc::clone(&filter_args);
    let rerun_for_worker = Arc::clone(&rerun_failures);
    let selected_sources_for_worker = Arc::clone(&selected_sources_relative);

    let spawn_result = std::thread::Builder::new()
        .stack_size(MUTATION_STACK_SIZE)
        .name("mutation-worker".to_string())
        .spawn(move || {
            // `test_collect` uses Rayon internally. Because this timeout worker
            // is not itself a Rayon worker, nested parallel iterators would
            // otherwise escape to the global pool and its default-sized stacks.
            let res = panic::catch_unwind(AssertUnwindSafe(|| {
                with_mutation_test_pool(|| {
                    compile_and_test(
                        &cfg,
                        &evm,
                        &filter_for_worker,
                        rerun_for_worker.as_ref().as_deref(),
                        &selected_sources_for_worker,
                        isolate,
                        Some(deadline),
                    )
                })
            }))
            .unwrap_or_else(|_| Err(eyre::eyre!("worker panicked")));
            let _ = tx.send(res);
            // Keep `temp_dir` alive until *after* the worker is done with the
            // workspace. Dropping here (vs at function entry on timeout)
            // guarantees no use-after-free of the filesystem.
            drop(temp_dir);
        });

    let handle = match spawn_result {
        Ok(h) => h,
        Err(_) => return MutationResult::Invalid,
    };

    match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        // The worker stops its tests at the deadline, so a result that arrives
        // after it can come from an incomplete test run.
        Ok(_) if Instant::now() >= deadline => {
            let _ = handle.join();
            MutationResult::TimedOut
        }
        Ok(Ok(true)) => {
            // Worker finished and sent a result; join briefly so the TempDir
            // is actually cleaned up before we return.
            let _ = handle.join();
            MutationResult::Dead
        }
        Ok(Ok(false)) => {
            let _ = handle.join();
            MutationResult::Alive
        }
        Ok(Err(_)) => {
            let _ = handle.join();
            MutationResult::Invalid
        }
        Err(_) => {
            // Timeout fired. The worker is still running and still owns the
            // TempDir; park the handle so we can join (and reclaim cleanup)
            // at the end of the parallel run instead of leaking it.
            shared_state.park_timed_out_worker(handle);
            MutationResult::TimedOut
        }
    }
}

fn with_mutation_test_pool<T: Send>(op: impl FnOnce() -> Result<T> + Send) -> Result<T> {
    rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .stack_size(MUTATION_STACK_SIZE)
        .thread_name(|_| "mutation-test".to_string())
        .build()
        .map_err(|err| eyre::eyre!("failed to create mutation test pool: {err}"))?
        .install(|| {
            #[cfg(test)]
            if std::env::var_os(MUTATION_STACK_PROBE_ENV).is_some() {
                std::hint::black_box(mutation_stack_probe(1024));
                MUTATION_STACK_PROBE_RAN.store(true, Ordering::Release);
            }
            op()
        })
}

#[cfg(test)]
#[inline(never)]
fn mutation_stack_probe(depth: usize) -> usize {
    let frame = [depth as u8; 8 * 1024];
    std::hint::black_box(&frame);
    if depth == 0 {
        frame[0] as usize
    } else {
        let result = mutation_stack_probe(depth - 1).wrapping_add(frame[depth % frame.len()] as _);
        std::hint::black_box(&frame);
        result
    }
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

/// Compiles and tests a mutant using the campaign's already selected EVM and remote backend.
///
/// If `deadline` is set, the test run stops at the deadline like an interrupted `forge test`.
/// This keeps fuzz and invariant run limits unchanged, because their `timeout` settings would
/// turn a run-limited campaign into a time-based one.
fn compile_and_test<FEN: FoundryEvmNetwork>(
    config: &Arc<Config>,
    evm: &MutationEvmConfig<FEN>,
    filter_args: &FilterArgs,
    rerun_failures: Option<&[RerunFailure]>,
    selected_sources_relative: &[PathBuf],
    isolate: bool,
    deadline: Option<Instant>,
) -> Result<bool> {
    let evm_opts = &evm.opts;
    // Compile
    let files = selected_sources_relative
        .iter()
        .map(|path| config.root.join(path))
        .filter(|path| path.exists())
        .collect::<Vec<_>>();
    let compiler = ProjectCompiler::new()
        .dynamic_test_linking(config.dynamic_test_linking)
        .quiet(true)
        .files(files);

    let compile_output = compiler.compile(&config.project()?)?;
    let inline_config = Arc::new(InlineConfig::new_parsed(&compile_output, config)?);

    // Rebuild the per-mutant test filter so `--match-test`, `--match-contract`,
    // `--match-path`, ... are honored against the temp workspace's paths
    // (not the original project root). Without this the mutant runs would
    // ignore user filters and execute a different test set than the baseline.
    let mut filter = filter_args.clone().merge_with_config(config);
    if let Some(rerun_failures) = rerun_failures {
        filter.set_rerun_failures(rerun_failures.to_vec());
    }

    // Run tests - need a multi-threaded Tokio runtime since test() uses rayon internally
    // with par_iter, and rayon workers need tokio handle access
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1) // Minimize overhead, tests use rayon for parallelism
        .enable_all()
        .build()
        .map_err(|e| eyre::eyre!("Failed to create tokio runtime: {}", e))?;

    // Use block_on to run within the runtime context
    let results: BTreeMap<String, SuiteResult> = rt.block_on(async {
        let (evm_env, tx_env) = evm.backend.env(evm_opts).await?;

        // Build test runner mirroring the canonical `forge test` runner: same
        // isolation flag, same fail-fast semantics for mutation, and same
        // filter so kept/skipped tests stay consistent across baseline and
        // mutant runs.
        let mut runner = MultiContractRunnerBuilder::new(config.clone(), inline_config)
            .set_debug(false)
            .initial_balance(evm_opts.initial_balance)
            .sender(evm_opts.sender)
            .enable_isolation(isolate)
            .fail_fast(true)
            .with_create2_deployer_available(evm.create2_deployer_available)
            .build::<FEN, MultiCompiler>(
                &compile_output,
                evm_env,
                tx_env,
                evm_opts.clone(),
                evm.backend.clone_with_fork_scope()?,
                evm.executor_builder.clone(),
            )?;

        if let Some(deadline) = deadline {
            let early_exit = runner.tcfg.early_exit.clone();
            tokio::spawn(async move {
                tokio::time::sleep_until(deadline.into()).await;
                early_exit.record_ctrl_c();
            });
        }

        runner.test_collect(&filter)
    })?;

    // Check if any test failed (mutant killed)
    let killed = results.values().any(|suite| suite.failed() > 0);

    Ok(killed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::U256;
    use std::process::Command;

    #[test]
    fn timeout_path_uses_enlarged_test_worker_stack() {
        if std::env::var_os(MUTATION_STACK_PROBE_ENV).is_some() {
            let workspace = TempDir::new().unwrap();
            let source = PathBuf::from("src/StackProbe.t.sol");
            fs::create_dir_all(workspace.path().join("src")).unwrap();
            fs::write(
                workspace.path().join(&source),
                "contract StackProbeTest { function test_stackProbe() public {} }",
            )
            .unwrap();
            let mut config = Config::with_root(workspace.path());
            config.out = workspace.path().join("out");
            config.cache_path = workspace.path().join("cache/solidity-files-cache.json");
            let config = Arc::new(config);
            let evm = MutationEvmConfig {
                opts: EvmOpts::default(),
                backend: Backend::<EthEvmNetwork>::spawn(None).unwrap(),
                executor_builder: ExecutorBuilder::<EthEvmNetwork>::new(),
                create2_deployer_available: false,
            };
            let shared_state = Arc::new(SharedMutationState::default());

            let result = run_compile_and_test_with_timeout(
                config,
                &evm,
                Duration::from_secs(60),
                workspace,
                &shared_state,
                Arc::new(FilterArgs::default()),
                Arc::new(None),
                Arc::new(vec![source]),
                false,
            );

            assert!(!matches!(result, MutationResult::TimedOut), "unexpected result: {result:?}");
            assert!(MUTATION_STACK_PROBE_RAN.load(Ordering::Acquire));
            fs::write(std::env::var_os(MUTATION_STACK_PROBE_MARKER_ENV).unwrap(), b"ok").unwrap();
            return;
        }

        let marker_dir = TempDir::new().unwrap();
        let marker = marker_dir.path().join("completed");
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "mutation::runner::tests::timeout_path_uses_enlarged_test_worker_stack",
            ])
            .env(MUTATION_STACK_PROBE_ENV, "1")
            .env(MUTATION_STACK_PROBE_MARKER_ENV, &marker)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert!(
            output.status.success(),
            "stack-pressure child failed:\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
        assert!(marker.exists(), "stack-pressure child did not complete the production path");
    }

    #[test]
    fn park_timed_out_worker_bounds_pending_handles() {
        let state = SharedMutationState::default();
        state.set_max_pending_workers(1);

        state.park_timed_out_worker(std::thread::spawn(|| {}));
        assert_eq!(state.pending_workers.lock().unwrap().len(), 1);

        state.park_timed_out_worker(std::thread::spawn(|| {}));
        assert_eq!(state.pending_workers.lock().unwrap().len(), 1);

        let pending = std::mem::take(&mut *state.pending_workers.lock().unwrap());
        for handle in pending {
            handle.join().unwrap();
        }
    }

    #[test]
    fn temp_config_preserves_materialized_overrides_and_rebases_paths() {
        let project = TempDir::new().unwrap();
        let temp = TempDir::new().unwrap();
        let root = project.path();

        let mut config = Config {
            root: root.to_path_buf(),
            src: root.join("contracts"),
            test: root.join("checks"),
            script: root.join("deploy"),
            out: root.join("custom-out"),
            cache_path: root.join("custom-cache"),
            snapshots: root.join("custom-snapshots"),
            broadcast: root.join("custom-broadcast"),
            mutation_dir: root.join("custom-cache/mutation"),
            libs: vec![root.join("vendor")],
            include_paths: vec![root.join("shared")],
            allow_paths: vec![root.join("fixtures")],
            dynamic_test_linking: true,
            cache: true,
            ..Default::default()
        };
        config.fuzz.seed = Some(U256::from(42));
        config.fuzz.timeout = Some(90);
        config.invariant.timeout = Some(80);
        config.fuzz.failure_persist_dir = Some(root.join("custom-cache/fuzz"));
        config.invariant.failure_persist_dir = Some(root.join("custom-cache/invariant"));
        config.mutation.timeout = Some(5);

        let temp_config = workspace::rebase_config_paths(&config, temp.path());
        let expected_root = canonicalize_path(temp.path()).unwrap();

        assert_eq!(temp_config.root, expected_root);
        assert_eq!(temp_config.src, expected_root.join("contracts"));
        assert_eq!(temp_config.test, expected_root.join("checks"));
        assert_eq!(temp_config.script, expected_root.join("deploy"));
        assert_eq!(temp_config.out, expected_root.join("custom-out"));
        assert_eq!(temp_config.cache_path, expected_root.join("custom-cache"));
        assert_eq!(temp_config.snapshots, expected_root.join("custom-snapshots"));
        assert_eq!(temp_config.broadcast, expected_root.join("custom-broadcast"));
        assert_eq!(temp_config.mutation_dir, expected_root.join("custom-cache/mutation"));
        assert_eq!(temp_config.libs, vec![expected_root.join("vendor")]);
        assert_eq!(temp_config.include_paths, vec![expected_root.join("shared")]);
        assert_eq!(temp_config.allow_paths, vec![expected_root.join("fixtures")]);
        assert_eq!(
            temp_config.fuzz.failure_persist_dir,
            Some(expected_root.join("custom-cache/fuzz"))
        );
        assert_eq!(
            temp_config.invariant.failure_persist_dir,
            Some(expected_root.join("custom-cache/invariant"))
        );
        assert!(temp_config.dynamic_test_linking);
        assert!(temp_config.cache);
        assert_eq!(temp_config.fuzz.seed, Some(U256::from(42)));
        assert_eq!(temp_config.fuzz.timeout, Some(90));
        assert_eq!(temp_config.invariant.timeout, Some(80));
    }
}
