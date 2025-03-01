//! Subscribe to events in the compiler pipeline
//!
//! The _reporter_ is the component of the [`crate::Project::compile()`] pipeline which is
//! responsible for reporting on specific steps in the process.
//!
//! By default, the current reporter is a noop that does
//! nothing.
//!
//! To use another report implementation, it must be set as the current reporter.
//! There are two methods for doing so: [`with_scoped`] and
//! [`try_init`]. `with_scoped` sets the reporter for the
//! duration of a scope, while `set_global` sets a global default report
//! for the entire process.

// <https://github.com/tokio-rs/tracing/blob/master/tracing-core/src/dispatch.rs>

#![allow(static_mut_refs)] // TODO

use foundry_compilers_artifacts::remappings::Remapping;
use semver::Version;
use std::{
    any::{Any, TypeId},
    cell::RefCell,
    error::Error,
    fmt,
    path::{Path, PathBuf},
    ptr::NonNull,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

mod compiler;
pub use compiler::SolcCompilerIoReporter;

thread_local! {
    static CURRENT_STATE: State = State {
        scoped: RefCell::new(Report::none()),
    };
}

static EXISTS: AtomicBool = AtomicBool::new(false);
static SCOPED_COUNT: AtomicUsize = AtomicUsize::new(0);

// tracks the state of `GLOBAL_REPORTER`
static GLOBAL_REPORTER_STATE: AtomicUsize = AtomicUsize::new(UN_SET);

const UN_SET: usize = 0;
const SETTING: usize = 1;
const SET: usize = 2;

static mut GLOBAL_REPORTER: Option<Report> = None;

/// Install this `Reporter` as the global default if one is
/// not already set.
///
/// # Errors
/// Returns an Error if the initialization was unsuccessful, likely
/// because a global reporter was already installed by another
/// call to `try_init`.
pub fn try_init<T>(reporter: T) -> Result<(), Box<dyn Error + Send + Sync + 'static>>
where
    T: Reporter + Send + Sync + 'static,
{
    set_global_reporter(Report::new(reporter))?;
    Ok(())
}

/// Install this `Reporter` as the global default.
///
/// # Panics
///
/// Panics if the initialization was unsuccessful, likely because a
/// global reporter was already installed by another call to `try_init`.
/// ```
/// use foundry_compilers::report::BasicStdoutReporter;
/// let subscriber = foundry_compilers::report::init(BasicStdoutReporter::default());
/// ```
pub fn init<T>(reporter: T)
where
    T: Reporter + Send + Sync + 'static,
{
    try_init(reporter).expect("Failed to install global reporter")
}

/// Trait representing the functions required to emit information about various steps in the
/// compiler pipeline.
///
/// This trait provides a series of callbacks that are invoked at certain parts of the
/// [`crate::Project::compile()`] process.
///
/// Implementers of this trait can use these callbacks to emit additional information, for example
/// print custom messages to `stdout`.
///
/// A `Reporter` is entirely passive and only listens to incoming "events".
pub trait Reporter: 'static + std::fmt::Debug {
    /// Callback invoked right before [Compiler::compile] is called
    ///
    /// This contains the [Compiler] its [Version] and all files that triggered the compile job. The
    /// dirty files are only provided to give a better feedback what was actually compiled.
    ///
    /// [Compiler]: crate::compilers::Compiler
    /// [Compiler::compile]: crate::compilers::Compiler::compile
    fn on_compiler_spawn(
        &self,
        _compiler_name: &str,
        _version: &Version,
        _dirty_files: &[PathBuf],
    ) {
    }

    /// Invoked with the `CompilerOutput` if [`Compiler::compile()`] was successful
    ///
    /// [`Compiler::compile()`]: crate::compilers::Compiler::compile
    fn on_compiler_success(&self, _compiler_name: &str, _version: &Version, _duration: &Duration) {}

    /// Invoked before a new compiler version is installed
    fn on_solc_installation_start(&self, _version: &Version) {}

    /// Invoked after a new compiler version was successfully installed
    fn on_solc_installation_success(&self, _version: &Version) {}

    /// Invoked after a compiler installation failed
    fn on_solc_installation_error(&self, _version: &Version, _error: &str) {}

    /// Invoked if imports couldn't be resolved with the given remappings, where `imports` is the
    /// list of all import paths and the file they occurred in: `(import stmt, file)`
    fn on_unresolved_imports(&self, _imports: &[(&Path, &Path)], _remappings: &[Remapping]) {}

    /// If `self` is the same type as the provided `TypeId`, returns an untyped
    /// [`NonNull`] pointer to that type. Otherwise, returns `None`.
    ///
    /// If you wish to downcast a `Reporter`, it is strongly advised to use
    /// the safe API provided by downcast_ref instead.
    ///
    /// This API is required for `downcast_raw` to be a trait method; a method
    /// signature like downcast_ref (with a generic type parameter) is not
    /// object-safe, and thus cannot be a trait method for `Reporter`. This
    /// means that if we only exposed downcast_ref, `Reporter`
    /// implementations could not override the downcasting behavior
    ///
    /// # Safety
    ///
    /// The downcast_ref method expects that the pointer returned by
    /// `downcast_raw` points to a valid instance of the type
    /// with the provided `TypeId`. Failure to ensure this will result in
    /// undefined behaviour, so implementing `downcast_raw` is unsafe.
    unsafe fn downcast_raw(&self, id: TypeId) -> Option<NonNull<()>> {
        if id == TypeId::of::<Self>() {
            Some(NonNull::from(self).cast())
        } else {
            None
        }
    }
}

impl dyn Reporter {
    /// Returns `true` if this `Reporter` is the same type as `T`.
    pub fn is<T: Any>(&self) -> bool {
        self.downcast_ref::<T>().is_some()
    }

    /// Returns some reference to this `Reporter` value if it is of type `T`,
    /// or `None` if it isn't.
    pub fn downcast_ref<T: Any>(&self) -> Option<&T> {
        unsafe {
            let raw = self.downcast_raw(TypeId::of::<T>())?;
            Some(&*(raw.cast().as_ptr()))
        }
    }
}

pub(crate) fn compiler_spawn(compiler_name: &str, version: &Version, dirty_files: &[PathBuf]) {
    get_default(|r| r.reporter.on_compiler_spawn(compiler_name, version, dirty_files));
}

pub(crate) fn compiler_success(compiler_name: &str, version: &Version, duration: &Duration) {
    get_default(|r| r.reporter.on_compiler_success(compiler_name, version, duration));
}

#[allow(dead_code)]
pub(crate) fn solc_installation_start(version: &Version) {
    get_default(|r| r.reporter.on_solc_installation_start(version));
}

#[allow(dead_code)]
pub(crate) fn solc_installation_success(version: &Version) {
    get_default(|r| r.reporter.on_solc_installation_success(version));
}

#[allow(dead_code)]
pub(crate) fn solc_installation_error(version: &Version, error: &str) {
    get_default(|r| r.reporter.on_solc_installation_error(version, error));
}

pub(crate) fn unresolved_imports(imports: &[(&Path, &Path)], remappings: &[Remapping]) {
    get_default(|r| r.reporter.on_unresolved_imports(imports, remappings));
}

fn get_global() -> Option<&'static Report> {
    if GLOBAL_REPORTER_STATE.load(Ordering::SeqCst) != SET {
        return None;
    }
    unsafe {
        // This is safe given the invariant that setting the global reporter
        // also sets `GLOBAL_REPORTER_STATE` to `SET`.
        Some(GLOBAL_REPORTER.as_ref().expect(
            "Reporter invariant violated: GLOBAL_REPORTER must be initialized before GLOBAL_REPORTER_STATE is set",
        ))
    }
}

/// Executes a closure with a reference to this thread's current reporter.
#[inline(always)]
pub fn get_default<T, F>(mut f: F) -> T
where
    F: FnMut(&Report) -> T,
{
    if SCOPED_COUNT.load(Ordering::Acquire) == 0 {
        // fast path if no scoped reporter has been set; use the global
        // default.
        return if let Some(glob) = get_global() { f(glob) } else { f(&Report::none()) };
    }

    get_default_scoped(f)
}

#[inline(never)]
fn get_default_scoped<T, F>(mut f: F) -> T
where
    F: FnMut(&Report) -> T,
{
    CURRENT_STATE
        .try_with(|state| {
            let scoped = state.scoped.borrow_mut();
            f(&scoped)
        })
        .unwrap_or_else(|_| f(&Report::none()))
}

/// Executes a closure with a reference to the `Reporter`.
pub fn with_global<T>(f: impl FnOnce(&Report) -> T) -> Option<T> {
    let report = get_global()?;
    Some(f(report))
}

/// Sets this reporter as the scoped reporter for the duration of a closure.
pub fn with_scoped<T>(report: &Report, f: impl FnOnce() -> T) -> T {
    // When this guard is dropped, the scoped reporter will be reset to the
    // prior reporter. Using this (rather than simply resetting after calling
    // `f`) ensures that we always reset to the prior reporter even if `f`
    // panics.
    let _guard = set_scoped(report);
    f()
}

/// The report state of a thread.
struct State {
    /// This thread's current scoped reporter.
    scoped: RefCell<Report>,
}

impl State {
    /// Replaces the current scoped reporter on this thread with the provided
    /// reporter.
    ///
    /// Dropping the returned `ResetGuard` will reset the scoped reporter to
    /// the previous value.
    #[inline]
    fn set_scoped(new_report: Report) -> ScopeGuard {
        let prior = CURRENT_STATE.try_with(|state| state.scoped.replace(new_report)).ok();
        EXISTS.store(true, Ordering::Release);
        SCOPED_COUNT.fetch_add(1, Ordering::Release);
        ScopeGuard(prior)
    }
}

/// A guard that resets the current scoped reporter to the prior
/// scoped reporter when dropped.
#[derive(Debug)]
pub struct ScopeGuard(Option<Report>);

impl Drop for ScopeGuard {
    #[inline]
    fn drop(&mut self) {
        SCOPED_COUNT.fetch_sub(1, Ordering::Release);
        if let Some(report) = self.0.take() {
            // Replace the reporter and then drop the old one outside
            // of the thread-local context.
            let prev = CURRENT_STATE.try_with(|state| state.scoped.replace(report));
            drop(prev)
        }
    }
}

/// Sets the reporter as the scoped reporter for the duration of the lifetime
/// of the returned DefaultGuard
#[must_use = "Dropping the guard unregisters the reporter."]
pub fn set_scoped(reporter: &Report) -> ScopeGuard {
    // When this guard is dropped, the scoped reporter will be reset to the
    // prior default. Using this ensures that we always reset to the prior
    // reporter even if the thread calling this function panics.
    State::set_scoped(reporter.clone())
}

/// A no-op [`Reporter`] that does nothing.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoReporter(());

impl Reporter for NoReporter {}

/// A [`Reporter`] that emits some general information to `stdout`
#[derive(Clone, Debug, Default)]
pub struct BasicStdoutReporter {
    _priv: (),
}

impl Reporter for BasicStdoutReporter {
    /// Callback invoked right before [`Compiler::compile()`] is called
    ///
    /// [`Compiler::compile()`]: crate::compilers::Compiler::compile
    fn on_compiler_spawn(&self, compiler_name: &str, version: &Version, dirty_files: &[PathBuf]) {
        println!(
            "Compiling {} files with {} {}.{}.{}",
            dirty_files.len(),
            compiler_name,
            version.major,
            version.minor,
            version.patch
        );
    }

    fn on_compiler_success(&self, compiler_name: &str, version: &Version, duration: &Duration) {
        println!(
            "{} {}.{}.{} finished in {duration:.2?}",
            compiler_name, version.major, version.minor, version.patch
        );
    }

    /// Invoked before a new compiler is installed
    fn on_solc_installation_start(&self, version: &Version) {
        println!("installing solc version \"{version}\"");
    }

    /// Invoked before a new compiler was successfully installed
    fn on_solc_installation_success(&self, version: &Version) {
        println!("Successfully installed solc {version}");
    }

    fn on_solc_installation_error(&self, version: &Version, error: &str) {
        eprintln!("Failed to install solc {version}: {error}");
    }

    fn on_unresolved_imports(&self, imports: &[(&Path, &Path)], remappings: &[Remapping]) {
        if imports.is_empty() {
            return;
        }
        println!("{}", format_unresolved_imports(imports, remappings))
    }
}

/// Creates a meaningful message for all unresolved imports
pub fn format_unresolved_imports(imports: &[(&Path, &Path)], remappings: &[Remapping]) -> String {
    let info = imports
        .iter()
        .map(|(import, file)| format!("\"{}\" in \"{}\"", import.display(), file.display()))
        .collect::<Vec<_>>()
        .join("\n      ");
    format!(
        "Unable to resolve imports:\n      {}\nwith remappings:\n      {}",
        info,
        remappings.iter().map(|r| r.to_string()).collect::<Vec<_>>().join("\n      ")
    )
}

/// Returned if setting the global reporter fails.
#[derive(Debug)]
pub struct SetGlobalReporterError {
    // private marker so this type can't be initiated
    _priv: (),
}

impl fmt::Display for SetGlobalReporterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad("a global reporter has already been set")
    }
}

impl Error for SetGlobalReporterError {}

/// `Report` trace data to a [`Reporter`].
#[derive(Clone)]
pub struct Report {
    reporter: Arc<dyn Reporter + Send + Sync>,
}

impl Report {
    /// Returns a new `Report` that does nothing
    pub fn none() -> Self {
        Self { reporter: Arc::new(NoReporter::default()) }
    }

    /// Returns a `Report` that forwards to the given [`Reporter`].
    ///
    /// [`Reporter`]: ../reporter/trait.Reporter.html
    pub fn new<S>(reporter: S) -> Self
    where
        S: Reporter + Send + Sync + 'static,
    {
        Self { reporter: Arc::new(reporter) }
    }

    /// Returns `true` if this `Report` forwards to a reporter of type
    /// `T`.
    #[inline]
    pub fn is<T: Any>(&self) -> bool {
        <dyn Reporter>::is::<T>(&*self.reporter)
    }
}

impl fmt::Debug for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad("Report(...)")
    }
}

/// Sets this report as the global default for the duration of the entire program.
///
/// The global reporter can only be set once; additional attempts to set the global reporter will
/// fail. Returns `Err` if the global reporter has already been set.
fn set_global_reporter(report: Report) -> Result<(), SetGlobalReporterError> {
    // `compare_exchange` tries to store `SETTING` if the current value is `UN_SET`
    // this returns `Ok(_)` if the current value of `GLOBAL_REPORTER_STATE` was `UN_SET` and
    // `SETTING` was written, this guarantees the value is `SETTING`.
    if GLOBAL_REPORTER_STATE
        .compare_exchange(UN_SET, SETTING, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
    {
        unsafe {
            GLOBAL_REPORTER = Some(report);
        }
        GLOBAL_REPORTER_STATE.store(SET, Ordering::SeqCst);
        Ok(())
    } else {
        Err(SetGlobalReporterError { _priv: () })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn scoped_reporter_works() {
        #[derive(Debug)]
        struct TestReporter;
        impl Reporter for TestReporter {}

        with_scoped(&Report::new(TestReporter), || {
            get_default(|reporter| assert!(reporter.is::<TestReporter>()))
        });
    }

    #[test]
    fn global_and_scoped_reporter_works() {
        get_default(|reporter| {
            assert!(reporter.is::<NoReporter>());
        });

        set_global_reporter(Report::new(BasicStdoutReporter::default())).unwrap();
        #[derive(Debug)]
        struct TestReporter;
        impl Reporter for TestReporter {}

        with_scoped(&Report::new(TestReporter), || {
            get_default(|reporter| assert!(reporter.is::<TestReporter>()))
        });

        get_default(|reporter| assert!(reporter.is::<BasicStdoutReporter>()))
    }

    #[test]
    fn test_unresolved_message() {
        let unresolved = vec![(Path::new("./src/Import.sol"), Path::new("src/File.col"))];

        let remappings = vec![Remapping::from_str("oz=a/b/c/d").unwrap()];

        assert_eq!(
            format_unresolved_imports(&unresolved, &remappings).trim(),
            r#"
Unable to resolve imports:
      "./src/Import.sol" in "src/File.col"
with remappings:
      oz/=a/b/c/d/"#
                .trim()
        )
    }
}
