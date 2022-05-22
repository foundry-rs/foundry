use crate::config::Config;
use std::process::ExitCode;

/// Self update downloads foundryup to `FOUNDRY_HOME`/bin/foundryup-init
/// and runs it.
///
/// It does a few things to accommodate self-delete problems on windows:
///
/// foundryup-init is run in two stages, first with `--self-upgrade`,
/// which displays update messages and asks for confirmations, etc;
/// then with `--self-replace`, which replaces the foundryup binary and
/// hardlinks. The last step is done without waiting for confirmation
/// on windows so that the running exe can be deleted.
///
/// Because it's again difficult for foundryup-init to delete itself
/// (and on windows this process will not be running to do it),
/// foundryup-init is stored in `FOUNDRY_HOME`/bin, and then deleted next
/// time foundryup runs.
pub(crate) fn update(config: &Config) -> eyre::Result<ExitCode> {
    Ok(0.into())
}

pub(crate) fn uninstall() -> eyre::Result<ExitCode> {
    Ok(0.into())
}
