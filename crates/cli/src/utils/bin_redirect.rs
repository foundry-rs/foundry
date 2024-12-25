use std::path::PathBuf;

/// Loads config and checks if there is a binary remapping for the current binary.
/// If there is a remapping, returns the path to the binary that should be executed.
/// Returns `None` if the binary is not remapped _or_ if the current binary is not found in the
/// config.
pub fn should_redirect_to() -> Option<PathBuf> {
    let current_exe = std::env::current_exe().ok()?;
    let binary_name = current_exe.file_stem()?.to_str()?;
    let config = foundry_config::Config::load();
    config.binary_mappings().redirect_for(binary_name).cloned()
}

/// Launches the `to` binary with the same arguments as the current binary.
/// E.g. if user runs `forge build --arg1 --arg2`, and `to` is `/path/to/custom/forge`, then
/// this function will run `/path/to/custom/forge build --arg1 --arg2`.
pub fn redirect_execution(to: PathBuf) -> eyre::Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let status = std::process::Command::new(to)
        .args(args)
        .status()
        .map_err(|e| eyre::eyre!("Failed to run command: {}", e))?;
    if !status.success() {
        eyre::bail!("Command failed with status: {}", status);
    }
    Ok(())
}
