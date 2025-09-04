use foundry_compilers::{
    error::SolcError, multi::MultiCompilerLanguage, resolc::ResolcSettings, solc::SolcSettings,
    ProjectPathsConfig,
};
use serde::{Deserialize, Serialize};

use crate::{Config, SolcReq};

/// Filename for resolc cache
pub const RESOLC_SOLIDITY_FILES_CACHE_FILENAME: &str = "resolc-solidity-files-cache.json";

/// Name of the subdirectory for solc artifacts in dual compilation mode
pub const SOLC_ARTIFACTS_SUBDIR: &str = "solc";

pub const CONTRACT_SIZE_LIMIT: usize = 250_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Default, Deserialize)]
/// Resolc Config
pub struct ResolcConfig {
    /// Enable compilation using resolc
    pub resolc_compile: bool,

    /// Enable PVM mode at startup (independent of compilation)
    pub resolc_startup: bool,

    /// The resolc compiler
    pub resolc: Option<SolcReq>,

    /// The optimization mode string for resolc
    pub optimizer_mode: Option<char>,

    /// The emulated EVM linear heap memory static buffer size in bytes
    pub heap_size: Option<u32>,

    /// The contracts total stack size in bytes
    pub stack_size: Option<u32>,

    /// Generate source based debug information in the output code file
    pub debug_information: Option<bool>,
}

impl ResolcConfig {
    /// Returns the `ProjectPathsConfig` sub set of the config.
    pub fn project_paths(config: &Config) -> ProjectPathsConfig<MultiCompilerLanguage> {
        let builder = ProjectPathsConfig::builder()
            .cache(config.cache_path.join(RESOLC_SOLIDITY_FILES_CACHE_FILENAME))
            .sources(&config.src)
            .tests(&config.test)
            .scripts(&config.script)
            .libs(config.libs.iter())
            .remappings(config.get_all_remappings())
            .allowed_path(&config.root)
            .allowed_paths(&config.libs)
            .allowed_paths(&config.allow_paths)
            .include_paths(&config.include_paths)
            .artifacts(&config.out);

        builder.build_with_root(&config.root)
    }

    pub fn resolc_settings(config: &Config) -> Result<SolcSettings, SolcError> {
        config.solc_settings().map(|mut s| {
            s.extra_settings = ResolcSettings::new(
                config.resolc.optimizer_mode,
                config.resolc.heap_size,
                config.resolc.stack_size,
                config.resolc.debug_information,
            );
            s
        })
    }
}
