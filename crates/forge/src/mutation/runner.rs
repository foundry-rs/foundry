//! Parallel mutation testing runner.
//!
//! This module provides high-performance parallel execution of mutation tests.
//! Each mutant is tested in an isolated temporary workspace to enable concurrent execution.

use std::{
    collections::BTreeMap,
    fs,
    panic::{self, AssertUnwindSafe},
    path::{Component, Path, PathBuf},
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
};

/// Check if a path is safe for use as a relative path within a workspace.
/// Rejects absolute paths, parent directory components (..), and other unsafe patterns.
fn is_safe_relative_path(p: &Path) -> bool {
    !p.is_absolute()
        && p.components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

/// Validates that `rel` is a safe relative path. Returns an error mentioning `label` and `orig`
/// if the path contains `..`, is absolute, or otherwise escapes the project root.
fn ensure_safe_relative_path(rel: &Path, label: &str, orig: &Path) -> Result<()> {
    if !is_safe_relative_path(rel) {
        eyre::bail!(
            "mutation testing requires {label} directory under project root, got: {}",
            orig.display()
        );
    }
    Ok(())
}

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

/// Compute relative path of `path` under `root`, or return the path unchanged if not under root.
fn relative_to_root(root: &Path, path: &Path) -> PathBuf {
    path.strip_prefix(root).map(|p| p.to_path_buf()).unwrap_or_else(|_| path.to_path_buf())
}

/// Copy only essential project files for mutation testing.
/// Uses symlinks for lib directories (read-only dependencies) to avoid expensive copies.
/// Preserves the project's directory layout (handles custom src/test paths).
fn copy_project_for_mutation(config: &Config, temp_dir: &Path) -> Result<()> {
    let src_rel = relative_to_root(&config.root, &config.src);
    ensure_safe_relative_path(&src_rel, "src", &config.src)?;

    let test_rel = relative_to_root(&config.root, &config.test);
    ensure_safe_relative_path(&test_rel, "test", &config.test)?;

    // Copy src directory (will be mutated)
    copy_dir_recursive(&config.src, &temp_dir.join(&src_rel))?;

    // Copy test directory (needs to be in temp for compilation)
    // Only copy if different from src to avoid double-copying
    if config.test != config.src {
        copy_dir_recursive(&config.test, &temp_dir.join(&test_rel))?;
    }

    // Symlink all library directories (read-only, no need to copy)
    // Preserve relative paths to maintain remapping compatibility
    for lib_path in &config.libs {
        if lib_path.exists() {
            let lib_rel = relative_to_root(&config.root, lib_path);
            ensure_safe_relative_path(&lib_rel, "lib", lib_path)?;
            let target = temp_dir.join(&lib_rel);

            if !target.exists() {
                // Ensure parent directories exist
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)?;
                }
                // Try symlink first, fall back to copy on failure (Windows without privileges)
                if symlink_dir(lib_path, &target).is_err() {
                    copy_dir_recursive(lib_path, &target)?;
                }
            }

            // Recursively symlink nested lib directories within each library.
            // This handles projects with nested submodules (e.g., lib/euler-earn/lib/*)
            // that have their own dependencies with context-specific remappings.
            symlink_nested_libs(lib_path, &target)?;
        }
    }

    // Symlink common external dependency directories (npm, soldeer)
    // These are not always included in config.libs but are required for compilation
    for dep_dir in ["node_modules", "dependencies"] {
        let dep_path = config.root.join(dep_dir);
        if dep_path.exists() && dep_path.is_dir() {
            let target = temp_dir.join(dep_dir);
            if !target.exists() {
                // Try symlink first, fall back to copy on failure (Windows without privileges)
                if symlink_dir(&dep_path, &target).is_err() {
                    copy_dir_recursive(&dep_path, &target)?;
                }
            }
        }
    }

    // Copy foundry.toml if exists
    let foundry_toml = config.root.join("foundry.toml");
    if foundry_toml.exists() {
        fs::copy(&foundry_toml, temp_dir.join("foundry.toml"))?;
    }

    // Copy remappings.txt if exists
    let remappings = config.root.join("remappings.txt");
    if remappings.exists() {
        fs::copy(&remappings, temp_dir.join("remappings.txt"))?;
    }

    Ok(())
}

/// Create a symlink to a directory (cross-platform).
/// Returns Err if symlink creation fails (e.g., on Windows without privileges).
fn symlink_dir(src: &Path, dst: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(src, dst)?;
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_dir(src, dst)?;
    }
    Ok(())
}

/// Recursively symlink nested lib directories within a library.
///
/// Many projects use git submodules that themselves have dependencies in their own
/// `lib/` directories. When the top-level lib is symlinked, these nested libs are
/// included via the symlink. However, if remappings reference these nested paths
/// with context-specific prefixes (e.g., `lib/euler-earn:@openzeppelin=lib/euler-earn/lib/...`),
/// the mutation workspace needs these paths to exist.
///
/// This function walks through each top-level library and symlinks any nested `lib/`
/// directories to ensure they're accessible in the temp workspace.
fn symlink_nested_libs(lib_src: &Path, lib_dst: &Path) -> Result<()> {
    // Try to load nested library's config to get its actual lib paths.
    // Fall back to default "lib" if no config exists.
    let nested_lib_dirs: Vec<PathBuf> =
        if let Ok(config) = Config::load_with_root_and_fallback(lib_src) {
            config.libs
        } else {
            vec![PathBuf::from("lib")]
        };

    for nested_lib_dir in nested_lib_dirs {
        let nested_lib = lib_src.join(&nested_lib_dir);
        if !nested_lib.exists() || !nested_lib.is_dir() {
            continue;
        }

        process_nested_lib_dir(&nested_lib, lib_dst, &nested_lib_dir)?;
    }

    Ok(())
}

/// Process a single nested lib directory, symlinking its contents.
fn process_nested_lib_dir(nested_lib: &Path, lib_dst: &Path, lib_rel: &Path) -> Result<()> {
    if !nested_lib.exists() || !nested_lib.is_dir() {
        return Ok(());
    }

    // Read entries in the nested lib directory
    let entries = match fs::read_dir(nested_lib) {
        Ok(e) => e,
        Err(_) => return Ok(()), // Skip if we can't read
    };

    for entry in entries.flatten() {
        let entry_path = entry.path();
        if !entry_path.is_dir() {
            continue;
        }

        let entry_name = entry.file_name();
        let nested_dst = lib_dst.join(lib_rel).join(&entry_name);

        // Only create if doesn't exist (symlinked parent may already provide it)
        if !nested_dst.exists() {
            // Ensure parent exists
            if let Some(parent) = nested_dst.parent() {
                let _ = fs::create_dir_all(parent);
            }
            // Symlink the nested library
            let _ = symlink_dir(&entry_path, &nested_dst);
        }

        // Recurse into nested libs (handles deeply nested submodules)
        symlink_nested_libs(&entry_path, &nested_dst)?;
    }

    Ok(())
}

/// Recursively copy a directory, skipping symlinked directories for safety.
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    if !src.exists() {
        return Ok(());
    }

    fs::create_dir_all(dst)?;

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let dest_path = dst.join(entry.file_name());

        // Use symlink_metadata to detect symlinks without following them
        let meta = fs::symlink_metadata(&path)?;

        if meta.file_type().is_symlink() {
            // Skip symlinked directories to prevent traversal attacks
            // For symlinked files, we copy the target content
            if path.is_dir() {
                continue;
            }
            fs::copy(&path, &dest_path)?;
        } else if meta.is_dir() {
            copy_dir_recursive(&path, &dest_path)?;
        } else {
            fs::copy(&path, &dest_path)?;
        }
    }

    Ok(())
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

    ensure_safe_relative_path(&source_relative, "source", &source_abs)?;

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
    if let Err(e) = copy_project_for_mutation(config, temp_dir.path()) {
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
    let src_rel = relative_to_root(&config.root, &config.src);
    let test_rel = relative_to_root(&config.root, &config.test);

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
            let lib_rel = relative_to_root(&config.root, lib);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to create a directory structure for testing
    fn create_test_dir_structure(base: &Path, structure: &[&str]) {
        for path in structure {
            let full_path = base.join(path);
            if path.ends_with('/') {
                fs::create_dir_all(&full_path).unwrap();
            } else {
                if let Some(parent) = full_path.parent() {
                    fs::create_dir_all(parent).unwrap();
                }
                fs::write(&full_path, format!("// {path}")).unwrap();
            }
        }
    }

    #[test]
    fn test_symlink_dir_creates_symlink() {
        let temp = TempDir::new().unwrap();
        let src = temp.path().join("source_dir");
        let dst = temp.path().join("target_link");

        fs::create_dir(&src).unwrap();
        fs::write(src.join("file.txt"), "content").unwrap();

        symlink_dir(&src, &dst).unwrap();

        assert!(dst.exists());
        assert!(dst.is_symlink());
        assert!(dst.join("file.txt").exists());
    }

    #[test]
    fn test_symlink_nested_libs_single_level() {
        let temp = TempDir::new().unwrap();

        // Create source lib with nested lib directory
        let lib_src = temp.path().join("lib_src");
        create_test_dir_structure(
            &lib_src,
            &[
                "src/Contract.sol",
                "lib/",
                "lib/openzeppelin/contracts/token/ERC20.sol",
                "lib/solmate/src/tokens/ERC20.sol",
            ],
        );

        // Create destination (simulating symlinked lib in temp workspace)
        let lib_dst = temp.path().join("lib_dst");
        fs::create_dir(&lib_dst).unwrap();

        symlink_nested_libs(&lib_src, &lib_dst).unwrap();

        // Verify nested libs are symlinked
        assert!(lib_dst.join("lib/openzeppelin").exists());
        assert!(lib_dst.join("lib/solmate").exists());
        assert!(lib_dst.join("lib/openzeppelin/contracts/token/ERC20.sol").exists());
        assert!(lib_dst.join("lib/solmate/src/tokens/ERC20.sol").exists());
    }

    #[test]
    fn test_symlink_nested_libs_deeply_nested() {
        let temp = TempDir::new().unwrap();

        // Create deeply nested structure (3 levels)
        let lib_src = temp.path().join("lib_src");
        create_test_dir_structure(
            &lib_src,
            &[
                "src/Main.sol",
                "lib/",
                "lib/dep-a/src/A.sol",
                "lib/dep-a/lib/",
                "lib/dep-a/lib/dep-b/src/B.sol",
                "lib/dep-a/lib/dep-b/lib/",
                "lib/dep-a/lib/dep-b/lib/dep-c/src/C.sol",
            ],
        );

        let lib_dst = temp.path().join("lib_dst");
        fs::create_dir(&lib_dst).unwrap();

        symlink_nested_libs(&lib_src, &lib_dst).unwrap();

        // All levels should be accessible
        assert!(lib_dst.join("lib/dep-a").exists());
        assert!(lib_dst.join("lib/dep-a/lib/dep-b").exists());
        assert!(lib_dst.join("lib/dep-a/lib/dep-b/lib/dep-c").exists());
        assert!(lib_dst.join("lib/dep-a/lib/dep-b/lib/dep-c/src/C.sol").exists());
    }

    #[test]
    fn test_symlink_nested_libs_no_nested_lib_dir() {
        let temp = TempDir::new().unwrap();

        // Create lib without nested lib directory
        let lib_src = temp.path().join("lib_src");
        create_test_dir_structure(&lib_src, &["src/Contract.sol", "test/Test.sol"]);

        let lib_dst = temp.path().join("lib_dst");
        fs::create_dir(&lib_dst).unwrap();

        // Should not error when no lib/ exists
        symlink_nested_libs(&lib_src, &lib_dst).unwrap();

        // lib_dst/lib should not exist
        assert!(!lib_dst.join("lib").exists());
    }

    #[test]
    fn test_symlink_nested_libs_skips_existing() {
        let temp = TempDir::new().unwrap();

        let lib_src = temp.path().join("lib_src");
        create_test_dir_structure(&lib_src, &["lib/", "lib/existing/src/File.sol"]);

        let lib_dst = temp.path().join("lib_dst");
        fs::create_dir_all(lib_dst.join("lib/existing")).unwrap();
        fs::write(lib_dst.join("lib/existing/marker.txt"), "pre-existing").unwrap();

        symlink_nested_libs(&lib_src, &lib_dst).unwrap();

        // Should not overwrite existing directory
        assert!(lib_dst.join("lib/existing/marker.txt").exists());
    }

    #[test]
    fn test_copy_dir_recursive_basic() {
        let temp = TempDir::new().unwrap();

        let src = temp.path().join("src");
        create_test_dir_structure(
            &src,
            &["file1.sol", "subdir/file2.sol", "subdir/nested/file3.sol"],
        );

        let dst = temp.path().join("dst");
        copy_dir_recursive(&src, &dst).unwrap();

        assert!(dst.join("file1.sol").exists());
        assert!(dst.join("subdir/file2.sol").exists());
        assert!(dst.join("subdir/nested/file3.sol").exists());
    }

    #[test]
    fn test_copy_dir_recursive_skips_symlinked_dirs() {
        let temp = TempDir::new().unwrap();

        let src = temp.path().join("src");
        let external = temp.path().join("external");

        // Create external directory and symlink to it
        fs::create_dir_all(&external).unwrap();
        fs::write(external.join("secret.txt"), "should not be copied").unwrap();

        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("file.sol"), "content").unwrap();

        // Create symlink inside src pointing to external
        symlink_dir(&external, &src.join("external_link")).unwrap();

        let dst = temp.path().join("dst");
        copy_dir_recursive(&src, &dst).unwrap();

        // Regular file should be copied
        assert!(dst.join("file.sol").exists());
        // Symlinked directory should be skipped
        assert!(!dst.join("external_link").exists());
    }

    #[test]
    fn test_copy_dir_recursive_nonexistent_src() {
        let temp = TempDir::new().unwrap();

        let src = temp.path().join("nonexistent");
        let dst = temp.path().join("dst");

        // Should not error for nonexistent source
        copy_dir_recursive(&src, &dst).unwrap();
        assert!(!dst.exists());
    }

    #[test]
    fn test_relative_to_root_basic() {
        let root = PathBuf::from("/project");
        let path = PathBuf::from("/project/src/contracts");

        let rel = relative_to_root(&root, &path);
        assert_eq!(rel, PathBuf::from("src/contracts"));
    }

    #[test]
    fn test_relative_to_root_same_path() {
        let root = PathBuf::from("/project");
        let path = PathBuf::from("/project");

        let rel = relative_to_root(&root, &path);
        assert_eq!(rel, PathBuf::from(""));
    }

    #[test]
    fn test_relative_to_root_outside_root() {
        let root = PathBuf::from("/project");
        let path = PathBuf::from("/other/location");

        // When path is outside root, returns the original path
        let rel = relative_to_root(&root, &path);
        assert_eq!(rel, path);
    }
}
