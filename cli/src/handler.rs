use eyre::EyreHandler;
use std::error::Error;
use tracing::error;
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
        write!(f, "{}", Paint::red(error))?;

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
#[cfg_attr(windows, inline(never))]
pub fn install() -> eyre::Result<()> {
    let debug_enabled = std::env::var("FOUNDRY_DEBUG").is_ok();

    if debug_enabled {
        color_eyre::install()?;
    } else {
        let (panic_hook, _) = color_eyre::config::HookBuilder::default()
            .panic_section(
                "This is a bug. Consider reporting it at https://github.com/foundry-rs/foundry",
            )
            .into_hooks();
        panic_hook.install();
        // see <https://github.com/foundry-rs/foundry/issues/3050>
        if cfg!(windows) {
            if let Err(err) = eyre::set_hook(Box::new(move |_| Box::new(Handler))) {
                error!(?err, "failed to install panic hook");
            }
        } else {
            eyre::set_hook(Box::new(move |_| Box::new(Handler)))?;
        }
    }

    Ok(())
}
