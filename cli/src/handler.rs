use eyre::EyreHandler;
use foundry_config::error::FAILED_TO_EXTRACT_CONFIG_PANIC_MSG;
use once_cell::sync::OnceCell;
use std::{error::Error, fmt};
use yansi::Paint;

const BUG_REPORT: &str =
    "This is a bug. Consider reporting it at https://github.com/foundry-rs/foundry";

/// Contains the panic section if initialized
static SECTION: OnceCell<BugPanicSection> = OnceCell::new();

/// Responsible for displaying the panic section
struct PanicSection;

impl fmt::Display for PanicSection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        SECTION.get().copied().unwrap_or_default().fmt(f)
    }
}

/// Represents the panic section
#[derive(Debug, Copy, Clone)]
struct BugPanicSection {
    /// whether to display the `BUG_REPORT` section
    is_bug: bool,
}

impl BugPanicSection {
    const fn bug() -> Self {
        Self { is_bug: true }
    }
    const fn no_bug() -> Self {
        Self { is_bug: false }
    }
}

impl Default for BugPanicSection {
    fn default() -> Self {
        BugPanicSection::bug()
    }
}

impl fmt::Display for BugPanicSection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_bug {
            f.write_str(BUG_REPORT)?;
        }
        Ok(())
    }
}

/// A custom context type for Foundry specific error reporting via `eyre`
#[derive(Debug)]
pub struct Handler;

impl EyreHandler for Handler {
    fn debug(&self, error: &(dyn Error + 'static), f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if f.alternate() {
            return fmt::Debug::fmt(error, f)
        }
        writeln!(f)?;
        write!(f, "{}", Paint::red(error))?;

        if let Some(cause) = error.source() {
            write!(f, "\n\nContext:")?;

            let multiple = cause.source().is_some();
            let errors = std::iter::successors(Some(cause), |e| (*e).source());

            for (n, error) in errors.enumerate() {
                writeln!(f)?;
                if multiple {
                    write!(f, "- Error #{}: {}", n, error)?;
                } else {
                    write!(f, "- {}", error)?;
                }
            }
        }

        Ok(())
    }
}

/// A wrapper around `color-eyre`'s PanicHook that's used to intercept the panic message
struct PanicHook {
    panic_hook: color_eyre::config::PanicHook,
}

// === impl PanicHook ===

impl PanicHook {
    /// Install self as a global panic hook via `std::panic::set_hook`.
    pub fn install(self) {
        std::panic::set_hook(self.into_panic_hook());
    }

    /// Convert self into the type expected by `std::panic::set_hook`.
    pub fn into_panic_hook(
        self,
    ) -> Box<dyn Fn(&std::panic::PanicInfo<'_>) + Send + Sync + 'static> {
        Box::new(move |panic_info| {
            let payload = panic_info.payload();
            install_panic_section(ErrorKind::NonRecoverable(payload));
            eprintln!("{}", self.panic_hook.panic_report(panic_info));
        })
    }
}

/// The kind of type erased error being reported
enum ErrorKind<'a> {
    /// A non recoverable error aka `panic!`
    NonRecoverable(&'a dyn std::any::Any),
    /// A recoverable error aka `impl std::error::Error`
    #[allow(unused)]
    Recoverable(&'a (dyn Error + 'static)),
}

/// tries to reason whether this is an actual bug or something else, like invalid config etc
fn is_no_bug_error(error: &(dyn Error + 'static)) -> bool {
    // TODO additional errors to exclude
    error.is::<foundry_config::figment::Error>()
}

/// Returns true if the `panic_message` is known and not a considered a bug
fn is_no_bug_panic(panic_message: &str) -> bool {
    panic_message.contains(FAILED_TO_EXTRACT_CONFIG_PANIC_MSG)
}

/// This installs the appropriate `BugReport` that links to the repo's issue section in the
/// panic section. This circumvents the auto-generated issue content and title which would override
/// foundry's template
fn install_panic_section(kind: ErrorKind) {
    let is_no_bug = match kind {
        ErrorKind::NonRecoverable(payload) => {
            let payload = payload
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| payload.downcast_ref::<&str>().cloned())
                .unwrap_or("<non string panic payload>");

            is_no_bug_panic(payload)
        }
        ErrorKind::Recoverable(error) => is_no_bug_error(error),
    };

    // if we determined that the issue is unrelated to a bug, like misconfigured config, then we set
    // the panic section accordingly
    if is_no_bug {
        let _ = SECTION.set(BugPanicSection::no_bug());
    }
}

/// Installs the Foundry eyre hook as the global error report hook.
///
/// # Details
///
/// By default, a simple user-centric handler is installed, unless
/// `FOUNDRY_DEBUG` is set in the environment, in which case a more
/// verbose debug-centric handler is installed.
///
/// Panics are always caught by the more debug-centric handler.
pub fn install() -> eyre::Result<()> {
    let debug_enabled = std::env::var("FOUNDRY_DEBUG").is_ok();

    if debug_enabled {
        color_eyre::install()?;
    } else {
        let (panic_hook, _) =
            color_eyre::config::HookBuilder::default().panic_section(PanicSection).into_hooks();
        PanicHook { panic_hook }.install();
        eyre::set_hook(Box::new(move |_| Box::new(Handler)))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[should_panic]
    fn test_panic_hook() {
        install().unwrap();
        panic!("{}", FAILED_TO_EXTRACT_CONFIG_PANIC_MSG)
    }
}
