use crate::executor::opts::EvmOpts;
use bytes::Bytes;

use foundry_config::{cache::StorageCachingConfig, Config, ResolvedRpcEndpoints};
use std::path::{Path, PathBuf};
use tracing::trace;

use super::util;

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
    pub rpc_endpoints: ResolvedRpcEndpoints,

    /// Project root
    pub root: PathBuf,

    /// Paths (directories) where file reading/writing is allowed
    pub allowed_paths: Vec<PathBuf>,

    /// How the evm was configured by the user
    pub evm_opts: EvmOpts,
}

// === impl CheatsConfig ===

impl CheatsConfig {
    /// Extracts the necessary settings from the Config
    pub fn new(config: &Config, evm_opts: &EvmOpts) -> Self {
        let mut allowed_paths = vec![config.__root.0.clone()];
        allowed_paths.extend(config.libs.clone());
        allowed_paths.extend(config.allow_paths.clone());

        let rpc_endpoints = config.rpc_endpoints.clone().resolved();
        trace!(?rpc_endpoints, "using resolved rpc endpoints");
        Self {
            ffi: evm_opts.ffi,
            rpc_storage_caching: config.rpc_storage_caching.clone(),
            rpc_endpoints,
            root: config.__root.0.clone(),
            allowed_paths,
            evm_opts: evm_opts.clone(),
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

    /// Returns the RPC to use
    ///
    /// If `url_or_alias` is a known alias in the `ResolvedRpcEndpoints` then it returns the
    /// corresponding URL of that alias. otherwise this assumes `url_or_alias` is itself a URL
    /// if it starts with a `http` or `ws` scheme
    ///
    /// # Errors
    ///
    ///  - Returns an error if `url_or_alias` is a known alias but references an unresolved env var.
    ///  - Returns an error if `url_or_alias` is not an alias but does not start with a `http` or
    ///    `scheme`
    pub fn get_rpc_url(&self, url_or_alias: impl Into<String>) -> Result<String, Bytes> {
        let url_or_alias = url_or_alias.into();
        match self.rpc_endpoints.get(&url_or_alias) {
            Some(Ok(url)) => Ok(url.clone()),
            Some(Err(err)) => {
                // try resolve again, by checking if env vars are now set
                if let Ok(url) = err.try_resolve() {
                    return Ok(url)
                }
                Err(util::encode_error(err))
            }
            None => {
                if !url_or_alias.starts_with("http") && !url_or_alias.starts_with("ws") {
                    Err(util::encode_error(format!("invalid rpc url {}", url_or_alias)))
                } else {
                    Ok(url_or_alias)
                }
            }
        }
    }
}
