use super::Result;
use crate::{script::ScriptWallets, Vm::Rpc};
use alloy_primitives::{Address, U256};
use foundry_common::{fs::normalize_path, ContractsByArtifact};
use foundry_compilers::{utils::canonicalize, ProjectPathsConfig};
use foundry_config::{
    cache::StorageCachingConfig, fs_permissions::FsAccessKind, Config, FsPermissions,
    ResolvedRpcEndpoints,
};
use foundry_evm_core::opts::EvmOpts;
use semver::Version;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::Duration,
};

/// Additional, configurable context the `Cheatcodes` inspector has access to
///
/// This is essentially a subset of various `Config` settings `Cheatcodes` needs to know.
#[derive(Clone, Debug)]
pub struct CheatsConfig {
    /// Whether the FFI cheatcode is enabled.
    pub ffi: bool,
    /// Use the create 2 factory in all cases including tests and non-broadcasting scripts.
    pub always_use_create_2_factory: bool,
    /// Sets a timeout for vm.prompt cheatcodes
    pub prompt_timeout: Duration,
    /// RPC storage caching settings determines what chains and endpoints to cache
    pub rpc_storage_caching: StorageCachingConfig,
    /// Disables storage caching entirely.
    pub no_storage_caching: bool,
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
    /// Address labels from config
    pub labels: HashMap<Address, String>,
    /// Script wallets
    pub script_wallets: Option<ScriptWallets>,
    /// Artifacts which are guaranteed to be fresh (either recompiled or cached).
    /// If Some, `vm.getDeployedCode` invocations are validated to be in scope of this list.
    /// If None, no validation is performed.
    pub available_artifacts: Option<ContractsByArtifact>,
    /// Version of the script/test contract which is currently running.
    pub running_version: Option<Version>,
    /// Whether to enable legacy (non-reverting) assertions.
    pub assertions_revert: bool,
    /// Optional seed for the RNG algorithm.
    pub seed: Option<U256>,
}

impl CheatsConfig {
    /// Extracts the necessary settings from the Config
    pub fn new(
        config: &Config,
        evm_opts: EvmOpts,
        available_artifacts: Option<ContractsByArtifact>,
        script_wallets: Option<ScriptWallets>,
        running_version: Option<Version>,
    ) -> Self {
        let mut allowed_paths = vec![config.root.0.clone()];
        allowed_paths.extend(config.libs.clone());
        allowed_paths.extend(config.allow_paths.clone());

        let rpc_endpoints = config.rpc_endpoints.clone().resolved();
        trace!(?rpc_endpoints, "using resolved rpc endpoints");

        // If user explicitly disabled safety checks, do not set available_artifacts
        let available_artifacts =
            if config.unchecked_cheatcode_artifacts { None } else { available_artifacts };

        Self {
            ffi: evm_opts.ffi,
            always_use_create_2_factory: evm_opts.always_use_create_2_factory,
            prompt_timeout: Duration::from_secs(config.prompt_timeout),
            rpc_storage_caching: config.rpc_storage_caching.clone(),
            no_storage_caching: config.no_storage_caching,
            rpc_endpoints,
            paths: config.project_paths(),
            fs_permissions: config.fs_permissions.clone().joined(config.root.as_ref()),
            root: config.root.0.clone(),
            allowed_paths,
            evm_opts,
            labels: config.labels.clone(),
            script_wallets,
            available_artifacts,
            running_version,
            assertions_revert: config.assertions_revert,
            seed: config.fuzz.seed,
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
    /// Returns the normalized version of `path`, see [`CheatsConfig::normalized_path`]
    pub fn ensure_path_allowed(
        &self,
        path: impl AsRef<Path>,
        kind: FsAccessKind,
    ) -> Result<PathBuf> {
        let path = path.as_ref();
        let normalized = self.normalized_path(path);
        ensure!(
            self.is_normalized_path_allowed(&normalized, kind),
            "the path {} is not allowed to be accessed for {kind} operations",
            normalized.strip_prefix(&self.root).unwrap_or(path).display()
        );
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
    pub fn ensure_not_foundry_toml(&self, path: impl AsRef<Path>) -> Result<()> {
        ensure!(!self.is_foundry_toml(path), "access to `foundry.toml` is not allowed");
        Ok(())
    }

    /// Returns the RPC to use
    ///
    /// If `url_or_alias` is a known alias in the `ResolvedRpcEndpoints` then it returns the
    /// corresponding URL of that alias. otherwise this assumes `url_or_alias` is itself a URL
    /// if it starts with a `http` or `ws` scheme.
    ///
    /// If the url is a path to an existing file, it is also considered a valid RPC URL, IPC path.
    ///
    /// # Errors
    ///
    ///  - Returns an error if `url_or_alias` is a known alias but references an unresolved env var.
    ///  - Returns an error if `url_or_alias` is not an alias but does not start with a `http` or
    ///    `ws` `scheme` and is not a path to an existing file
    pub fn rpc_url(&self, url_or_alias: &str) -> Result<String> {
        match self.rpc_endpoints.get(url_or_alias) {
            Some(Ok(url)) => Ok(url.clone()),
            Some(Err(err)) => {
                // try resolve again, by checking if env vars are now set
                err.try_resolve().map_err(Into::into)
            }
            None => {
                // check if it's a URL or a path to an existing file to an ipc socket
                if url_or_alias.starts_with("http") ||
                    url_or_alias.starts_with("ws") ||
                    // check for existing ipc file
                    Path::new(url_or_alias).exists()
                {
                    Ok(url_or_alias.into())
                } else {
                    Err(fmt_err!("invalid rpc url: {url_or_alias}"))
                }
            }
        }
    }

    /// Returns all the RPC urls and their alias.
    pub fn rpc_urls(&self) -> Result<Vec<Rpc>> {
        let mut urls = Vec::with_capacity(self.rpc_endpoints.len());
        for alias in self.rpc_endpoints.keys() {
            let url = self.rpc_url(alias)?;
            urls.push(Rpc { key: alias.clone(), url });
        }
        Ok(urls)
    }
}

impl Default for CheatsConfig {
    fn default() -> Self {
        Self {
            ffi: false,
            always_use_create_2_factory: false,
            prompt_timeout: Duration::from_secs(120),
            rpc_storage_caching: Default::default(),
            no_storage_caching: false,
            rpc_endpoints: Default::default(),
            paths: ProjectPathsConfig::builder().build_with_root("./"),
            fs_permissions: Default::default(),
            root: Default::default(),
            allowed_paths: vec![],
            evm_opts: Default::default(),
            labels: Default::default(),
            script_wallets: None,
            available_artifacts: Default::default(),
            running_version: Default::default(),
            assertions_revert: true,
            seed: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use foundry_config::fs_permissions::PathPermission;

    fn config(root: &str, fs_permissions: FsPermissions) -> CheatsConfig {
        CheatsConfig::new(
            &Config { root: PathBuf::from(root).into(), fs_permissions, ..Default::default() },
            Default::default(),
            None,
            None,
            None,
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
