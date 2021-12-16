use eyre::{ContextCompat, WrapErr};
use std::{fs, path::PathBuf};

const APP_NAME: &str = "forge-shell";

pub fn forge_shell_dir() -> eyre::Result<PathBuf> {
    let path = dirs_next::data_dir().wrap_err("Failed to find data directory")?;
    let path = path.join(APP_NAME);
    fs::create_dir_all(&path).wrap_err("Failed to create data directory")?;
    Ok(path)
}

pub fn project_dir() -> eyre::Result<PathBuf> {
    let path = forge_shell_dir()?.join("project");
    fs::create_dir_all(&path).wrap_err("Failed to create project directory")?;
    Ok(path)
}

pub fn history_path() -> eyre::Result<PathBuf> {
    let path = forge_shell_dir()?;
    Ok(path.join("history"))
}

pub fn data_dir() -> eyre::Result<PathBuf> {
    let path = forge_shell_dir()?.join("data");
    fs::create_dir_all(&path).wrap_err("Failed to create module directory")?;
    Ok(path)
}

pub fn cache_dir() -> eyre::Result<PathBuf> {
    let path = dirs_next::cache_dir().wrap_err("Failed to find cache directory")?;
    let path = path.join(APP_NAME);
    fs::create_dir_all(&path).wrap_err("Failed to create cache directory")?;
    Ok(path)
}
