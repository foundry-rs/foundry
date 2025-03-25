use foundry_compilers::{multi::MultiCompilerLanguage, ProjectPathsConfig};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::Config;

/// Filename for Revive cache
pub const REVIVE_SOLIDITY_FILES_CACHE_FILENAME: &str = "revive-solidity-files-cache.json";

/// Directory for Revive artifacts
pub const REVIVE_ARTIFACTS_DIR: &str = "revive-out";

pub const CONTRACT_SIZE_LIMIT: usize = 250_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
/// Revive Config
pub struct ReviveConfig {
    /// Enable compilation using revive
    pub revive_compile: bool,
    /// The revive path
    pub revive_path: Option<PathBuf>,
    /// solc path to use along the revive compiler
    pub solc_path: Option<PathBuf>,
}

impl ReviveConfig {
    /// Returns the `ProjectPathsConfig` sub set of the config.
    pub fn project_paths(config: &Config) -> ProjectPathsConfig<MultiCompilerLanguage> {
        let builder = ProjectPathsConfig::builder()
            .cache(config.cache_path.join(REVIVE_SOLIDITY_FILES_CACHE_FILENAME))
            .sources(&config.src)
            .tests(&config.test)
            .scripts(&config.script)
            .artifacts(config.root.join(REVIVE_ARTIFACTS_DIR))
            .libs(config.libs.iter())
            .remappings(config.get_all_remappings())
            .allowed_path(&config.root)
            .allowed_paths(&config.libs)
            .allowed_paths(&config.allow_paths)
            .include_paths(&config.include_paths);

        builder.build_with_root(&config.root)
    }
}
