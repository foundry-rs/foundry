//! This is adapted from <https://github.com/rust-lang/foundryup/tree/master/src/cli/self_update>

use crate::{
    config::Config, errors::FoundryupError, location::get_available_foundryup_version, utils,
    utils::ExitCode,
};
use std::{
    env::consts::EXE_SUFFIX,
    path::{PathBuf, MAIN_SEPARATOR},
};
use tracing::info;

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
pub(crate) fn update(_config: &Config) -> eyre::Result<ExitCode> {
    Ok(0.into())
}

pub(crate) fn uninstall() -> eyre::Result<ExitCode> {
    Ok(0.into())
}

pub(crate) async fn prepare_update() -> eyre::Result<Option<PathBuf>> {
    let foundry_home = utils::foundry_home()?;
    let foundryup_path =
        foundry_home.join(&format!("bin{}foundryup{}", MAIN_SEPARATOR, EXE_SUFFIX));
    let setup_path =
        foundry_home.join(&format!("bin{}foundryup-init{}", MAIN_SEPARATOR, EXE_SUFFIX));

    if !foundryup_path.exists() {
        return Err(FoundryupError::FoundryupNotInstalled { p: foundry_home }.into())
    }

    if setup_path.exists() {
        utils::remove_file("setup", &setup_path)?;
    }

    let release = get_available_foundryup_version().await?;
    let current_version = env!("CARGO_PKG_VERSION");

    // If up-to-date
    if release.version == current_version {
        return Ok(None)
    }

    // Download new version
    info!("downloading foundryup self-update");
    utils::download_file(&release.tarball_url, &setup_path).await?;

    // Mark as executable
    utils::make_executable(&setup_path)?;

    Ok(Some(setup_path))
}
