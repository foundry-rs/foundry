use std::{
    any::{Any, TypeId},
    cell::RefCell,
    env,
    ffi::OsString,
    fmt, io,
    ops::Deref,
    path::PathBuf,
    ptr::NonNull,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
};

thread_local! {
    static CURRENT_STATE: State = State {
        current: RefCell::new(Process::default()),
    };
}

static EXISTS: AtomicBool = AtomicBool::new(false);
static CURRENT_COUNT: AtomicUsize = AtomicUsize::new(0);

// tracks the state of `GLOBAL_PROCESS`
static GLOBAL_PROCESS_STATE: AtomicUsize = AtomicUsize::new(UN_SET);

const UN_SET: usize = 0;
const SETTING: usize = 1;
const SET: usize = 2;

static mut GLOBAL_PROCESS: Option<Process> = None;

/// Install this `Processor` as the global default if one is
/// not already set.
///
/// # Errors
///
/// Returns an Error if the initialization was unsuccessful, likely
/// because a global processor was already installed by another
/// call to `try_init`.
pub fn try_init<T>(processor: T) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>>
where
    T: Processor + Send + Sync + 'static,
{
    set_global_process(Process::new(processor))?;
    Ok(())
}

/// Install this `Processor` as the global default.
///
/// # Panics
///
/// Panics if the initialization was unsuccessful, likely because a
/// global processor was already installed by another call to `try_init`.
pub fn init<T>(processor: T)
where
    T: Processor + Send + Sync + 'static,
{
    try_init(processor).expect("Failed to install global processor")
}

/// The core trait which bundles various utilities.
///
/// The main reasons for this abstraction is that this becomes customisable for tests.
///
/// A `Process` can be installed globally exactly once via `init`, or set for the current scope with
/// `set_current()`, which register the process in a `thread_local!` variable, so when making new
/// threads, e sure to clone the process into the new thread before using any functions from
/// `Process`. Otherwise, it would fallback to the global `Process`
pub trait Processor: 'static + fmt::Debug + Send + Sync {
    fn home_dir(&self) -> Option<PathBuf> {
        dirs_next::home_dir()
    }
    fn current_dir(&self) -> io::Result<PathBuf> {
        env::current_dir()
    }

    /// Returns an iterator over all env args
    fn args(&self) -> Box<dyn Iterator<Item = String>> {
        Box::new(env::args())
    }

    /// Returns an iterator over all arguments that this program was started with
    fn args_os(&self) -> Box<dyn Iterator<Item = OsString>> {
        Box::new(env::args_os())
    }

    fn var(&self, key: &str) -> Result<String, env::VarError> {
        env::var(key)
    }
    fn var_os(&self, key: &str) -> Option<OsString> {
        env::var_os(key)
    }

    /// Returns the file name of file this process was started with
    fn name(&self) -> Option<String> {
        let arg0 = match self.var("FOUNDRYUP_FORCE_ARG0") {
            Ok(v) => Some(v),
            Err(_) => self.args().next(),
        }
        .map(PathBuf::from);

        arg0.as_ref()
            .and_then(|a| a.file_stem())
            .and_then(std::ffi::OsStr::to_str)
            .map(String::from)
    }

    /// If `self` is the same type as the provided `TypeId`, returns an untyped
    /// [`NonNull`] pointer to that type. Otherwise, returns `None`.
    ///
    /// If you wish to downcast a `Processor`, it is strongly advised to use
    /// the safe API provided by downcast_ref instead.
    ///
    /// This API is required for `downcast_raw` to be a trait method; a method
    /// signature like downcast_ref (with a generic type parameter) is not
    /// object-safe, and thus cannot be a trait method for `Processor`. This
    /// means that if we only exposed downcast_ref, `Processor`
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

/// Sets this process as the global default for the duration of the entire program.
///
/// The global processor can only be set once; additional attempts to set the global processor will
/// fail. Returns `Err` if the global processor has already been set.
fn set_global_process(process: Process) -> Result<(), SetGlobalProcessError> {
    // `compare_exchange` tries to store `SETTING` if the current value is `UN_SET`
    // this returns `Ok(_)` if the current value of `GLOBAL_PROCESS_STATE` was `UN_SET` and
    // `SETTING` was written, this guarantees the value is `SETTING`.
    if GLOBAL_PROCESS_STATE
        .compare_exchange(UN_SET, SETTING, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
    {
        unsafe {
            GLOBAL_PROCESS = Some(process);
        }
        GLOBAL_PROCESS_STATE.store(SET, Ordering::SeqCst);
        Ok(())
    } else {
        Err(SetGlobalProcessError { _priv: () })
    }
}

fn get_global() -> Option<&'static Process> {
    if GLOBAL_PROCESS_STATE.load(Ordering::SeqCst) != SET {
        return None
    }
    unsafe {
        // This is safe given the invariant that setting the global process
        // also sets `GLOBAL_PROCESS_STATE` to `SET`.
        Some(GLOBAL_PROCESS.as_ref().expect(
            "Process invariant violated: GLOBAL_PROCESS must be initialized before GLOBAL_PROCESS_STATE is set",
        ))
    }
}

/// Sets this processor as the scoped processor for the duration of a closure.
pub fn with<T>(process: &Process, f: impl FnOnce() -> T) -> T {
    // When this guard is dropped, the scoped processor will be reset to the
    // prior processor
    let _guard = set_current(process);
    f()
}

/// Returns a clone of the current `Process`
pub fn get_process() -> Process {
    with_default(|p| p.clone())
}

/// Executes a closure with a reference to this thread's current Processor.
#[inline(always)]
pub fn with_default<T, F>(mut f: F) -> T
where
    F: FnMut(&Process) -> T,
{
    if CURRENT_COUNT.load(Ordering::Acquire) == 0 {
        // fast path if no scoped processor has been set; use the global
        // default.
        return if let Some(glob) = get_global() { f(glob) } else { f(&Process::default()) }
    }

    with_current(f)
}

#[inline(never)]
fn with_current<T, F>(mut f: F) -> T
where
    F: FnMut(&Process) -> T,
{
    CURRENT_STATE
        .try_with(|state| {
            let current = state.current.borrow_mut();
            f(&*current)
        })
        .unwrap_or_else(|_| f(&Process::default()))
}

/// Sets the processor as the current processor for the duration of the lifetime
/// of the returned DefaultGuard
#[must_use = "Dropping the guard unregisters the processor."]
pub fn set_current(processor: &Process) -> ScopeGuard {
    // When this guard is dropped, the current processor will be reset to the
    // prior default. Using this ensures that we always reset to the prior
    // processor even if the thread calling this function panics.
    State::set_current(processor.clone())
}

/// Returned if setting the global process fails.
#[derive(Debug)]
pub struct SetGlobalProcessError {
    // private marker so this type can't be initiated
    _priv: (),
}

impl fmt::Display for SetGlobalProcessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad("a global process has already been set")
    }
}

impl std::error::Error for SetGlobalProcessError {}

#[derive(Clone, Debug)]
pub struct Process {
    process: Arc<dyn Processor>,
}

impl Process {
    /// Returns a `Process` that forwards to the given [`Processor`].
    pub fn new<S>(process: S) -> Self
    where
        S: Processor,
    {
        Self { process: Arc::new(process) }
    }
}

impl Default for Process {
    fn default() -> Self {
        Self { process: Arc::new(DefaultProcess::default()) }
    }
}

impl Deref for Process {
    type Target = dyn Processor;

    fn deref(&self) -> &Self::Target {
        &*self.process
    }
}

impl dyn Processor {
    /// Returns `true` if this `Processor` is the same type as `T`.
    pub fn is<T: Any>(&self) -> bool {
        self.downcast_ref::<T>().is_some()
    }

    /// Returns some reference to this `Processor` value if it is of type `T`,
    /// or `None` if it isn't.
    pub fn downcast_ref<T: Any>(&self) -> Option<&T> {
        unsafe {
            let raw = self.downcast_raw(TypeId::of::<T>())?;
            Some(&*(raw.cast().as_ptr()))
        }
    }
}

/// The process state of a thread.
struct State {
    /// This thread's current processor.
    current: RefCell<Process>,
}

impl State {
    /// Replaces the current  processor on this thread with the provided
    /// processor.
    ///
    /// Dropping the returned `ScopeGuard` will reset the current processor to
    /// the previous value.
    #[inline]
    fn set_current(new_process: Process) -> ScopeGuard {
        let prior = CURRENT_STATE.try_with(|state| state.current.replace(new_process)).ok();
        EXISTS.store(true, Ordering::Release);
        CURRENT_COUNT.fetch_add(1, Ordering::Release);
        ScopeGuard(prior)
    }
}

/// A guard that resets the current processor to the prior
/// current processor when dropped.
#[derive(Debug)]
pub struct ScopeGuard(Option<Process>);

impl Drop for ScopeGuard {
    #[inline]
    fn drop(&mut self) {
        CURRENT_COUNT.fetch_sub(1, Ordering::Release);
        if let Some(process) = self.0.take() {
            // Replace the processor and then drop the old one outside
            // of the thread-local context.
            let prev = CURRENT_STATE.try_with(|state| state.current.replace(process));
            drop(prev)
        }
    }
}

/// The standard `Process` impl
#[derive(Copy, Clone, Debug, Default)]
pub struct DefaultProcess(());

impl Processor for DefaultProcess {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_processor_works() {
        #[derive(Debug)]
        struct TestProcessor;
        impl Processor for TestProcessor {}

        with(&Process::new(TestProcessor), || {
            with_default(|processor| assert!(processor.is::<TestProcessor>()))
        });
    }
}
