use eyre::EyreHandler;
use std::{error::Error, fmt};

/// A custom context type for Foundry specific error reporting via `eyre`
#[derive(Debug)]
pub struct Handler;

impl EyreHandler for Handler {
    fn debug(&self, error: &(dyn Error + 'static), f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if f.alternate() {
            return fmt::Debug::fmt(error, f)
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
}

/// Installs the Foundry [eyre] and [panic](mod@std::panic) hooks as the global ones.
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

    let panic_section =
        "This is a bug. Consider reporting it at https://github.com/foundry-rs/foundry";
    let (panic_hook, debug_eyre_hook) =
        color_eyre::config::HookBuilder::default().panic_section(panic_section).into_hooks();
    panic_hook.install();
    let eyre_install_result = if std::env::var_os("FOUNDRY_DEBUG").is_some() {
        debug_eyre_hook.install()
    } else {
        eyre::set_hook(Box::new(|_| Box::new(Handler)))
    };
    if let Err(e) = eyre_install_result {
        debug!("failed to install eyre error hook: {e}");
    }
}
