use eyre::EyreHandler;
use std::error::Error;
use yansi::Paint;

/// A custom context type for Foundry specific error reporting via `eyre`
#[derive(Debug)]
pub struct Handler;

impl EyreHandler for Handler {
    fn debug(
        &self,
        error: &(dyn Error + 'static),
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        if f.alternate() {
            return core::fmt::Debug::fmt(error, f)
        }
        writeln!(f)?;
        write!(f, "{}", error.red())?;

        if let Some(cause) = error.source() {
            write!(f, "\n\nContext:")?;

            let multiple = cause.source().is_some();
            let errors = std::iter::successors(Some(cause), |e| (*e).source());

            for (n, error) in errors.enumerate() {
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
}

/// Installs the Foundry eyre hook as the global error report hook.
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
        std::env::set_var("RUST_BACKTRACE", "1");
    }

    if std::env::var_os("FOUNDRY_DEBUG").is_some() {
        if let Err(e) = color_eyre::install() {
            debug!("failed to install color eyre error hook: {e}");
        }
    } else {
        let (panic_hook, _) = color_eyre::config::HookBuilder::default()
            .panic_section(
                "This is a bug. Consider reporting it at https://github.com/foundry-rs/foundry",
            )
            .into_hooks();
        panic_hook.install();
        if let Err(e) = eyre::set_hook(Box::new(move |_| Box::new(Handler))) {
            debug!("failed to install eyre error hook: {e}");
        }
    }
}
