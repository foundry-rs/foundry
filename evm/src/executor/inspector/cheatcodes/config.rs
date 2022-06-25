use crate::executor::opts::EvmOpts;
use foundry_config::{cache::StorageCachingConfig, Config, RpcEndpoints};
use std::path::{Path, PathBuf};

/// Additional, configurable context the `Cheatcodes` inspector has access to
///
/// This is essentially a subset of various `Config` settings `Cheatcodes` needs to know.
/// Since each test gets its own cheatcode inspector, but these values here are expected to be
/// constant for all test runs, everything is `Arc'ed` here to avoid unnecessary, expensive clones.
#[derive(Debug, Clone, Default)]
pub struct CheatsConfig {
    pub ffi: bool,
    /// RPC storage caching settings determines what chains and endpoints to cache
    pub rpc_storage_caching: StorageCachingConfig,
    /// All known endpoints and their aliases
    pub rpc_endpoints: RpcEndpoints,

    /// Project root
    pub root: PathBuf,

    /// Paths (directories) where file reading/writing is allowed
    pub allowed_paths: Vec<PathBuf>,
}

// === impl CheatsConfig ===

impl CheatsConfig {
    /// Extracts the necessary settings from the Config
    pub fn new(config: &Config, evm_opts: &EvmOpts) -> Self {
        let mut allowed_paths = vec![config.__root.0.clone()];
        allowed_paths.extend(config.libs.clone());
        allowed_paths.extend(config.allow_paths.clone());

        Self {
            ffi: evm_opts.ffi,
            rpc_storage_caching: config.rpc_storage_caching.clone(),
            rpc_endpoints: config.rpc_endpoints.clone(),
            root: config.__root.0.clone(),
            allowed_paths,
        }
    }

    pub fn is_path_allowed(&self, path: impl AsRef<Path>) -> bool {
        return self.allowed_paths.iter().any(|allowed_path| path.as_ref().starts_with(allowed_path))
    }

    pub fn ensure_path_allowed(&self, path: impl AsRef<Path>) -> Result<(), String> {
        if !self.is_path_allowed(path) {
            return Err("Path is not allowed.".to_string())
        }

        Ok(())
    }
}
