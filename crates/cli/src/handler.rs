use eyre::EyreHandler;
use itertools::Itertools;
use std::{error::Error, fmt};

/// A custom context type for Foundry specific error reporting via `eyre`.
pub struct Handler {
    debug_handler: Option<Box<dyn EyreHandler>>,
}

impl Default for Handler {
    fn default() -> Self {
        Self::new()
    }
}

impl Handler {
    /// Create a new instance of the `Handler`.
    pub fn new() -> Self {
        Self { debug_handler: None }
    }

    /// Override the debug handler with a custom one.
    pub fn debug_handler(mut self, debug_handler: Option<Box<dyn EyreHandler>>) -> Self {
        self.debug_handler = debug_handler;
        self
    }
}

impl EyreHandler for Handler {
    fn display(&self, error: &(dyn Error + 'static), f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use fmt::Display;
        foundry_common::errors::dedup_chain(error).into_iter().format("; ").fmt(f)
    }

    fn debug(&self, error: &(dyn Error + 'static), f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(debug_handler) = &self.debug_handler {
            return debug_handler.debug(error, f);
        }

        if f.alternate() {
            return fmt::Debug::fmt(error, f);
        }
        let errors = foundry_common::errors::dedup_chain(error);

        let (error, sources) = errors.split_first().unwrap();
        write!(f, "{error}")?;

        if !sources.is_empty() {
            write!(f, "\n\nContext:")?;

            let multiple = sources.len() > 1;
            for (n, error) in sources.iter().enumerate() {
                writeln!(f)?;
                if multiple {
                    write!(f, "- Error #{n}: {error}")?;
                } else {
                    write!(f, "- {error}")?;
                }
            }
        }

        Ok(())
    }

    fn track_caller(&mut self, location: &'static std::panic::Location<'static>) {
        if let Some(debug_handler) = &mut self.debug_handler {
            debug_handler.track_caller(location);
        }
    }
}

/// Installs the Foundry [`eyre`] and [`panic`](mod@std::panic) hooks as the global ones.
///
/// # Details
///
/// By default a simple user-centric handler is installed, unless
/// `FOUNDRY_DEBUG` is set in the environment, in which case a more
/// verbose debug-centric handler is installed.
///
/// Panics are always caught by the more debug-centric handler.
pub fn install() {
    if std::env::var_os("RUST_BACKTRACE").is_none() {
        unsafe {
            std::env::set_var("RUST_BACKTRACE", "1");
        }
    }

    let panic_section =
        "This is a bug. Consider reporting it at https://github.com/foundry-rs/foundry";
    let (panic_hook, debug_hook) =
        color_eyre::config::HookBuilder::default().panic_section(panic_section).into_hooks();
    panic_hook.install();
    let debug_hook = debug_hook.into_eyre_hook();
    let debug = std::env::var_os("FOUNDRY_DEBUG").is_some();
    if let Err(e) = eyre::set_hook(Box::new(move |e| {
        Box::new(Handler::new().debug_handler(debug.then(|| debug_hook(e))))
    })) {
        debug!("failed to install eyre error hook: {e}");
    }
}
