use reqwest::{
    header::{HeaderMap, HeaderValue, ACCEPT, USER_AGENT},
    Client,
};
use std::{cell::RefCell, env, ffi::OsString, fmt, io, ops::Deref, path::PathBuf, sync::Arc};

thread_local! {
    static CURRENT_STATE: State = State {
        current: RefCell::new(Process::default()),
    };
}

/// The core trait which bundles various utilities.
///
/// The main reasons for this abstraction is that this becomes customisable for tests.
///
/// A `Process` can be installed manually  for the current scope via `set_current`, which register
/// the process in a `thread_local!` variable, so when making new threads, be sure to clone the
/// process into the new thread before using any functions from `Process`.
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

    /// Returns the client to make requests with
    fn client(&self) -> &Client;
}

/// Sets this processor as the scoped processor for the duration of a closure.
pub fn with<T>(process: &Process, f: impl FnOnce() -> T) -> T {
    // When this guard is dropped, the scoped processor will be reset to the
    // prior processor
    let _guard = set_current(process.clone());
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
pub fn set_current(processor: Process) -> ScopeGuard {
    // When this guard is dropped, the current processor will be reset to the
    // prior default. Using this ensures that we always reset to the prior
    // processor even if the thread calling this function panics.
    State::set_current(processor)
}

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
        if let Some(process) = self.0.take() {
            // Replace the processor and then drop the old one outside
            // of the thread-local context.
            let prev = CURRENT_STATE.try_with(|state| state.current.replace(process));
            drop(prev)
        }
    }
}

/// The default `Process` impl
///
/// The `CURRENT_STATE` will hold a new instance of this on every thread.
#[derive(Clone, Debug)]
pub struct DefaultProcess {
    client: Client,
}

impl Default for DefaultProcess {
    fn default() -> Self {
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(60))
                .default_headers(HeaderMap::from_iter([
                    (
                        USER_AGENT,
                        HeaderValue::from_static(concat!("foundryup/", env!("CARGO_PKG_VERSION"))),
                    ),
                    (ACCEPT, HeaderValue::from_static("application/vnd.github.v3+json")),
                ]))
                .build()
                .expect("Failed to create reqwest::Client"),
        }
    }
}

impl Processor for DefaultProcess {
    fn client(&self) -> &Client {
        &self.client
    }
}

#[cfg(test)]
mod tests {}
