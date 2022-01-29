//! foundry configuration.
use std::{
    borrow::Cow,
    path::{Path, PathBuf},
};

use figment::{
    providers::{Env, Format, Serialized, Toml},
    value::{Dict, Map},
    Error, Figment, Metadata, Profile, Provider,
};
// reexport so cli types can implement `figment::Provider` to easily merge compiler arguments
pub use figment;
use semver::Version;
use serde::{Deserialize, Serialize};

use ethers_core::types::{Address, U256};
use ethers_solc::{
    artifacts::{Optimizer, Settings},
    error::SolcError,
    remappings::{RelativeRemapping, Remapping},
    EvmVersion, Project, ProjectPathsConfig, SolcConfig,
};
use figment::providers::Data;
use inflector::Inflector;

// Macros useful for creating a figment.
mod macros;

// Utilities for making it easier to handle tests.
pub mod utils;
pub use crate::utils::*;

/// Foundry configuration
///
/// # Defaults
///
/// All configuration values have a default, documented in the [fields](#fields)
/// section below. [`Config::default()`] returns the default values for
/// the default profile while [`Config::with_root()`] returns the values based on the given
/// directory. [`Config::load()`] starts with the default profile and merges various providers into
/// the config, same for [`Config::load_with_root()`], but there the default values are determined
/// by [`Config::with_root()`]
///
/// # Provider Details
///
/// `Config` is a Figment [`Provider`] with the following characteristics:
///
///   * **Profile**
///
///     The profile is set to the value of the `profile` field.
///
///   * **Metadata**
///
///     This provider is named `Foundry Config`. It does not specify a
///     [`Source`](figment::Source) and uses default interpolation.
///
///   * **Data**
///
///     The data emitted by this provider are the keys and values corresponding
///     to the fields and values of the structure. The dictionary is emitted to
///     the "default" meta-profile.
///
/// Note that these behaviors differ from those of [`Config::figment()`].
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Config {
    /// The selected profile. **(default: _default_ `default`)**
    ///
    /// **Note:** This field is never serialized nor deserialized. When a
    /// `Config` is merged into a `Figment` as a `Provider`, this profile is
    /// selected on the `Figment`. When a `Config` is extracted, this field is
    /// set to the extracting Figment's selected `Profile`.
    #[serde(skip)]
    pub profile: Profile,
    /// path of the source contracts dir, like `src` or `contracts`
    pub src: PathBuf,
    /// path of the test dir
    pub test: PathBuf,
    /// path to where artifacts shut be written to
    pub out: PathBuf,
    /// all library folders to include, `lib`, `node_modules`
    pub libs: Vec<PathBuf>,
    /// `Remappings` to use for this repo
    pub remappings: Vec<RelativeRemapping>,
    /// library addresses to link
    pub libraries: Vec<String>,
    /// whether to enable cache
    pub cache: bool,
    /// whether to force a `project.clean()`
    pub force: bool,
    /// evm version to use
    #[serde(with = "from_str_lowercase")]
    pub evm_version: EvmVersion,
    /// Concrete solc version to use if any.
    ///
    /// This takes precedence over `auto_detect_solc`, if a version is set then this overrides
    /// auto-detection.
    pub solc_version: Option<Version>,
    /// whether to autodetect the solc compiler version to use
    pub auto_detect_solc: bool,
    /// Whether to activate optimizer
    pub optimizer: bool,
    /// Sets the optimizer runs
    pub optimizer_runs: usize,
    /// verbosity to use
    pub verbosity: u8,
    /// url of the rpc server that should be used for any rpc calls
    pub eth_rpc_url: Option<String>,
    /// list of solidity error codes to always silence
    pub ignored_error_codes: Vec<u64>,
    /// The number of test cases that must execute for each property test
    pub fuzz_runs: u32,
    /// Whether to allow ffi cheatcodes in test
    pub ffi: bool,
    /// The address which will be executing all tests
    pub sender: Address,
    /// The tx.origin value during EVM execution
    pub tx_origin: Address,
    /// the initial balance of each deployed test contract
    pub initial_balance: U256,
    /// the block.number value during EVM execution
    pub block_number: u64,
    /// pins the block number for the state fork
    pub fork_block_number: Option<u64>,
    /// the chainid opcode value
    pub chain_id: Option<Chain>,
    /// Block gas limit
    pub gas_limit: u64,
    /// `tx.gasprice` value during EVM execution"
    pub gas_price: u64,
    /// the base fee in a block
    pub block_base_fee_per_gas: u64,
    /// the `block.coinbase` value during EVM execution
    pub block_coinbase: Address,
    /// the `block.timestamp` value during EVM execution
    pub block_timestamp: u64,
    /// the `block.difficulty` value during EVM execution
    pub block_difficulty: u64,
    /// the `block.gaslimit` value during EVM execution
    pub block_gas_limit: Option<u64>,
    /// Settings to pass to the `solc` compiler input
    // TODO consider making this more structured https://stackoverflow.com/questions/48998034/does-toml-support-nested-arrays-of-objects-tables
    // TODO this needs to work as extension to the defaults:
    //{
    //   "*": {
    //     "": [
    //       "ast"
    //     ],
    //     "*": [
    //       "abi",
    //       "evm.bytecode",
    //       "evm.deployedBytecode",
    //       "evm.methodIdentifiers"
    //     ]
    //   }
    // }
    // "#
    pub solc_settings: Option<String>,
    /// The root path where the config detection started from, `Config::with_root`
    ///
    /// **Note:** This field is never serialized nor deserialized. This is merely used to provided
    /// additional context.
    #[doc(hidden)]
    #[serde(skip)]
    pub __root: RootPath,
    /// PRIVATE: This structure may grow, As such, constructing this structure should
    /// _always_ be done using a public constructor or update syntax:
    ///
    /// ```rust
    /// use foundry_config::Config;
    ///
    /// let config = Config {
    ///     src: "other".into(),
    ///     ..Default::default()
    /// };
    /// ```
    #[doc(hidden)]
    #[serde(skip)]
    pub __non_exhaustive: (),
}

impl Config {
    /// The default profile: "default"
    pub const DEFAULT_PROFILE: Profile = Profile::const_new("default");

    /// The hardhat profile: "hardhat"
    pub const HARDHAT_PROFILE: Profile = Profile::const_new("hardhat");

    /// File name of config toml file
    pub const FILE_NAME: &'static str = "foundry.toml";

    /// Returns the current `Config`
    ///
    /// See `Config::figment`
    pub fn load() -> Self {
        Config::from_provider(Config::figment())
    }

    /// Returns the current `Config`
    ///
    /// See `Config::figment_with_root`
    pub fn load_with_root(root: impl Into<PathBuf>) -> Self {
        Config::from_provider(Config::figment_with_root(root))
    }

    /// Extract a `Config` from `provider`, panicking if extraction fails.
    ///
    /// # Panics
    ///
    /// If extraction fails, prints an error message indicating the failure and
    /// panics. For a version that doesn't panic, use [`Config::try_from()`].
    ///
    /// # Example
    ///
    /// ```no_run
    /// use foundry_config::Config;
    /// use figment::providers::{Toml, Format, Env};
    ///
    /// // Use foundry's default `Figment`, but allow values from `other.toml`
    /// // to supersede its values.
    /// let figment = Config::figment()
    ///     .merge(Toml::file("other.toml").nested());
    ///
    /// let config = Config::from_provider(figment);
    /// ```
    pub fn from_provider<T: Provider>(provider: T) -> Self {
        Self::try_from(provider).expect("failed to extract from provider")
    }

    /// Attempts to extract a `Config` from `provider`, returning the result.
    ///
    /// # Example
    ///
    /// ```rust
    /// use foundry_config::Config;
    /// use figment::providers::{Toml, Format, Env};
    ///
    /// // Use foundry's default `Figment`, but allow values from `other.toml`
    /// // to supersede its values.
    /// let figment = Config::figment()
    ///     .merge(Toml::file("other.toml").nested());
    ///
    /// let config = Config::try_from(figment);
    /// ```
    pub fn try_from<T: Provider>(provider: T) -> Result<Self, figment::Error> {
        let figment = Figment::from(provider);
        let mut config = figment.extract::<Self>()?;
        config.profile = figment.profile().clone();
        Ok(config)
    }

    /// The config supports relative paths and tracks the root path separately see
    /// `Config::with_root`
    ///
    /// This joins all relative paths with the current root and attempts to make them canonic
    #[must_use]
    pub fn canonic(self) -> Self {
        let root = self.__root.0.clone();
        self.canonic_at(root)
    }

    /// Joins all relative paths with the given root so that paths that are defined as:
    ///
    /// ```toml
    /// [default]
    /// src = "src"
    /// out = "./out"
    /// libs = ["lib", "/var/lib"]
    /// ```
    ///
    /// Will be made canonic with the given root:
    ///
    /// ```toml
    /// [default]
    /// src = "<root>/src"
    /// out = "<root>/out"
    /// libs = ["<root>/lib", "/var/lib"]
    /// ```
    #[must_use]
    pub fn canonic_at(mut self, root: impl Into<PathBuf>) -> Self {
        let root = canonic(root);

        fn p(root: &Path, rem: &Path) -> PathBuf {
            canonic(root.join(rem))
        }

        self.src = p(&root, &self.src);
        self.test = p(&root, &self.test);
        self.out = p(&root, &self.out);

        self.libs = self.libs.into_iter().map(|lib| p(&root, &lib)).collect();

        self.remappings =
            self.remappings.into_iter().map(|r| RelativeRemapping::new(r.into(), &root)).collect();

        self
    }

    /// Returns a sanitized version of the Config where are paths are set correctly and potential
    /// duplicates are resolved
    ///
    /// See [`Self::canonic`]
    #[must_use]
    pub fn sanitized(self) -> Self {
        let mut config = self.canonic();

        // remove any potential duplicates
        config.remappings.sort_unstable();
        config.remappings.dedup();

        config.libs.sort_unstable();
        config.libs.dedup();

        config
    }

    /// Serves as the entrypoint for obtaining the project.
    ///
    /// Returns the `Project` configured with all `solc` and path related values.
    ///
    /// *Note*: this also _cleans_ [`Project::cleanup`] the workspace if `force` is set to true.
    ///
    /// # Example
    ///
    /// ```
    /// use foundry_config::Config;
    /// let config = Config::load_with_root(".").sanitized();
    /// let project = config.project();
    /// ```
    pub fn project(&self) -> Result<Project, SolcError> {
        self.create_project(true, false)
    }

    /// Same as [`Self::project()`] but sets configures the project to not emit artifacts and ignore
    /// cache, caching causes no output until https://github.com/gakonst/ethers-rs/issues/727
    pub fn ephemeral_no_artifacts_project(&self) -> Result<Project, SolcError> {
        self.create_project(false, true)
    }

    fn create_project(&self, cached: bool, no_artifacts: bool) -> Result<Project, SolcError> {
        let project = Project::builder()
            .paths(self.project_paths())
            .allowed_path(&self.__root.0)
            .allowed_paths(self.libraries.clone())
            .solc_config(SolcConfig::builder().settings(self.solc_settings()?).build())
            .ignore_error_codes(self.ignored_error_codes.clone())
            .set_auto_detect(self.auto_detect_solc)
            .set_cached(cached)
            .set_no_artifacts(no_artifacts)
            .build()?;

        if self.force {
            project.cleanup()?;
        }

        Ok(project)
    }

    /// Returns the `ProjectPathsConfig`  sub set of the config.
    ///
    /// **NOTE**: this uses the paths as they are and does __not__ modify them, see
    /// `[Self::sanitized]`
    ///
    /// # Example
    ///
    /// ```
    /// use foundry_config::Config;
    /// let config = Config::load_with_root(".").sanitized();
    /// let paths = config.project_paths();
    /// ```
    pub fn project_paths(&self) -> ProjectPathsConfig {
        ProjectPathsConfig::builder()
            .sources(&self.src)
            .artifacts(&self.out)
            .libs(self.libs.clone())
            .remappings(self.remappings.iter().map(|m| m.clone().into()))
            .build_with_root(&self.__root.0)
    }

    /// Returns the `Optimizer` based on the configured settings
    pub fn optimizer(&self) -> Optimizer {
        Optimizer { enabled: Some(self.optimizer), runs: Some(self.optimizer_runs) }
    }

    /// Returns the configured `solc` `Settings` that includes:
    ///   - all libraries
    ///   - the optimizer
    ///   - evm version
    pub fn solc_settings(&self) -> Result<Settings, SolcError> {
        let libraries = parse_libraries(&self.libraries)?;
        let optimizer = self.optimizer();
        Ok(Settings {
            optimizer,
            evm_version: Some(self.evm_version),
            libraries,
            ..Default::default()
        })
    }

    /// Returns the default figment
    ///
    /// The default figment reads from the following sources, in ascending
    /// priority order:
    ///
    ///   1. [`Config::default()`] (see [defaults](#defaults))
    ///   2. `foundry.toml` _or_ filename in `FOUNDRY_CONFIG` environment variable
    ///   3. `FOUNDRY_` prefixed environment variables
    ///
    /// The profile selected is the value set in the `FOUNDRY_PROFILE`
    /// environment variable. If it is not set, it defaults to `default`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use foundry_config::Config;
    /// use serde::Deserialize;
    ///
    /// let my_config = Config::figment().extract::<Config>();
    /// ```
    pub fn figment() -> Figment {
        Config::default().into()
    }

    /// Returns the default figment enhanced with additional context extracted from the provided
    /// root, like remappings and directories.
    ///
    /// # Example
    ///
    /// ```rust
    /// use foundry_config::Config;
    /// use serde::Deserialize;
    ///
    /// let my_config = Config::figment_with_root(".").extract::<Config>();
    /// ```
    pub fn figment_with_root(root: impl Into<PathBuf>) -> Figment {
        Self::with_root(root).into()
    }

    /// Creates a new Config that adds additional context extracted from the provided root.
    ///
    /// # Example
    ///
    /// ```rust
    /// use foundry_config::Config;
    /// let my_config = Config::with_root(".");
    /// ```
    pub fn with_root(root: impl Into<PathBuf>) -> Self {
        // autodetect paths
        let root = root.into();
        let paths = ProjectPathsConfig::builder().build_with_root(&root);
        Config {
            __root: paths.root.into(),
            src: paths.sources.file_name().unwrap().into(),
            out: paths.artifacts.file_name().unwrap().into(),
            libs: paths.libraries.into_iter().map(|lib| lib.file_name().unwrap().into()).collect(),
            remappings: paths
                .remappings
                .into_iter()
                .map(|r| RelativeRemapping::new(r, &root))
                .collect(),
            ..Config::default()
        }
    }

    /// Returns the default config but with hardhat paths
    pub fn hardhat() -> Self {
        Config {
            src: "contracts".into(),
            out: "artifacts".into(),
            libs: vec!["node_modules".into()],
            ..Config::default()
        }
    }

    /// Returns the default config that uses dapptools style paths
    pub fn dapptools() -> Self {
        Self::default()
    }

    /// Extracts a basic subset of the config, used for initialisations.
    ///
    /// # Example
    ///
    /// ```rust
    /// use foundry_config::Config;
    /// let my_config = Config::with_root(".").into_basic();
    /// ```
    pub fn into_basic(self) -> BasicConfig {
        BasicConfig {
            profile: self.profile,
            src: self.src,
            out: self.out,
            libs: self.libs,
            remappings: self.remappings,
        }
    }

    /// Serialize the config type as a String of TOML.
    ///
    /// This serializes to a table with the name of the profile
    ///
    /// ```toml
    /// [default]
    /// src = "src"
    /// out = "out"
    /// libs = ["lib"]
    /// # ...
    /// ```
    pub fn to_string_pretty(&self) -> Result<String, toml::ser::Error> {
        let s = toml::to_string_pretty(self)?;
        Ok(format!(
            r#"[{}]
{}"#,
            self.profile, s
        ))
    }

    /// Returns the selected profile
    ///
    /// If the `FOUNDRY_PROFILE` env variable is not set, this returns the `DEFAULT_PROFILE`
    pub fn selected_profile() -> Profile {
        Profile::from_env_or("FOUNDRY_PROFILE", Config::DEFAULT_PROFILE)
    }

    /// Returns the path to the `foundry.toml` file, the file is searched for in
    /// the current working directory and all parent directories until the root,
    /// and the first hit is used.
    pub fn find_config_file() -> Option<PathBuf> {
        fn find(path: &Path) -> Option<PathBuf> {
            if path.is_absolute() {
                return match path.is_file() {
                    true => Some(path.to_path_buf()),
                    false => None,
                }
            }
            let cwd = std::env::current_dir().ok()?;
            let mut cwd = cwd.as_path();
            loop {
                let file_path = cwd.join(path);
                if file_path.is_file() {
                    return Some(file_path)
                }
                cwd = cwd.parent()?;
            }
        }
        find(Env::var_or("FOUNDRY_CONFIG", Config::FILE_NAME).as_ref())
    }
}

impl From<Config> for Figment {
    fn from(c: Config) -> Figment {
        let profile = Config::selected_profile();
        let figment = Figment::default()
            .merge(DappHardhatDirProvider(&c.__root.0))
            .merge(ForcedSnakeCaseData(
                Toml::file(Env::var_or("FOUNDRY_CONFIG", Config::FILE_NAME)).nested(),
            ))
            .merge(Env::prefixed("DAPP_").ignore(&["REMAPPINGS"]).global())
            .merge(Env::prefixed("DAPP_TEST_").global())
            .merge(DappEnvCompatProvider)
            .merge(Env::prefixed("FOUNDRY_").ignore(&["PROFILE", "REMAPPINGS"]).global())
            .select(profile.clone());

        // we try to merge remappings after we've merged all other providers, this prevents
        // redundant fs lookups to determine the default remappings that are eventually updated by
        // other providers, like the toml file
        let remappings = RemappingsProvider {
            lib_paths: figment
                .extract_inner::<Vec<PathBuf>>("libs")
                .map(Cow::Owned)
                .unwrap_or_else(|_| Cow::Borrowed(&c.libs)),
            root: &c.__root.0,
            remappings: figment.extract_inner::<Vec<Remapping>>("remappings"),
        };
        let merge = figment.merge(remappings);

        Figment::from(c).merge(merge).select(profile)
    }
}

/// A helper wrapper around the root path used during Config detection
#[derive(Debug, PartialEq, Eq, Hash, Clone, PartialOrd, Ord)]
pub struct RootPath(pub PathBuf);

impl Default for RootPath {
    fn default() -> Self {
        ".".into()
    }
}

impl<P: Into<PathBuf>> From<P> for RootPath {
    fn from(p: P) -> Self {
        RootPath(p.into())
    }
}

/// Parses a config profile
///
/// All `Profile` date is ignored by serde, however the `Config::to_string_pretty` includes it and
/// returns a toml table like
///
/// ```toml
/// #[default]
/// src = "..."
/// ```
/// This ignores the `#[default]` part in the toml
pub fn parse_with_profile<T: serde::de::DeserializeOwned>(
    s: &str,
) -> Result<Option<(Profile, T)>, toml::de::Error> {
    let val: Map<Profile, T> = toml::from_str(s)?;
    Ok(val.into_iter().next())
}

impl Provider for Config {
    fn metadata(&self) -> Metadata {
        Metadata::named("Foundry Config")
    }

    #[track_caller]
    fn data(&self) -> Result<Map<Profile, Dict>, figment::Error> {
        Serialized::defaults(self).data()
    }

    fn profile(&self) -> Option<Profile> {
        Some(self.profile.clone())
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            profile: Self::DEFAULT_PROFILE,
            __root: Default::default(),
            src: "src".into(),
            test: "test".into(),
            out: "out".into(),
            libs: vec!["lib".into()],
            cache: true,
            force: false,
            evm_version: Default::default(),
            solc_version: None,
            auto_detect_solc: true,
            optimizer: true,
            optimizer_runs: 200,
            solc_settings: None,
            fuzz_runs: 256,
            ffi: false,
            sender: "00a329c0648769A73afAc7F9381E08FB43dBEA72".parse().unwrap(),
            tx_origin: "00a329c0648769A73afAc7F9381E08FB43dBEA72".parse().unwrap(),
            initial_balance: U256::from(0xffffffffffffffffffffffffu128),
            block_number: 0,
            fork_block_number: None,
            chain_id: None,
            // toml-rs can't handle larger number because integers are stored signed
            // https://github.com/alexcrichton/toml-rs/issues/256
            gas_limit: i64::MAX as u64,
            gas_price: 0,
            block_base_fee_per_gas: 0,
            block_coinbase: Address::zero(),
            block_timestamp: 0,
            block_difficulty: 0,
            block_gas_limit: None,
            eth_rpc_url: None,
            verbosity: 0,
            remappings: vec![],
            libraries: vec![],
            ignored_error_codes: vec![],
            __non_exhaustive: (),
        }
    }
}

/// A Provider that ensures all keys are snake case
struct ForcedSnakeCaseData<F: Format>(Data<F>);

impl<F: Format> Provider for ForcedSnakeCaseData<F> {
    fn metadata(&self) -> Metadata {
        Metadata::named("Snake Case toml provider")
    }

    fn data(&self) -> Result<Map<Profile, Dict>, Error> {
        let mut map = Map::new();
        for (profile, dict) in self.0.data()? {
            map.insert(profile, dict.into_iter().map(|(k, v)| (k.to_snake_case(), v)).collect());
        }
        Ok(map)
    }
}

/// A provider that sets the `src` and `output` path depending on their existence.
struct DappHardhatDirProvider<'a>(&'a Path);

impl<'a> Provider for DappHardhatDirProvider<'a> {
    fn metadata(&self) -> Metadata {
        Metadata::named("Dapp Hardhat dir compat")
    }

    fn data(&self) -> Result<Map<Profile, Dict>, Error> {
        let mut dict = Dict::new();
        dict.insert(
            "src".to_string(),
            ProjectPathsConfig::find_source_dir(self.0)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_string()
                .into(),
        );
        dict.insert(
            "out".to_string(),
            ProjectPathsConfig::find_artifacts_dir(self.0)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_string()
                .into(),
        );
        dict.insert(
            "libs".to_string(),
            ProjectPathsConfig::find_libs(self.0)
                .into_iter()
                .map(|lib| lib.file_name().unwrap().to_string_lossy().to_string())
                .collect::<Vec<_>>()
                .into(),
        );

        Ok(Map::from([(Config::selected_profile(), dict)]))
    }
}

/// A provider that checks for DAPP_ env vars that are named differently than FOUNDRY_
struct DappEnvCompatProvider;

impl Provider for DappEnvCompatProvider {
    fn metadata(&self) -> Metadata {
        Metadata::named("Dapp env compat")
    }

    fn data(&self) -> Result<Map<Profile, Dict>, Error> {
        use serde::de::Error as _;
        use std::env;

        let mut dict = Dict::new();
        if let Ok(val) = env::var("DAPP_TEST_NUMBER") {
            dict.insert(
                "block_number".to_string(),
                val.parse::<u64>().map_err(figment::Error::custom)?.into(),
            );
        }
        if let Ok(val) = env::var("DAPP_TEST_ADDRESS") {
            dict.insert("sender".to_string(), val.into());
        }
        if let Ok(val) = env::var("DAPP_FORK_BLOCK") {
            dict.insert(
                "fork_block_number".to_string(),
                val.parse::<u64>().map_err(figment::Error::custom)?.into(),
            );
        } else if let Ok(val) = env::var("DAPP_TEST_NUMBER") {
            dict.insert(
                "fork_block_number".to_string(),
                val.parse::<u64>().map_err(figment::Error::custom)?.into(),
            );
        }
        if let Ok(val) = env::var("DAPP_TEST_TIMESTAMP") {
            dict.insert(
                "block_timestamp".to_string(),
                val.parse::<u64>().map_err(figment::Error::custom)?.into(),
            );
        }
        if let Ok(val) = env::var("DAPP_BUILD_OPTIMIZE_RUNS") {
            dict.insert(
                "optimizer_runs".to_string(),
                val.parse::<u64>().map_err(figment::Error::custom)?.into(),
            );
        }
        if let Ok(val) = env::var("DAPP_BUILD_OPTIMIZE") {
            // Activate Solidity optimizer (0 or 1)
            let val = val.parse::<u8>().map_err(figment::Error::custom)?;
            if val > 1 {
                return Err(format!(
                    "Invalid $DAPP_BUILD_OPTIMIZE value `{}`,  expected 0 or 1",
                    val
                )
                .into())
            }
            dict.insert("optimizer".to_string(), (val == 1).into());
        }

        Ok(Map::from([(Config::selected_profile(), dict)]))
    }
}

/// A figment provider that checks if the remappings were previously set and if they're unset looks
/// up the fs via
///   - `DAPP_REMAPPINGS` || `FOUNDRY_REMAPPINGS` env var
///   - `<root>/remappings.txt` file
///   - `Remapping::find_many`.
struct RemappingsProvider<'a> {
    lib_paths: Cow<'a, Vec<PathBuf>>,
    /// the root path used to turn an absolute `Remapping`, as we're getting it from
    /// `Remapping::find_many` into a relative one.
    root: &'a PathBuf,
    /// This contains either:
    ///   - previously set remappings
    ///   - a `MissingField` error, which means previous provider didn't set the "remappings" field
    ///   - other error, like formatting
    remappings: Result<Vec<Remapping>, figment::Error>,
}

impl<'a> RemappingsProvider<'a> {
    /// Find and parse remappings for the projects
    ///
    /// **Order**
    ///
    /// Remappings are built in this order (last item takes precedence)
    /// - Autogenerated remappings
    /// - toml remappings
    /// - `remappings.txt`
    /// - Environment variables
    /// - CLI parameters
    fn get_remappings(&self, remappings: Vec<Remapping>) -> Result<Vec<Remapping>, Error> {
        let mut new_remappings = Vec::new();

        // check env var
        if let Some(env_remappings) = remappings_from_env_var("DAPP_REMAPPINGS")
            .or_else(|| remappings_from_env_var("FOUNDRY_REMAPPINGS"))
        {
            new_remappings
                .extend(env_remappings.map_err::<Error, _>(|err| err.to_string().into())?);
        }

        // check remappings.txt file
        let remappings_file = self.root.join("remappings.txt");
        if remappings_file.is_file() {
            let content =
                std::fs::read_to_string(remappings_file).map_err(|err| err.to_string())?;
            let remappings_from_file: Result<Vec<_>, _> =
                remappings_from_newline(&content).collect();
            new_remappings
                .extend(remappings_from_file.map_err::<Error, _>(|err| err.to_string().into())?);
        }

        new_remappings.extend(remappings);

        // look up lib paths
        new_remappings.extend(
            self.lib_paths
                .iter()
                .map(|lib| self.root.join(lib))
                .flat_map(Remapping::find_many)
                .collect::<Vec<Remapping>>(),
        );

        // remove duplicates
        new_remappings.sort_by(|a, b| a.name.cmp(&b.name));
        new_remappings.dedup_by(|a, b| a.name.eq(&b.name));

        Ok(new_remappings)
    }
}

impl<'a> Provider for RemappingsProvider<'a> {
    fn metadata(&self) -> Metadata {
        Metadata::named("Remapping Provider")
    }

    fn data(&self) -> Result<Map<Profile, Dict>, Error> {
        let remappings = match &self.remappings {
            Ok(remappings) => self.get_remappings(remappings.clone()),
            Err(err) => {
                if let figment::error::Kind::MissingField(_) = err.kind {
                    self.get_remappings(vec![])
                } else {
                    return Err(err.clone())
                }
            }
        }?;

        // turn the absolute remapping into a relative one by stripping the `root`
        let remappings = remappings
            .into_iter()
            .map(|r| RelativeRemapping::new(r, &self.root).to_string())
            .collect::<Vec<_>>();

        Ok(Map::from([(
            Config::selected_profile(),
            Dict::from([("remappings".to_string(), figment::value::Value::from(remappings))]),
        )]))
    }

    fn profile(&self) -> Option<Profile> {
        Some(Config::selected_profile())
    }
}

/// A subset of the foundry `Config`
/// used to initialize a `foundry.toml` file
///
/// # Example
///
/// ```rust
/// use foundry_config::{Config, BasicConfig};
/// use serde::Deserialize;
///
/// let my_config = Config::figment().extract::<BasicConfig>();
/// ```
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct BasicConfig {
    #[serde(skip)]
    pub profile: Profile,
    /// path of the source contracts dir, like `src` or `contracts`
    pub src: PathBuf,
    /// path to where artifacts shut be written to
    pub out: PathBuf,
    /// all library folders to include, `lib`, `node_modules`
    pub libs: Vec<PathBuf>,
    /// `Remappings` to use for this repo
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remappings: Vec<RelativeRemapping>,
}

impl BasicConfig {
    /// Serialize the config as a String of TOML.
    ///
    /// This serializes to a table with the name of the profile
    pub fn to_string_pretty(&self) -> Result<String, toml::ser::Error> {
        let s = toml::to_string_pretty(self)?;
        Ok(format!(
            r#"[{}]
{}
# See more config options https://github.com/gakonst/foundry/tree/master/config"#,
            self.profile, s
        ))
    }
}

/// Either a named or chain id or the actual id value
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Chain {
    #[serde(with = "from_str_lowercase")]
    Named(ethers_core::types::Chain),
    Id(u64),
}

mod from_str_lowercase {
    use std::str::FromStr;

    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<T, S>(value: &T, serializer: S) -> Result<S::Ok, S::Error>
    where
        T: std::fmt::Display,
        S: Serializer,
    {
        serializer.collect_str(&value.to_string().to_lowercase())
    }

    pub fn deserialize<'de, T, D>(deserializer: D) -> Result<T, D::Error>
    where
        D: Deserializer<'de>,
        T: FromStr,
        T::Err: std::fmt::Display,
    {
        String::deserialize(deserializer)?.to_lowercase().parse().map_err(serde::de::Error::custom)
    }
}

impl From<Chain> for u64 {
    fn from(c: Chain) -> Self {
        match c {
            Chain::Named(c) => c as u64,
            Chain::Id(id) => id,
        }
    }
}

fn canonic(path: impl Into<PathBuf>) -> PathBuf {
    let path = path.into();
    ethers_solc::utils::canonicalize(&path).unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use figment::error::Kind::InvalidType;
    use std::str::FromStr;

    use figment::{value::Value, Figment};
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_figment_is_default() {
        figment::Jail::expect_with(|_| {
            let mut default: Config = Config::figment().extract().unwrap();
            default.profile = Config::default().profile;
            assert_eq!(default, Config::default());
            Ok(())
        });
    }

    #[test]
    fn test_default_round_trip() {
        figment::Jail::expect_with(|_| {
            let original = Config::figment();
            let roundtrip = Figment::from(Config::from_provider(&original));
            for figment in &[original, roundtrip] {
                let config = Config::from_provider(figment);
                assert_eq!(config, Config::default());
            }
            Ok(())
        });
    }

    #[test]
    fn test_profile_env() {
        figment::Jail::expect_with(|jail| {
            jail.set_env("FOUNDRY_PROFILE", "default");
            let figment = Config::figment();
            assert_eq!(figment.profile(), "default");

            jail.set_env("FOUNDRY_PROFILE", "hardhat");
            let figment: Figment = Config::hardhat().into();
            assert_eq!(figment.profile(), "hardhat");
            Ok(())
        });
    }

    #[test]
    fn test_remappings() {
        figment::Jail::expect_with(|jail| {
            jail.create_file(
                "foundry.toml",
                r#"
                [default]
                src = "some-source"
                out = "some-out"
                cache = true
            "#,
            )?;
            let config = Config::load();
            assert!(config.remappings.is_empty());

            jail.create_file(
                "remappings.txt",
                r#"
                file-ds-test/=lib/ds-test/
                file-other/=lib/other/
            "#,
            )?;

            let config = Config::load();
            assert_eq!(
                config.remappings,
                vec![
                    Remapping::from_str("file-ds-test/=lib/ds-test/").unwrap().into(),
                    Remapping::from_str("file-other/=lib/other/").unwrap().into(),
                ],
            );

            jail.set_env("DAPP_REMAPPINGS", "ds-test=lib/ds-test/\nother/=lib/other/");
            let config = Config::load();

            assert_eq!(
                config.remappings,
                vec![
                    // From environment
                    Remapping::from_str("ds-test=lib/ds-test/").unwrap().into(),
                    // From remapping.txt
                    Remapping::from_str("file-ds-test/=lib/ds-test/").unwrap().into(),
                    Remapping::from_str("file-other/=lib/other/").unwrap().into(),
                    // From environment
                    Remapping::from_str("other/=lib/other/").unwrap().into(),
                ],
            );

            Ok(())
        });
    }

    #[test]
    fn test_remappings_override() {
        figment::Jail::expect_with(|jail| {
            jail.create_file(
                "foundry.toml",
                r#"
                [default]
                src = "some-source"
                out = "some-out"
                cache = true
            "#,
            )?;
            let config = Config::load();
            assert!(config.remappings.is_empty());

            jail.create_file(
                "remappings.txt",
                r#"
                ds-test/=lib/ds-test/
                other/=lib/other/
            "#,
            )?;

            let config = Config::load();
            assert_eq!(
                config.remappings,
                vec![
                    Remapping::from_str("ds-test/=lib/ds-test/").unwrap().into(),
                    Remapping::from_str("other/=lib/other/").unwrap().into(),
                ],
            );

            jail.set_env("DAPP_REMAPPINGS", "ds-test/=lib/ds-test/src/\nenv-lib/=lib/env-lib/");
            let config = Config::load();

            // Remappings should now be:
            // - ds-test from environment (lib/ds-test/src/)
            // - other from remappings.txt (lib/other/)
            // - env-lib from environment (lib/env-lib/)
            assert_eq!(
                config.remappings,
                vec![
                    Remapping::from_str("ds-test/=lib/ds-test/src/").unwrap().into(),
                    Remapping::from_str("env-lib/=lib/env-lib/").unwrap().into(),
                    Remapping::from_str("other/=lib/other/").unwrap().into(),
                ],
            );

            Ok(())
        });
    }

    #[test]
    fn test_toml_file() {
        figment::Jail::expect_with(|jail| {
            jail.create_file(
                "foundry.toml",
                r#"
                [default]
                src = "some-source"
                out = "some-out"
                cache = true
                eth_rpc_url = "https://example.com/"
                verbosity = 3
                remappings = ["ds-test=lib/ds-test/"]
            "#,
            )?;

            let config = Config::load();
            assert_eq!(
                config,
                Config {
                    src: "some-source".into(),
                    out: "some-out".into(),
                    cache: true,
                    eth_rpc_url: Some("https://example.com/".to_string()),
                    remappings: vec![Remapping::from_str("ds-test=lib/ds-test/").unwrap().into()],
                    verbosity: 3,
                    ..Config::default()
                }
            );

            Ok(())
        });
    }

    #[test]
    fn test_toml_casing_file() {
        figment::Jail::expect_with(|jail| {
            jail.create_file(
                "foundry.toml",
                r#"
                [default]
                src = "some-source"
                out = "some-out"
                cache = true
                eth-rpc-url = "https://example.com/"
                evm-version = "berlin"
                auto-detect-solc = false
            "#,
            )?;

            let config = Config::load();
            assert_eq!(
                config,
                Config {
                    src: "some-source".into(),
                    out: "some-out".into(),
                    cache: true,
                    eth_rpc_url: Some("https://example.com/".to_string()),
                    auto_detect_solc: false,
                    evm_version: EvmVersion::Berlin,
                    ..Config::default()
                }
            );

            Ok(())
        });
    }

    #[test]
    fn test_precedence() {
        figment::Jail::expect_with(|jail| {
            jail.create_file(
                "foundry.toml",
                r#"
                [default]
                src = "mysrc"
                out = "myout"
                verbosity = 3
            "#,
            )?;

            let config = Config::load();
            assert_eq!(
                config,
                Config {
                    src: "mysrc".into(),
                    out: "myout".into(),
                    verbosity: 3,
                    ..Config::default()
                }
            );

            jail.set_env("FOUNDRY_SRC", r#"other-src"#);
            let config = Config::load();
            assert_eq!(
                config,
                Config {
                    src: "other-src".into(),
                    out: "myout".into(),
                    verbosity: 3,
                    ..Config::default()
                }
            );

            jail.set_env("FOUNDRY_PROFILE", "foo");
            let val: Result<String, _> = Config::figment().extract_inner("profile");
            assert!(val.is_err());

            Ok(())
        });
    }

    #[test]
    fn test_extract_basic() {
        figment::Jail::expect_with(|jail| {
            jail.create_file(
                "foundry.toml",
                r#"
                [default]
                src = "mysrc"
                out = "myout"
                verbosity = 3
                evm_version = 'berlin'

                [other]
                src = "other-src"
            "#,
            )?;
            let loaded = Config::load();
            assert_eq!(loaded.evm_version, EvmVersion::Berlin);
            let base = loaded.into_basic();
            let default = Config::default();
            assert_eq!(
                base,
                BasicConfig {
                    profile: Config::DEFAULT_PROFILE,
                    src: "mysrc".into(),
                    out: "myout".into(),
                    libs: default.libs.clone(),
                    remappings: default.remappings.clone(),
                }
            );
            jail.set_env("FOUNDRY_PROFILE", r#"other"#);
            let base = Config::figment().extract::<BasicConfig>().unwrap();
            assert_eq!(
                base,
                BasicConfig {
                    profile: Config::DEFAULT_PROFILE,
                    src: "other-src".into(),
                    out: "out".into(),
                    libs: default.libs.clone(),
                    remappings: default.remappings,
                }
            );
            Ok(())
        });
    }

    #[test]
    fn can_handle_deviating_dapp_aliases() {
        figment::Jail::expect_with(|jail| {
            let addr = Address::random();
            jail.set_env("DAPP_TEST_NUMBER", 1337);
            jail.set_env("DAPP_TEST_ADDRESS", format!("{:?}", addr));
            jail.set_env("DAPP_TEST_FUZZ_RUNS", 420);
            jail.set_env("DAPP_FORK_BLOCK", 100);
            jail.set_env("DAPP_BUILD_OPTIMIZE_RUNS", 999);
            jail.set_env("DAPP_BUILD_OPTIMIZE", 0);

            let config = Config::load();

            assert_eq!(config.block_number, 1337);
            assert_eq!(config.sender, addr);
            assert_eq!(config.fuzz_runs, 420);
            assert_eq!(config.fork_block_number, Some(100));
            assert_eq!(config.optimizer_runs, 999);
            assert!(!config.optimizer);

            Ok(())
        });
    }

    #[test]
    fn config_roundtrip() {
        figment::Jail::expect_with(|jail| {
            let default = Config::default();
            let basic = default.clone().into_basic();
            jail.create_file("foundry.toml", &basic.to_string_pretty().unwrap())?;

            let other = Config::load();
            assert_eq!(default, other);

            let other = other.into_basic();
            assert_eq!(basic, other);

            jail.create_file("foundry.toml", &default.to_string_pretty().unwrap())?;
            let other = Config::load();
            assert_eq!(default, other);

            // println!("{}", default.to_string_pretty().unwrap());
            Ok(())
        });
    }

    #[test]
    fn can_use_impl_figment_macro() {
        #[derive(Default, Serialize)]
        struct MyArgs {
            root: Option<PathBuf>,
        }
        impl_figment_convert!(MyArgs);

        impl Provider for MyArgs {
            fn metadata(&self) -> Metadata {
                Metadata::default()
            }

            fn data(&self) -> Result<Map<Profile, Dict>, Error> {
                let value = Value::serialize(self)?;
                let error = InvalidType(value.to_actual(), "map".into());
                let dict = value.into_dict().ok_or(error)?;
                Ok(Map::from([(Config::selected_profile(), dict)]))
            }
        }

        let _figment: Figment = From::from(&MyArgs::default());
        let _config: Config = From::from(&MyArgs::default());

        #[derive(Default)]
        struct Outer {
            start: MyArgs,
            other: MyArgs,
            another: MyArgs,
        }
        impl_figment_convert!(Outer, start, other, another);

        let _figment: Figment = From::from(&Outer::default());
        let _config: Config = From::from(&Outer::default());
    }
}
