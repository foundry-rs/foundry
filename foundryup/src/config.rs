use crate::utils;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Config {
    pub foundry_dir: PathBuf,
    pub foundryup_dir: PathBuf,
    pub download_dir: PathBuf,
    pub silent: bool,
}

// === impl Config ===

impl Config {
    pub fn new() -> eyre::Result<Self> {
        let foundry_dir = utils::foundry_home()?;
        let foundryup_dir = utils::foundryup_dir()?;
        let download_dir = foundryup_dir.join("downloads");

        Ok(Self { foundry_dir, foundryup_dir, download_dir, silent: false })
    }
}
