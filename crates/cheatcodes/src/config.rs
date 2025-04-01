use super::Result;
use crate::Vm::Rpc;
use alloy_primitives::{map::AddressHashMap, U256};
use foundry_common::{fs::normalize_path, ContractsByArtifact};
use foundry_compilers::{utils::canonicalize, ArtifactId, ProjectPathsConfig};
use foundry_config::{
    cache::StorageCachingConfig, fs_permissions::FsAccessKind, Config, FsPermissions,
    ResolvedRpcEndpoint, ResolvedRpcEndpoints, RpcEndpoint, RpcEndpointUrl,
};
use foundry_evm_core::opts::EvmOpts;
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
    /// Absolute Path to broadcast dir i.e project_root/broadcast
    pub broadcast: PathBuf,
    /// Paths (directories) where file reading/writing is allowed
    pub allowed_paths: Vec<PathBuf>,
    /// How the evm was configured by the user
    pub evm_opts: EvmOpts,
    /// Address labels from config
    pub labels: AddressHashMap<String>,
    /// Artifacts which are guaranteed to be fresh (either recompiled or cached).
    /// If Some, `vm.getDeployedCode` invocations are validated to be in scope of this list.
    /// If None, no validation is performed.
    pub available_artifacts: Option<ContractsByArtifact>,
    /// Currently running artifact.
    pub running_artifact: Option<ArtifactId>,
    /// Whether to enable legacy (non-reverting) assertions.
    pub assertions_revert: bool,
    /// Optional seed for the RNG algorithm.
    pub seed: Option<U256>,
    /// Whether to allow `expectRevert` to work for internal calls.
    pub internal_expect_revert: bool,
    /// Mapping of chain aliases to chain data
    pub chains: HashMap<String, ChainData>,
    /// Mapping of chain IDs to their aliases
    pub chain_id_to_alias: HashMap<u64, String>,
}

/// Chain data for getChain cheatcodes
#[derive(Clone, Debug)]
pub struct ChainData {
    pub name: String,
    pub chain_id: u64,
    pub default_rpc_url: String, // Store default RPC URL
}

impl CheatsConfig {
    /// Extracts the necessary settings from the Config
    pub fn new(
        config: &Config,
        evm_opts: EvmOpts,
        available_artifacts: Option<ContractsByArtifact>,
        running_artifact: Option<ArtifactId>,
    ) -> Self {
        let mut allowed_paths = vec![config.root.clone()];
        allowed_paths.extend(config.libs.iter().cloned());
        allowed_paths.extend(config.allow_paths.iter().cloned());

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
            root: config.root.clone(),
            broadcast: config.root.clone().join(&config.broadcast),
            allowed_paths,
            evm_opts,
            labels: config.labels.clone(),
            available_artifacts,
            running_artifact,
            assertions_revert: config.assertions_revert,
            seed: config.fuzz.seed,
            internal_expect_revert: config.allow_internal_expect_revert,
            chains: HashMap::new(),
            chain_id_to_alias: HashMap::new(),
        }
    }

    /// Returns a new `CheatsConfig` configured with the given `Config` and `EvmOpts`.
    pub fn clone_with(&self, config: &Config, evm_opts: EvmOpts) -> Self {
        Self::new(config, evm_opts, self.available_artifacts.clone(), self.running_artifact.clone())
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
    pub fn rpc_endpoint(&self, url_or_alias: &str) -> Result<ResolvedRpcEndpoint> {
        if let Some(endpoint) = self.rpc_endpoints.get(url_or_alias) {
            Ok(endpoint.clone().try_resolve())
        } else {
            // check if it's a URL or a path to an existing file to an ipc socket
            if url_or_alias.starts_with("http") ||
                url_or_alias.starts_with("ws") ||
                // check for existing ipc file
                Path::new(url_or_alias).exists()
            {
                let url = RpcEndpointUrl::Env(url_or_alias.to_string());
                Ok(RpcEndpoint::new(url).resolve())
            } else {
                Err(fmt_err!("invalid rpc url: {url_or_alias}"))
            }
        }
    }
    /// Returns all the RPC urls and their alias.
    pub fn rpc_urls(&self) -> Result<Vec<Rpc>> {
        let mut urls = Vec::with_capacity(self.rpc_endpoints.len());
        for alias in self.rpc_endpoints.keys() {
            let url = self.rpc_endpoint(alias)?.url()?;
            urls.push(Rpc { key: alias.clone(), url });
        }
        Ok(urls)
    }

    /// Initialize default chain data (similar to initializeStdChains in Solidity)
    pub fn initialize_chain_data(&mut self) {
        if !self.chains.is_empty() {
            return; // Already initialized
        }

        // Use the same function to create chains
        let chains = create_default_chains();

        // Add all chains to the config
        for (alias, data) in chains {
            self.set_chain_with_default_rpc_url(&alias, data);
        }
    }

    /// Set chain with default RPC URL (similar to setChainWithDefaultRpcUrl in Solidity)
    pub fn set_chain_with_default_rpc_url(&mut self, alias: &str, data: ChainData) {
        // Store the default RPC URL is already stored in the data
        // No need to clone it separately

        // Add chain data
        self.set_chain_data(alias, data);
    }

    /// Set chain data for a specific alias
    pub fn set_chain_data(&mut self, alias: &str, data: ChainData) {
        // Remove old chain ID mapping if it exists
        if let Some(old_data) = self.chains.get(alias) {
            self.chain_id_to_alias.remove(&old_data.chain_id);
        }

        // Add new mappings
        self.chain_id_to_alias.insert(data.chain_id, alias.to_string());
        self.chains.insert(alias.to_string(), data);
    }

    /// Get chain data by alias
    pub fn get_chain_data_by_alias_non_mut(&self, alias: &str) -> Result<ChainData> {
        // Initialize chains if not already done
        if self.chains.is_empty() {
            // Create a temporary copy with initialized chains
            // This is inefficient but handles the edge case
            let temp_chains = create_default_chains();

            if let Some(data) = temp_chains.get(alias) {
                return Ok(data.clone());
            }
        } else {
            // Normal path - chains are initialized
            if let Some(data) = self.chains.get(alias) {
                return Ok(data.clone());
            }
        }

        // Chain not found in either case
        Err(fmt_err!("vm.getChain: Chain with alias \"{}\" not found", alias))
    }

    /// Get RPC URL for an alias
    pub fn get_rpc_url_non_mut(&self, alias: &str) -> Result<String> {
        // Try to get from config first
        match self.rpc_endpoint(alias) {
            Ok(endpoint) => Ok(endpoint.url()?),
            Err(_) => {
                // If not in config, try to get default URL
                let chain_data = self.get_chain_data_by_alias_non_mut(alias)?;
                Ok(chain_data.default_rpc_url)
            }
        }
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
            broadcast: Default::default(),
            allowed_paths: vec![],
            evm_opts: Default::default(),
            labels: Default::default(),
            available_artifacts: Default::default(),
            running_artifact: Default::default(),
            assertions_revert: true,
            seed: None,
            internal_expect_revert: false,
            chains: HashMap::new(),
            chain_id_to_alias: HashMap::new(),
        }
    }
}

// Helper function to set default chains
fn create_default_chains() -> HashMap<String, ChainData> {
    let mut chains = HashMap::new();

    // Define all chains in one place
    chains.insert(
        "anvil".to_string(),
        ChainData {
            name: "Anvil".to_string(),
            chain_id: 31337,
            default_rpc_url: "http://127.0.0.1:8545".to_string(),
        },
    );

    chains.insert(
        "mainnet".to_string(),
        ChainData {
            name: "Mainnet".to_string(),
            chain_id: 1,
            default_rpc_url: "https://eth.llamarpc.com".to_string(),
        },
    );

    chains.insert(
        "sepolia".to_string(),
        ChainData {
            name: "Sepolia".to_string(),
            chain_id: 11155111,
            default_rpc_url: "https://sepolia.infura.io/v3/b9794ad1ddf84dfb8c34d6bb5dca2001"
                .to_string(),
        },
    );

    chains.insert(
        "holesky".to_string(),
        ChainData {
            name: "Holesky".to_string(),
            chain_id: 17000,
            default_rpc_url: "https://rpc.holesky.ethpandaops.io".to_string(),
        },
    );

    chains.insert(
        "optimism".to_string(),
        ChainData {
            name: "Optimism".to_string(),
            chain_id: 10,
            default_rpc_url: "https://mainnet.optimism.io".to_string(),
        },
    );

    chains.insert(
        "optimism_sepolia".to_string(),
        ChainData {
            name: "Optimism Sepolia".to_string(),
            chain_id: 11155420,
            default_rpc_url: "https://sepolia.optimism.io".to_string(),
        },
    );

    chains.insert(
        "arbitrum_one".to_string(),
        ChainData {
            name: "Arbitrum One".to_string(),
            chain_id: 42161,
            default_rpc_url: "https://arb1.arbitrum.io/rpc".to_string(),
        },
    );

    chains.insert(
        "arbitrum_one_sepolia".to_string(),
        ChainData {
            name: "Arbitrum One Sepolia".to_string(),
            chain_id: 421614,
            default_rpc_url: "https://sepolia-rollup.arbitrum.io/rpc".to_string(),
        },
    );

    chains.insert(
        "arbitrum_nova".to_string(),
        ChainData {
            name: "Arbitrum Nova".to_string(),
            chain_id: 42170,
            default_rpc_url: "https://nova.arbitrum.io/rpc".to_string(),
        },
    );

    chains.insert(
        "polygon".to_string(),
        ChainData {
            name: "Polygon".to_string(),
            chain_id: 137,
            default_rpc_url: "https://polygon-rpc.com".to_string(),
        },
    );

    chains.insert(
        "polygon_amoy".to_string(),
        ChainData {
            name: "Polygon Amoy".to_string(),
            chain_id: 80002,
            default_rpc_url: "https://rpc-amoy.polygon.technology".to_string(),
        },
    );

    chains.insert(
        "avalanche".to_string(),
        ChainData {
            name: "Avalanche".to_string(),
            chain_id: 43114,
            default_rpc_url: "https://api.avax.network/ext/bc/C/rpc".to_string(),
        },
    );

    chains.insert(
        "avalanche_fuji".to_string(),
        ChainData {
            name: "Avalanche Fuji".to_string(),
            chain_id: 43113,
            default_rpc_url: "https://api.avax-test.network/ext/bc/C/rpc".to_string(),
        },
    );

    chains.insert(
        "bnb_smart_chain".to_string(),
        ChainData {
            name: "BNB Smart Chain".to_string(),
            chain_id: 56,
            default_rpc_url: "https://bsc-dataseed1.binance.org".to_string(),
        },
    );

    chains.insert(
        "bnb_smart_chain_testnet".to_string(),
        ChainData {
            name: "BNB Smart Chain Testnet".to_string(),
            chain_id: 97,
            default_rpc_url: "https://rpc.ankr.com/bsc_testnet_chapel".to_string(),
        },
    );

    chains.insert(
        "gnosis_chain".to_string(),
        ChainData {
            name: "Gnosis Chain".to_string(),
            chain_id: 100,
            default_rpc_url: "https://rpc.gnosischain.com".to_string(),
        },
    );

    chains.insert(
        "moonbeam".to_string(),
        ChainData {
            name: "Moonbeam".to_string(),
            chain_id: 1284,
            default_rpc_url: "https://rpc.api.moonbeam.network".to_string(),
        },
    );

    chains.insert(
        "moonriver".to_string(),
        ChainData {
            name: "Moonriver".to_string(),
            chain_id: 1285,
            default_rpc_url: "https://rpc.api.moonriver.moonbeam.network".to_string(),
        },
    );

    chains.insert(
        "moonbase".to_string(),
        ChainData {
            name: "Moonbase".to_string(),
            chain_id: 1287,
            default_rpc_url: "https://rpc.testnet.moonbeam.network".to_string(),
        },
    );

    chains.insert(
        "base_sepolia".to_string(),
        ChainData {
            name: "Base Sepolia".to_string(),
            chain_id: 84532,
            default_rpc_url: "https://sepolia.base.org".to_string(),
        },
    );

    chains.insert(
        "base".to_string(),
        ChainData {
            name: "Base".to_string(),
            chain_id: 8453,
            default_rpc_url: "https://mainnet.base.org".to_string(),
        },
    );

    chains.insert(
        "blast_sepolia".to_string(),
        ChainData {
            name: "Blast Sepolia".to_string(),
            chain_id: 168587773,
            default_rpc_url: "https://sepolia.blast.io".to_string(),
        },
    );

    chains.insert(
        "blast".to_string(),
        ChainData {
            name: "Blast".to_string(),
            chain_id: 81457,
            default_rpc_url: "https://rpc.blast.io".to_string(),
        },
    );

    chains.insert(
        "fantom_opera".to_string(),
        ChainData {
            name: "Fantom Opera".to_string(),
            chain_id: 250,
            default_rpc_url: "https://rpc.ankr.com/fantom/".to_string(),
        },
    );

    chains.insert(
        "fantom_opera_testnet".to_string(),
        ChainData {
            name: "Fantom Opera Testnet".to_string(),
            chain_id: 4002,
            default_rpc_url: "https://rpc.ankr.com/fantom_testnet/".to_string(),
        },
    );

    chains.insert(
        "fraxtal".to_string(),
        ChainData {
            name: "Fraxtal".to_string(),
            chain_id: 252,
            default_rpc_url: "https://rpc.frax.com".to_string(),
        },
    );

    chains.insert(
        "fraxtal_testnet".to_string(),
        ChainData {
            name: "Fraxtal Testnet".to_string(),
            chain_id: 2522,
            default_rpc_url: "https://rpc.testnet.frax.com".to_string(),
        },
    );

    chains.insert(
        "berachain_bartio_testnet".to_string(),
        ChainData {
            name: "Berachain bArtio Testnet".to_string(),
            chain_id: 80084,
            default_rpc_url: "https://bartio.rpc.berachain.com".to_string(),
        },
    );

    chains.insert(
        "flare".to_string(),
        ChainData {
            name: "Flare".to_string(),
            chain_id: 14,
            default_rpc_url: "https://flare-api.flare.network/ext/C/rpc".to_string(),
        },
    );

    chains.insert(
        "flare_coston2".to_string(),
        ChainData {
            name: "Flare Coston2".to_string(),
            chain_id: 114,
            default_rpc_url: "https://coston2-api.flare.network/ext/C/rpc".to_string(),
        },
    );

    chains.insert(
        "mode".to_string(),
        ChainData {
            name: "Mode".to_string(),
            chain_id: 34443,
            default_rpc_url: "https://mode.drpc.org".to_string(),
        },
    );

    chains.insert(
        "mode_sepolia".to_string(),
        ChainData {
            name: "Mode Sepolia".to_string(),
            chain_id: 919,
            default_rpc_url: "https://sepolia.mode.network".to_string(),
        },
    );

    chains.insert(
        "zora".to_string(),
        ChainData {
            name: "Zora".to_string(),
            chain_id: 7777777,
            default_rpc_url: "https://zora.drpc.org".to_string(),
        },
    );

    chains.insert(
        "zora_sepolia".to_string(),
        ChainData {
            name: "Zora Sepolia".to_string(),
            chain_id: 999999999,
            default_rpc_url: "https://sepolia.rpc.zora.energy".to_string(),
        },
    );

    chains.insert(
        "race".to_string(),
        ChainData {
            name: "Race".to_string(),
            chain_id: 6805,
            default_rpc_url: "https://racemainnet.io".to_string(),
        },
    );

    chains.insert(
        "race_sepolia".to_string(),
        ChainData {
            name: "Race Sepolia".to_string(),
            chain_id: 6806,
            default_rpc_url: "https://racemainnet.io".to_string(),
        },
    );

    chains.insert(
        "metal".to_string(),
        ChainData {
            name: "Metal".to_string(),
            chain_id: 1750,
            default_rpc_url: "https://metall2.drpc.org".to_string(),
        },
    );

    chains.insert(
        "metal_sepolia".to_string(),
        ChainData {
            name: "Metal Sepolia".to_string(),
            chain_id: 1740,
            default_rpc_url: "https://testnet.rpc.metall2.com".to_string(),
        },
    );

    chains.insert(
        "binary".to_string(),
        ChainData {
            name: "Binary".to_string(),
            chain_id: 624,
            default_rpc_url: "https://rpc.zero.thebinaryholdings.com".to_string(),
        },
    );

    chains.insert(
        "binary_sepolia".to_string(),
        ChainData {
            name: "Binary Sepolia".to_string(),
            chain_id: 625,
            default_rpc_url: "https://rpc.zero.thebinaryholdings.com".to_string(),
        },
    );

    chains.insert(
        "orderly".to_string(),
        ChainData {
            name: "Orderly".to_string(),
            chain_id: 291,
            default_rpc_url: "https://rpc.orderly.network".to_string(),
        },
    );

    chains.insert(
        "orderly_sepolia".to_string(),
        ChainData {
            name: "Orderly Sepolia".to_string(),
            chain_id: 4460,
            default_rpc_url: "https://testnet-rpc.orderly.org".to_string(),
        },
    );

    chains
}

#[cfg(test)]
mod tests {
    use super::*;
    use foundry_config::fs_permissions::PathPermission;

    fn config(root: &str, fs_permissions: FsPermissions) -> CheatsConfig {
        CheatsConfig::new(
            &Config { root: root.into(), fs_permissions, ..Default::default() },
            Default::default(),
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
