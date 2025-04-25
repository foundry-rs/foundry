use foundry_compilers::{multi::MultiCompilerLanguage, ProjectPathsConfig};
use serde::{Deserialize, Serialize};

use crate::{Config, SolcReq};

/// Filename for resolc cache
pub const RESOLC_SOLIDITY_FILES_CACHE_FILENAME: &str = "resolc-solidity-files-cache.json";

/// Directory for resolc artifacts
pub const RESOLC_ARTIFACTS_DIR: &str = "resolc-out";

pub const CONTRACT_SIZE_LIMIT: usize = 250_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
/// Resolc Config
pub struct ResolcConfig {
    /// Enable compilation using resolc
    pub resolc_compile: bool,

    /// The resolc compiler
    pub resolc: Option<SolcReq>,
}

impl ResolcConfig {
    /// Returns the `ProjectPathsConfig` sub set of the config.
    pub fn project_paths(config: &Config) -> ProjectPathsConfig<MultiCompilerLanguage> {
        let builder = ProjectPathsConfig::builder()
            .cache(config.cache_path.join(RESOLC_SOLIDITY_FILES_CACHE_FILENAME))
            .sources(&config.src)
            .tests(&config.test)
            .scripts(&config.script)
            .artifacts(config.root.join(RESOLC_ARTIFACTS_DIR))
            .libs(config.libs.iter())
            .remappings(config.get_all_remappings())
            .allowed_path(&config.root)
            .allowed_paths(&config.libs)
            .allowed_paths(&config.allow_paths)
            .include_paths(&config.include_paths);

        builder.build_with_root(&config.root)
    }
}
