use crate::executor::opts::EvmOpts;
use bytes::Bytes;

use crate::error;
use ethers::solc::{utils::canonicalize, ProjectPathsConfig};
use foundry_common::fs::normalize_path;
use foundry_config::{
    cache::StorageCachingConfig, fs_permissions::FsAccessKind, Config, FsPermissions,
    ResolvedRpcEndpoints,
};
use std::path::{Path, PathBuf};
use tracing::trace;

/// Additional, configurable context the `Cheatcodes` inspector has access to
///
/// This is essentially a subset of various `Config` settings `Cheatcodes` needs to know.
#[derive(Debug, Clone)]
pub struct CheatsConfig {
    pub ffi: bool,
    /// RPC storage caching settings determines what chains and endpoints to cache
    pub rpc_storage_caching: StorageCachingConfig,
    /// All known endpoints and their aliases
    pub rpc_endpoints: ResolvedRpcEndpoints,
    /// Project's paths as configured
    pub paths: ProjectPathsConfig,
    /// Filesystem permissions for cheatcodes like `writeFile`, `readFile`
    pub fs_permissions: FsPermissions,
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
            paths: config.project_paths(),
            fs_permissions: config.fs_permissions.clone().joined(&config.__root),
            root: config.__root.0.clone(),
            allowed_paths,
            evm_opts: evm_opts.clone(),
        }
    }

    /// Attempts to canonicalize (see [std::fs::canonicalize]) the path.
    ///
    /// Canonicalization fails for non-existing paths, in which case we just normalize the path.
    pub fn normalized_path(&self, path: impl AsRef<Path>) -> PathBuf {
        let path = self.root.join(path);
        canonicalize(&path).unwrap_or_else(|_| normalize_path(&path))
    }

    /// Returns true if the given path is allowed, if any path `allowed_paths` is an ancestor of the
    /// path
    ///
    /// We only allow paths that are inside  allowed paths. To prevent path traversal
    /// ("../../etc/passwd") we canonicalize/normalize the path first. We always join with the
    /// configured root directory.
    pub fn is_path_allowed(&self, path: impl AsRef<Path>, kind: FsAccessKind) -> bool {
        self.is_normalized_path_allowed(&self.normalized_path(path), kind)
    }

    fn is_normalized_path_allowed(&self, path: &Path, kind: FsAccessKind) -> bool {
        self.fs_permissions.is_path_allowed(path, kind)
    }

    /// Returns an error if no access is granted to access `path`, See also [Self::is_path_allowed]
    ///
    /// Returns the normalized version of `path`, see [`Self::normalized_path`]
    pub fn ensure_path_allowed(
        &self,
        path: impl AsRef<Path>,
        kind: FsAccessKind,
    ) -> Result<PathBuf, String> {
        let path = path.as_ref();
        let normalized = self.normalized_path(path);
        if !self.is_normalized_path_allowed(&normalized, kind) {
            return Err(format!(
                "The path {path:?} is not allowed to be accessed for {kind} operations."
            ))
        }

        Ok(normalized)
    }

    /// Returns true if the given `path` is the project's foundry.toml file
    ///
    /// Note: this should be called with normalized path
    pub fn is_foundry_toml(&self, path: impl AsRef<Path>) -> bool {
        // path methods that do not access the filesystem are such as [`Path::starts_with`], are
        // case-sensitive no matter the platform or filesystem. to make this case-sensitive
        // we convert the underlying `OssStr` to lowercase checking that `path` and
        // `foundry.toml` are the same file by comparing the FD, because it may not exist
        let foundry_toml = self.root.join(Config::FILE_NAME);
        Path::new(&foundry_toml.to_string_lossy().to_lowercase())
            .starts_with(Path::new(&path.as_ref().to_string_lossy().to_lowercase()))
    }

    /// Same as [`Self::is_foundry_toml`] but returns an `Err` if [`Self::is_foundry_toml`] returns
    /// true
    pub fn ensure_not_foundry_toml(&self, path: impl AsRef<Path>) -> Result<(), String> {
        if self.is_foundry_toml(path) {
            return Err("Access to foundry.toml is not allowed.".to_string())
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
                Err(error::encode_error(err))
            }
            None => {
                if !url_or_alias.starts_with("http") && !url_or_alias.starts_with("ws") {
                    Err(error::encode_error(format!("invalid rpc url {url_or_alias}")))
                } else {
                    Ok(url_or_alias)
                }
            }
        }
    }
}

impl Default for CheatsConfig {
    fn default() -> Self {
        Self {
            ffi: false,
            rpc_storage_caching: Default::default(),
            rpc_endpoints: Default::default(),
            paths: ProjectPathsConfig::builder().build_with_root("./"),
            fs_permissions: Default::default(),
            root: Default::default(),
            allowed_paths: vec![],
            evm_opts: Default::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use foundry_config::fs_permissions::PathPermission;

    fn config(root: &str, fs_permissions: FsPermissions) -> CheatsConfig {
        CheatsConfig::new(
            &Config { __root: PathBuf::from(root).into(), fs_permissions, ..Default::default() },
            &Default::default(),
        )
    }

    #[test]
    fn test_allowed_paths() {
        let root = "/my/project/root/";
        let config = config(root, FsPermissions::new(vec![PathPermission::read_write("./")]));

        assert!(config.ensure_path_allowed("./t.txt", FsAccessKind::Read).is_ok());
        assert!(config.ensure_path_allowed("./t.txt", FsAccessKind::Write).is_ok());
        assert!(config.ensure_path_allowed("../root/t.txt", FsAccessKind::Read).is_ok());
        assert!(config.ensure_path_allowed("../root/t.txt", FsAccessKind::Write).is_ok());
        assert!(config.ensure_path_allowed("../../root/t.txt", FsAccessKind::Read).is_err());
        assert!(config.ensure_path_allowed("../../root/t.txt", FsAccessKind::Write).is_err());
    }

    #[test]
    fn test_is_foundry_toml() {
        let root = "/my/project/root/";
        let config = config(root, FsPermissions::new(vec![PathPermission::read_write("./")]));

        let f = format!("{root}foundry.toml");
        assert!(config.is_foundry_toml(f));

        let f = format!("{root}Foundry.toml");
        assert!(config.is_foundry_toml(f));

        let f = format!("{root}lib/other/foundry.toml");
        assert!(!config.is_foundry_toml(f));
    }
}
