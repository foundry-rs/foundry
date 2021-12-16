//! forge config

use eyre::{Context, ContextCompat};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default)]
// Serialize, Deserialize
pub struct Config {}

impl Config {
    /// Returns the path to the forge toml file at `~/forge.toml`
    pub fn path() -> eyre::Result<PathBuf> {
        let path = dirs_next::config_dir().wrap_err_with(|| "Failed to detect config directory")?;
        let path = path.join("forge.toml");
        Ok(path)
    }

    pub fn load_or_default() -> eyre::Result<Config> {
        let path = Config::path()?;
        if path.exists() {
            Config::load_from(&path)
        } else {
            Ok(Config::default())
        }
    }

    pub fn load_from(path: impl AsRef<Path>) -> eyre::Result<Config> {
        let _config = std::fs::read(&path).wrap_err("Failed to read config file")?;

        // let config = toml::from_slice(&config)?;
        // Ok(config)
        todo!()
    }
}
