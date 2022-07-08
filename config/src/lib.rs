//! foundry configuration.
#![deny(missing_docs, unsafe_code, unused_crate_dependencies)]

use crate::cache::StorageCachingConfig;
use ethers_core::types::{Address, H160, U256};
pub use ethers_solc::artifacts::OptimizerDetails;
use ethers_solc::{
    artifacts::{
        output_selection::ContractOutputSelection, serde_helpers, BytecodeHash, DebuggingSettings,
        Libraries, ModelCheckerSettings, ModelCheckerTarget, Optimizer, RevertStrings, Settings,
    },
    cache::SOLIDITY_FILES_CACHE_FILENAME,
    error::SolcError,
    remappings::{RelativeRemapping, Remapping},
    ConfigurableArtifacts, EvmVersion, Project, ProjectPathsConfig, Solc, SolcConfig,
};
use eyre::{ContextCompat, WrapErr};
use figment::{
    providers::{Env, Format, Serialized, Toml},
    value::{Dict, Map, Value},
    Error, Figment, Metadata, Profile, Provider,
};
use inflector::Inflector;
use regex::Regex;
use semver::Version;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{
    borrow::Cow,
    collections::{hash_map::Entry, BTreeSet, HashMap},
    fs,
    path::{Path, PathBuf},
    str::FromStr,
};
use tracing::trace;

// Macros useful for creating a figment.
mod macros;

// Utilities for making it easier to handle tests.
pub mod utils;
pub use crate::utils::*;

mod rpc;
pub use rpc::{ResolvedRpcEndpoints, RpcEndpoint, RpcEndpoints, UnresolvedEnvVarError};

pub mod cache;
use cache::{Cache, ChainCache};

mod chain;
pub use chain::Chain;

mod error;
pub use error::SolidityErrorCode;

// reexport so cli types can implement `figment::Provider` to easily merge compiler arguments
pub use figment;

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
    /// path of the script dir
    pub script: PathBuf,
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
    /// where the cache is stored if enabled
    pub cache_path: PathBuf,
    /// where the broadcast logs are stored
    pub broadcast: PathBuf,
    /// additional solc allow paths
    pub allow_paths: Vec<PathBuf>,
    /// whether to force a `project.clean()`
    pub force: bool,
    /// evm version to use
    #[serde(with = "from_str_lowercase")]
    pub evm_version: EvmVersion,
    /// list of contracts to report gas of
    pub gas_reports: Vec<String>,
    /// The Solc instance to use if any.
    ///
    /// This takes precedence over `auto_detect_solc`, if a version is set then this overrides
    /// auto-detection.
    ///
    /// **Note** for backwards compatibility reasons this also accepts solc_version from the toml
    /// file, see [`BackwardsCompatProvider`]
    pub solc: Option<SolcReq>,
    /// whether to autodetect the solc compiler version to use
    pub auto_detect_solc: bool,
    /// Offline mode, if set, network access (downloading solc) is disallowed.
    ///
    /// Relationship with `auto_detect_solc`:
    ///    - if `auto_detect_solc = true` and `offline = true`, the required solc version(s) will
    ///      be auto detected but if the solc version is not installed, it will _not_ try to
    ///      install it
    pub offline: bool,
    /// Whether to activate optimizer
    pub optimizer: bool,
    /// Sets the optimizer runs
    pub optimizer_runs: usize,
    /// Switch optimizer components on or off in detail.
    /// The "enabled" switch above provides two defaults which can be
    /// tweaked here. If "details" is given, "enabled" can be omitted.
    pub optimizer_details: Option<OptimizerDetails>,
    /// Model checker settings.
    pub model_checker: Option<ModelCheckerSettings>,
    /// verbosity to use
    pub verbosity: u8,
    /// url of the rpc server that should be used for any rpc calls
    pub eth_rpc_url: Option<String>,
    /// etherscan API key
    pub etherscan_api_key: Option<String>,
    /// list of solidity error codes to always silence in the compiler output
    pub ignored_error_codes: Vec<SolidityErrorCode>,
    /// Only run test functions matching the specified regex pattern.
    #[serde(rename = "match_test")]
    pub test_pattern: Option<RegexWrapper>,
    /// Only run test functions that do not match the specified regex pattern.
    #[serde(rename = "no_match_test")]
    pub test_pattern_inverse: Option<RegexWrapper>,
    /// Only run tests in contracts matching the specified regex pattern.
    #[serde(rename = "match_contract")]
    pub contract_pattern: Option<RegexWrapper>,
    /// Only run tests in contracts that do not match the specified regex pattern.
    #[serde(rename = "no_match_contract")]
    pub contract_pattern_inverse: Option<RegexWrapper>,
    /// Only run tests in source files matching the specified glob pattern.
    #[serde(rename = "match_path", with = "from_opt_glob")]
    pub path_pattern: Option<globset::Glob>,
    /// Only run tests in source files that do not match the specified glob pattern.
    #[serde(rename = "no_match_path", with = "from_opt_glob")]
    pub path_pattern_inverse: Option<globset::Glob>,
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
    pub gas_limit: GasLimit,
    /// `tx.gasprice` value during EVM execution"
    ///
    /// This is an Option, so we can determine in fork mode whether to use the config's gas price
    /// (if set by user) or the remote client's gas price
    pub gas_price: Option<u64>,
    /// the base fee in a block
    pub block_base_fee_per_gas: u64,
    /// the `block.coinbase` value during EVM execution
    pub block_coinbase: Address,
    /// the `block.timestamp` value during EVM execution
    pub block_timestamp: u64,
    /// the `block.difficulty` value during EVM execution
    pub block_difficulty: u64,
    /// the `block.gaslimit` value during EVM execution
    pub block_gas_limit: Option<GasLimit>,
    /// The memory limit of the EVM (32 MB by default)
    pub memory_limit: u64,
    /// Additional output selection for all contracts
    /// such as "ir", "devodc", "storageLayout", etc.
    /// See [Solc Compiler Api](https://docs.soliditylang.org/en/latest/using-the-compiler.html#compiler-api)
    ///
    /// The following values are always set because they're required by `forge`
    //{
    //   "*": [
    //       "abi",
    //       "evm.bytecode",
    //       "evm.deployedBytecode",
    //       "evm.methodIdentifiers"
    //     ]
    // }
    // "#
    #[serde(default)]
    pub extra_output: Vec<ContractOutputSelection>,
    /// If set , a separate `json` file will be emitted for every contract depending on the
    /// selection, eg. `extra_output_files = ["metadata"]` will create a `metadata.json` for
    /// each contract in the project. See [Contract Metadata](https://docs.soliditylang.org/en/latest/metadata.html)
    ///
    /// The difference between `extra_output = ["metadata"]` and
    /// `extra_output_files = ["metadata]` is that the former will include the
    /// contract's metadata in the contract's json artifact, whereas the latter will emit the
    /// output selection as separate files.
    #[serde(default)]
    pub extra_output_files: Vec<ContractOutputSelection>,
    /// The maximum number of local test case rejections allowed
    /// by proptest, to be encountered during usage of `vm.assume`
    /// cheatcode.
    pub fuzz_max_local_rejects: u32,
    /// The maximum number of global test case rejections allowed
    /// by proptest, to be encountered during usage of `vm.assume`
    /// cheatcode.
    pub fuzz_max_global_rejects: u32,
    /// Print the names of the compiled contracts
    pub names: bool,
    /// Print the sizes of the compiled contracts
    pub sizes: bool,
    /// If set to true, changes compilation pipeline to go through the Yul intermediate
    /// representation.
    pub via_ir: bool,
    /// RPC storage caching settings determines what chains and endpoints to cache
    pub rpc_storage_caching: StorageCachingConfig,
    /// Disables storage caching entirely. This overrides any settings made in
    /// `rpc_storage_caching`
    pub no_storage_caching: bool,
    /// Multiple rpc endpoints and their aliases
    #[serde(default, skip_serializing_if = "RpcEndpoints::is_empty")]
    pub rpc_endpoints: RpcEndpoints,
    /// Whether to include the metadata hash.
    ///
    /// The metadata hash is machine dependent. By default, this is set to [BytecodeHash::None] to allow for deterministic code, See: <https://docs.soliditylang.org/en/latest/metadata.html>
    #[serde(with = "from_str_lowercase")]
    pub bytecode_hash: BytecodeHash,
    /// How to treat revert (and require) reason strings.
    #[serde(with = "serde_helpers::display_from_str_opt")]
    pub revert_strings: Option<RevertStrings>,
    /// Whether to compile in sparse mode
    ///
    /// If this option is enabled, only the required contracts/files will be selected to be
    /// included in solc's output selection, see also
    /// [OutputSelection](ethers_solc::artifacts::output_selection::OutputSelection)
    pub sparse_mode: bool,
    /// Whether to emit additional build info files
    ///
    /// If set to `true`, `ethers-solc` will generate additional build info json files for every
    /// new build, containing the `CompilerInput` and `CompilerOutput`
    pub build_info: bool,
    /// The path to the `build-info` directory that contains the build info json files.
    pub build_info_path: Option<PathBuf>,
    /// The root path where the config detection started from, `Config::with_root`
    #[doc(hidden)]
    //  We're skipping serialization here, so it won't be included in the [`Config::to_string()`]
    // representation, but will be deserialized from the `Figment` so that forge commands can
    // override it.
    #[serde(rename = "root", default, skip_serializing)]
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

    /// The name of the directory foundry reserves for itself under the user's home directory: `~`
    pub const FOUNDRY_DIR_NAME: &'static str = ".foundry";

    /// Default address for tx.origin
    pub const DEFAULT_SENDER: H160 = H160([
        0, 163, 41, 192, 100, 135, 105, 167, 58, 250, 199, 249, 56, 30, 8, 251, 67, 219, 234, 114,
    ]);

    /// Returns the current `Config`
    ///
    /// See `Config::figment`
    #[track_caller]
    pub fn load() -> Self {
        Config::from_provider(Config::figment())
    }

    /// Returns the current `Config`
    ///
    /// See `Config::figment_with_root`
    #[track_caller]
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
        trace!("load config with provider: {:?}", provider.metadata());
        match Self::try_from(provider) {
            Ok(config) => config,
            Err(errors) => {
                // providers can be nested and can return duplicate errors
                let errors: BTreeSet<_> =
                    errors.into_iter().map(|err| format!("config error: {}", err)).collect();
                for error in errors {
                    eprintln!("{}", error);
                }
                panic!("failed to extract foundry config")
            }
        }
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
        self.script = p(&root, &self.script);
        self.out = p(&root, &self.out);

        if let Some(build_info_path) = self.build_info_path {
            self.build_info_path = Some(p(&root, &build_info_path));
        }

        self.libs = self.libs.into_iter().map(|lib| p(&root, &lib)).collect();

        self.remappings =
            self.remappings.into_iter().map(|r| RelativeRemapping::new(r.into(), &root)).collect();

        self.cache_path = p(&root, &self.cache_path);

        self.allow_paths = self.allow_paths.into_iter().map(|allow| p(&root, &allow)).collect();

        if let Some(ref mut model_checker) = self.model_checker {
            model_checker.contracts = std::mem::take(&mut model_checker.contracts)
                .into_iter()
                .map(|(path, contracts)| {
                    (format!("{}", p(&root, path.as_ref()).display()), contracts)
                })
                .collect();
        }

        self
    }

    /// Returns a sanitized version of the Config where are paths are set correctly and potential
    /// duplicates are resolved
    ///
    /// See [`Self::canonic`]
    #[must_use]
    pub fn sanitized(self) -> Self {
        let mut config = self.canonic();

        config.sanitize_remappings();

        config.libs.sort_unstable();
        config.libs.dedup();

        config
    }

    /// Cleans up any duplicate `Remapping` and sorts them
    ///
    /// On windows this will convert any `\` in the remapping path into a `/`
    pub fn sanitize_remappings(&mut self) {
        #[cfg(target_os = "windows")]
        {
            // force `/` in remappings on windows
            use path_slash::PathBufExt;
            self.remappings.iter_mut().for_each(|r| {
                r.path.path = r.path.path.to_slash_lossy().into_owned().into();
            });
        }
        // remove any potential duplicates
        self.remappings.sort_unstable();
        self.remappings.dedup();
    }

    /// Returns the directory in which dependencies should be installed
    ///
    /// Returns the first dir from `libs` that is not `node_modules` or `lib` if `libs` is empty
    pub fn install_lib_dir(&self) -> PathBuf {
        self.libs
            .iter()
            .find(|p| !p.ends_with("node_modules"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("lib"))
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
        let mut project = Project::builder()
            .artifacts(self.configured_artifacts_handler())
            .paths(self.project_paths())
            .allowed_path(&self.__root.0)
            .allowed_paths(&self.libs)
            .allowed_paths(&self.allow_paths)
            .solc_config(SolcConfig::builder().settings(self.solc_settings()?).build())
            .ignore_error_codes(self.ignored_error_codes.iter().copied().map(Into::into))
            .set_auto_detect(self.is_auto_detect())
            .set_offline(self.offline)
            .set_cached(cached)
            .set_build_info(cached & self.build_info)
            .set_no_artifacts(no_artifacts)
            .build()?;

        if self.force {
            project.cleanup()?;
        }

        if let Some(solc) = self.ensure_solc()? {
            project.solc = solc;
        }

        Ok(project)
    }

    /// Ensures that the configured version is installed if explicitly set
    ///
    /// If `solc` is [`SolcReq::Version`] then this will download and install the solc version if
    /// it's missing.
    ///
    /// If `solc` is [`SolcReq::Local`] then this will ensure that the path exists.
    fn ensure_solc(&self) -> Result<Option<Solc>, SolcError> {
        if let Some(ref solc) = self.solc {
            let solc = match solc {
                SolcReq::Version(version) => {
                    let v = version.to_string();
                    let mut solc = Solc::find_svm_installed_version(&v)?;
                    if solc.is_none() {
                        Solc::blocking_install(version)?;
                        solc = Solc::find_svm_installed_version(&v)?;
                    }
                    solc
                }
                SolcReq::Local(solc) => {
                    if !solc.is_file() {
                        return Err(SolcError::msg(format!(
                            "`solc` {} does not exist",
                            solc.display()
                        )))
                    }
                    Some(Solc::new(solc))
                }
            };
            return Ok(solc)
        }

        Ok(None)
    }

    /// Returns whether the compiler version should be auto-detected
    ///
    /// Returns `false` if `solc_version` is explicitly set, otherwise returns the value of
    /// `auto_detect_solc`
    pub fn is_auto_detect(&self) -> bool {
        if self.solc.is_some() {
            return false
        }
        self.auto_detect_solc
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
        let mut builder = ProjectPathsConfig::builder()
            .cache(&self.cache_path.join(SOLIDITY_FILES_CACHE_FILENAME))
            .sources(&self.src)
            .tests(&self.test)
            .scripts(&self.script)
            .artifacts(&self.out)
            .libs(self.libs.clone())
            .remappings(self.get_all_remappings());

        if let Some(build_info_path) = &self.build_info_path {
            builder = builder.build_infos(&build_info_path);
        }

        builder.build_with_root(&self.__root.0)
    }

    /// Returns all configured [`Remappings`]
    ///
    /// **Note:** this will add an additional `<src>/=<src path>` remapping here, see
    /// [Self::get_source_dir_remapping()]
    ///
    /// So that
    ///
    /// ```solidity
    /// import "./math/math.sol";
    /// import "contracts/tokens/token.sol";
    /// ```
    ///
    /// in `contracts/contract.sol` are resolved to
    ///
    /// ```text
    /// contracts/tokens/token.sol
    /// contracts/math/math.sol
    /// ```
    pub fn get_all_remappings(&self) -> Vec<Remapping> {
        self.remappings
            .iter()
            .map(|m| m.clone().into())
            .chain(self.get_source_dir_remapping())
            .chain(self.get_test_dir_remapping())
            .chain(self.get_script_dir_remapping())
            .collect()
    }

    /// Returns the remapping for the project's _src_ directory
    ///
    /// **Note:** this will add an additional `<src>/=<src path>` remapping here so imports that
    /// look like `import {Foo} from "src/Foo.sol";` are properly resolved.
    ///
    /// This is due the fact that `solc`'s VFS resolves [direct imports](https://docs.soliditylang.org/en/develop/path-resolution.html#direct-imports) that start with the source directory's name.
    pub fn get_source_dir_remapping(&self) -> Option<Remapping> {
        get_dir_remapping(&self.src)
    }

    /// Returns the remapping for the project's _test_ directory, but only if it exists
    pub fn get_test_dir_remapping(&self) -> Option<Remapping> {
        if self.__root.0.join(&self.test).exists() {
            get_dir_remapping(&self.test)
        } else {
            None
        }
    }

    /// Returns the remapping for the project's _script_ directory, but only if it exists
    pub fn get_script_dir_remapping(&self) -> Option<Remapping> {
        if self.__root.0.join(&self.script).exists() {
            get_dir_remapping(&self.script)
        } else {
            None
        }
    }

    /// Returns the `Optimizer` based on the configured settings
    pub fn optimizer(&self) -> Optimizer {
        Optimizer {
            enabled: Some(self.optimizer),
            runs: Some(self.optimizer_runs),
            details: self.optimizer_details.clone(),
        }
    }

    /// returns the [`ethers_solc::ConfigurableArtifacts`] for this config, that includes the
    /// `extra_output` fields
    pub fn configured_artifacts_handler(&self) -> ConfigurableArtifacts {
        ConfigurableArtifacts::new(self.extra_output.clone(), self.extra_output_files.clone())
    }

    /// Parses all libraries in the form of
    /// `<file>:<lib>:<addr>`
    pub fn parsed_libraries(&self) -> Result<Libraries, SolcError> {
        Libraries::parse(&self.libraries)
    }

    /// Returns the configured `solc` `Settings` that includes:
    ///   - all libraries
    ///   - the optimizer (including details, if configured)
    ///   - evm version
    pub fn solc_settings(&self) -> Result<Settings, SolcError> {
        let libraries = self.parsed_libraries()?.with_applied_remappings(&self.project_paths());
        let optimizer = self.optimizer();

        // By default if no targets are specifically selected the model checker uses all targets.
        // This might be too much here, so only enable assertion checks.
        // If users wish to enable all options they need to do so explicitly.
        let mut model_checker = self.model_checker.clone();
        if let Some(ref mut model_checker_settings) = model_checker {
            if model_checker_settings.targets.is_none() {
                model_checker_settings.targets = Some(vec![ModelCheckerTarget::Assert]);
            }
        }

        let mut settings = Settings {
            optimizer,
            evm_version: Some(self.evm_version),
            libraries,
            metadata: Some(self.bytecode_hash.into()),
            debug: self.revert_strings.map(|revert_strings| DebuggingSettings {
                revert_strings: Some(revert_strings),
                debug_info: Vec::new(),
            }),
            model_checker,
            ..Default::default()
        }
        .with_extra_output(self.configured_artifacts_handler().output_selection())
        .with_ast();

        if self.via_ir {
            settings = settings.with_via_ir();
        }

        Ok(settings)
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
        Config {
            chain_id: Some(Chain::Id(99)),
            block_timestamp: 0,
            block_number: 0,
            ..Config::default()
        }
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

    /// Updates the `foundry.toml` file for the given `root` based on the provided closure.
    ///
    /// **Note:** the closure will only be invoked if the `foundry.toml` file exists, See
    /// [Self::get_config_path()] and if the closure returns `true`.
    pub fn update_at<F>(root: impl Into<PathBuf>, f: F) -> eyre::Result<()>
    where
        F: FnOnce(&Config, &mut toml_edit::Document) -> bool,
    {
        let config = Self::load_with_root(root).sanitized();
        config.update(|doc| f(&config, doc))
    }

    /// Updates the `foundry.toml` file this `Config` ias based on with the provided closure.
    ///
    /// **Note:** the closure will only be invoked if the `foundry.toml` file exists, See
    /// [Self::get_config_path()] and if the closure returns `true`
    pub fn update<F>(&self, f: F) -> eyre::Result<()>
    where
        F: FnOnce(&mut toml_edit::Document) -> bool,
    {
        let file_path = self.get_config_path();
        if !file_path.exists() {
            return Ok(())
        }
        let cargo_toml_content = fs::read_to_string(&file_path)?;
        let mut doc = cargo_toml_content.parse::<toml_edit::Document>()?;
        if f(&mut doc) {
            fs::write(file_path, doc.to_string())?;
        }
        Ok(())
    }

    /// Sets the `libs` entry inside a `foundry.toml` file but only if it exists
    ///
    /// # Errors
    ///
    /// An error if the `foundry.toml` could not be parsed.
    pub fn update_libs(&self) -> eyre::Result<()> {
        self.update(|doc| {
            let profile = self.profile.as_str().as_str();
            let libs: toml_edit::Value =
                self.libs.iter().map(|p| toml_edit::Value::from(&*p.to_string_lossy())).collect();
            let libs = toml_edit::value(libs);
            doc[profile]["libs"] = libs;
            true
        })
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
        // serializing to value first to prevent `ValueAfterTable` errors
        let value = toml::Value::try_from(self)?;
        let mut s = toml::to_string_pretty(&value)?;

        if self.optimizer_details.is_some() {
            // this is a hack to make nested tables work because this requires the config's profile
            s = s
                .replace("[optimizer_details]", &format!("[{}.optimizer_details]", self.profile))
                .replace(
                    "[optimizer_details.yulDetails]",
                    &format!("[{}.optimizer_details.yulDetails]", self.profile),
                );
        }
        if self.model_checker.is_some() {
            // similarly to the optimizer details above,
            // this is a hack to make nested tables work because this requires the config's profile
            s = ["contracts", "engine", "targets", "timeout"]
                .iter()
                .fold(s, |acc, op| {
                    acc.replace(
                        &format!("[model_checker.{}]", op),
                        &format!("[{}.model_checker.{}]", self.profile, op),
                    )
                })
                .replace("[model_checker]", &format!("[{}.model_checker]", self.profile));
        }
        s = s.replace("[rpc_storage_caching]", &format!("[{}.rpc_storage_caching]", self.profile));

        if !self.rpc_endpoints.is_empty() {
            s = s.replace("[rpc_endpoints]", &format!("[{}.rpc_endpoints]", self.profile));
        }

        Ok(format!(
            r#"[{}]
{}"#,
            self.profile, s
        ))
    }

    /// Returns the path to the `foundry.toml`  of this `Config`
    pub fn get_config_path(&self) -> PathBuf {
        self.__root.0.join(Config::FILE_NAME)
    }

    /// Returns the selected profile
    ///
    /// If the `FOUNDRY_PROFILE` env variable is not set, this returns the `DEFAULT_PROFILE`
    pub fn selected_profile() -> Profile {
        Profile::from_env_or("FOUNDRY_PROFILE", Config::DEFAULT_PROFILE)
    }

    /// Returns the path to foundry's global toml file that's stored at `~/.foundry/foundry.toml`
    pub fn foundry_dir_toml() -> Option<PathBuf> {
        Self::foundry_dir().map(|p| p.join(Config::FILE_NAME))
    }

    /// Returns the path to foundry's config dir `~/.foundry/`
    pub fn foundry_dir() -> Option<PathBuf> {
        dirs_next::home_dir().map(|p| p.join(Config::FOUNDRY_DIR_NAME))
    }

    /// Returns the path to foundry's cache dir `~/.foundry/cache`
    pub fn foundry_cache_dir() -> Option<PathBuf> {
        Self::foundry_dir().map(|p| p.join("cache"))
    }

    /// Returns the path to foundry rpc cache dir `~/.foundry/cache/rpc`
    pub fn foundry_rpc_cache_dir() -> Option<PathBuf> {
        Some(Self::foundry_cache_dir()?.join("rpc"))
    }
    /// Returns the path to foundry chain's cache dir `~/.foundry/cache/rpc/<chain>`
    pub fn foundry_chain_cache_dir(chain_id: impl Into<Chain>) -> Option<PathBuf> {
        Some(Self::foundry_rpc_cache_dir()?.join(chain_id.into().to_string()))
    }

    /// Returns the path to foundry's etherscan cache dir `~/.foundry/cache/etherscan`
    pub fn foundry_etherscan_cache_dir() -> Option<PathBuf> {
        Some(Self::foundry_cache_dir()?.join("etherscan"))
    }

    /// Returns the path to foundry's etherscan cache dir for `chain_id`
    /// `~/.foundry/cache/etherscan/<chain>`
    pub fn foundry_etherscan_chain_cache_dir(chain_id: impl Into<Chain>) -> Option<PathBuf> {
        Some(Self::foundry_etherscan_cache_dir()?.join(chain_id.into().to_string()))
    }

    /// Returns the path to the cache dir of the `block` on the `chain`
    /// `~/.foundry/cache/rpc/<chain>/<block>
    pub fn foundry_block_cache_dir(chain_id: impl Into<Chain>, block: u64) -> Option<PathBuf> {
        Some(Self::foundry_chain_cache_dir(chain_id)?.join(format!("{block}")))
    }

    /// Returns the path to the cache file of the `block` on the `chain`
    /// `~/.foundry/cache/rpc/<chain>/<block>/storage.json`
    pub fn foundry_block_cache_file(chain_id: impl Into<Chain>, block: u64) -> Option<PathBuf> {
        Some(Self::foundry_block_cache_dir(chain_id, block)?.join("storage.json"))
    }

    #[doc = r#"Returns the path to `foundry`'s data directory inside the user's data directory
    |Platform | Value                                 | Example                          |
    | ------- | ------------------------------------- | -------------------------------- |
    | Linux   | `$XDG_CONFIG_HOME` or `$HOME`/.config/foundry | /home/alice/.config/foundry|
    | macOS   | `$HOME`/Library/Application Support/foundry   | /Users/Alice/Library/Application Support/foundry |
    | Windows | `{FOLDERID_RoamingAppData}/foundry`           | C:\Users\Alice\AppData\Roaming/foundry   |
    "#]
    pub fn data_dir() -> eyre::Result<PathBuf> {
        let path = dirs_next::data_dir().wrap_err("Failed to find data directory")?.join("foundry");
        std::fs::create_dir_all(&path).wrap_err("Failed to create module directory")?;
        Ok(path)
    }

    /// Returns the path to the `foundry.toml` file, the file is searched for in
    /// the current working directory and all parent directories until the root,
    /// and the first hit is used.
    ///
    /// If this search comes up empty, then it checks if a global `foundry.toml` exists at
    /// `~/.foundry/foundry.tol`, see [`Self::foundry_dir_toml()`]
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
            .or_else(|| Self::foundry_dir_toml().filter(|p| p.exists()))
    }

    /// Clears the foundry cache
    pub fn clean_foundry_cache() -> eyre::Result<()> {
        if let Some(cache_dir) = Config::foundry_cache_dir() {
            let path = cache_dir.as_path();
            let _ = fs::remove_dir_all(path);
        } else {
            eyre::bail!("failed to get foundry_cache_dir");
        }

        Ok(())
    }

    /// Clears the foundry cache for `chain`
    pub fn clean_foundry_chain_cache(chain: Chain) -> eyre::Result<()> {
        if let Some(cache_dir) = Config::foundry_chain_cache_dir(chain) {
            let path = cache_dir.as_path();
            let _ = fs::remove_dir_all(path);
        } else {
            eyre::bail!("failed to get foundry_chain_cache_dir");
        }

        Ok(())
    }

    /// Clears the foundry cache for `chain` and `block`
    pub fn clean_foundry_block_cache(chain: Chain, block: u64) -> eyre::Result<()> {
        if let Some(cache_dir) = Config::foundry_block_cache_dir(chain, block) {
            let path = cache_dir.as_path();
            let _ = fs::remove_dir_all(path);
        } else {
            eyre::bail!("failed to get foundry_block_cache_dir");
        }

        Ok(())
    }

    /// Clears the foundry etherscan cache
    pub fn clean_foundry_etherscan_cache() -> eyre::Result<()> {
        if let Some(cache_dir) = Config::foundry_etherscan_cache_dir() {
            let path = cache_dir.as_path();
            let _ = fs::remove_dir_all(path);
        } else {
            eyre::bail!("failed to get foundry_etherscan_cache_dir");
        }

        Ok(())
    }

    /// Clears the foundry etherscan cache for `chain`
    pub fn clean_foundry_etherscan_chain_cache(chain: Chain) -> eyre::Result<()> {
        if let Some(cache_dir) = Config::foundry_etherscan_chain_cache_dir(chain) {
            let path = cache_dir.as_path();
            let _ = fs::remove_dir_all(path);
        } else {
            eyre::bail!("failed to get foundry_etherscan_cache_dir for chain: {}", chain);
        }

        Ok(())
    }

    /// List the data in the foundry cache
    pub fn list_foundry_cache() -> eyre::Result<Cache> {
        if let Some(cache_dir) = Config::foundry_rpc_cache_dir() {
            let mut cache = Cache { chains: vec![] };
            if !cache_dir.exists() {
                return Ok(cache)
            }
            if let Ok(entries) = cache_dir.as_path().read_dir() {
                for entry in entries.flatten().filter(|x| x.path().is_dir()) {
                    match Chain::from_str(&entry.file_name().to_string_lossy()) {
                        Ok(chain) => cache.chains.push(Self::list_foundry_chain_cache(chain)?),
                        Err(_) => continue,
                    }
                }
                Ok(cache)
            } else {
                eyre::bail!("failed to access foundry_cache_dir");
            }
        } else {
            eyre::bail!("failed to get foundry_cache_dir");
        }
    }

    /// List the cached data for `chain`
    pub fn list_foundry_chain_cache(chain: Chain) -> eyre::Result<ChainCache> {
        let block_explorer_data_size = match Config::foundry_etherscan_chain_cache_dir(chain) {
            Some(cache_dir) => Self::get_cached_block_explorer_data(&cache_dir)?,
            None => eyre::bail!("failed to access foundry_etherscan_chain_cache_dir"),
        };

        if let Some(cache_dir) = Config::foundry_chain_cache_dir(chain) {
            let blocks = Self::get_cached_blocks(&cache_dir)?;
            Ok(ChainCache {
                name: chain.to_string(),
                blocks,
                block_explorer: block_explorer_data_size,
            })
        } else {
            eyre::bail!("failed to get foundry_chain_cache_dir");
        }
    }

    //The path provided to this function should point to a cached chain folder
    fn get_cached_blocks(chain_path: &Path) -> eyre::Result<Vec<(String, u64)>> {
        let mut blocks = vec![];
        if !chain_path.exists() {
            return Ok(blocks)
        }
        for block in chain_path.read_dir()?.flatten().filter(|x| x.file_type().unwrap().is_dir()) {
            let filepath = block.path().join("storage.json");
            blocks.push((
                block.file_name().to_string_lossy().into_owned(),
                fs::metadata(filepath)?.len(),
            ));
        }
        Ok(blocks)
    }

    //The path provided to this function should point to the etherscan cache for a chain
    fn get_cached_block_explorer_data(chain_path: &Path) -> eyre::Result<u64> {
        if !chain_path.exists() {
            return Ok(0)
        }

        fn dir_size_recursive(mut dir: fs::ReadDir) -> eyre::Result<u64> {
            dir.try_fold(0, |acc, file| {
                let file = file?;
                let size = match file.metadata()? {
                    data if data.is_dir() => dir_size_recursive(fs::read_dir(file.path())?)?,
                    data => data.len(),
                };
                Ok(acc + size)
            })
        }

        dir_size_recursive(fs::read_dir(chain_path)?)
    }
}

impl From<Config> for Figment {
    fn from(c: Config) -> Figment {
        let profile = Config::selected_profile();
        let mut figment = Figment::default().merge(DappHardhatDirProvider(&c.__root.0));

        // check global foundry.toml file
        if let Some(global_toml) = Config::foundry_dir_toml().filter(|p| p.exists()) {
            figment = figment.merge(BackwardsCompatTomlProvider(ForcedSnakeCaseData(
                Toml::file(global_toml).nested(),
            )))
        }

        if profile != Config::DEFAULT_PROFILE {
            // a different profile was set: inherit from the `default` profile by merging the
            // default profile of the toml file
            let inherit = InheritProvider {
                provider: BackwardsCompatTomlProvider(ForcedSnakeCaseData(TomlFileProvider::new(
                    "FOUNDRY_CONFIG",
                    c.__root.0.join(Config::FILE_NAME),
                ))),
                parent: Config::DEFAULT_PROFILE,
                profile: profile.clone(),
            };
            figment = figment.merge(inherit);
        }

        figment = figment
            .merge(BackwardsCompatTomlProvider(ForcedSnakeCaseData(TomlFileProvider::new(
                "FOUNDRY_CONFIG",
                c.__root.0.join(Config::FILE_NAME),
            ))))
            .merge(Env::prefixed("DAPP_").ignore(&["REMAPPINGS", "LIBRARIES"]).global())
            .merge(Env::prefixed("DAPP_TEST_").ignore(&["CACHE"]).global())
            .merge(DappEnvCompatProvider)
            .merge(Env::raw().only(&["ETHERSCAN_API_KEY"]))
            .merge(
                Env::prefixed("FOUNDRY_").ignore(&["PROFILE", "REMAPPINGS", "LIBRARIES"]).global(),
            )
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

/// Wrapper type for `regex::Regex` that implements `PartialEq`
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(transparent)]
pub struct RegexWrapper {
    #[serde(with = "serde_regex")]
    inner: regex::Regex,
}

impl std::ops::Deref for RegexWrapper {
    type Target = regex::Regex;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl std::cmp::PartialEq for RegexWrapper {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl From<RegexWrapper> for regex::Regex {
    fn from(wrapper: RegexWrapper) -> Self {
        wrapper.inner
    }
}

impl From<regex::Regex> for RegexWrapper {
    fn from(re: Regex) -> Self {
        RegexWrapper { inner: re }
    }
}

/// Ser/de `globset::Glob` explicitly to handle `Option<Glob>` properly
pub(crate) mod from_opt_glob {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(value: &Option<globset::Glob>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match value {
            Some(glob) => serializer.serialize_str(glob.glob()),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<globset::Glob>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s: Option<String> = Option::deserialize(deserializer)?;
        if let Some(s) = s {
            return Ok(Some(globset::Glob::new(&s).map_err(serde::de::Error::custom)?))
        }
        Ok(None)
    }
}

/// A helper wrapper around the root path used during Config detection
#[derive(Debug, PartialEq, Eq, Hash, Clone, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(transparent)]
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
        let mut data = Serialized::defaults(self).data()?;
        if let Some(entry) = data.get_mut(&self.profile) {
            entry.insert("root".to_string(), Value::serialize(self.__root.clone())?);
        }
        Ok(data)
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
            script: "script".into(),
            out: "out".into(),
            libs: vec!["lib".into()],
            cache: true,
            cache_path: "cache".into(),
            broadcast: "broadcast".into(),
            allow_paths: vec![],
            force: false,
            evm_version: Default::default(),
            gas_reports: vec!["*".to_string()],
            solc: None,
            auto_detect_solc: true,
            offline: false,
            optimizer: true,
            optimizer_runs: 200,
            optimizer_details: None,
            model_checker: None,
            extra_output: Default::default(),
            extra_output_files: Default::default(),
            names: false,
            sizes: false,
            test_pattern: None,
            test_pattern_inverse: None,
            contract_pattern: None,
            contract_pattern_inverse: None,
            path_pattern: None,
            path_pattern_inverse: None,
            fuzz_runs: 256,
            fuzz_max_local_rejects: 1024,
            fuzz_max_global_rejects: 65536,
            ffi: false,
            sender: Config::DEFAULT_SENDER,
            tx_origin: Config::DEFAULT_SENDER,
            initial_balance: U256::from(0xffffffffffffffffffffffffu128),
            block_number: 1,
            fork_block_number: None,
            chain_id: None,
            gas_limit: i64::MAX.into(),
            gas_price: None,
            block_base_fee_per_gas: 0,
            block_coinbase: Address::zero(),
            block_timestamp: 1,
            block_difficulty: 0,
            block_gas_limit: None,
            memory_limit: 2u64.pow(25),
            eth_rpc_url: None,
            etherscan_api_key: None,
            verbosity: 0,
            remappings: vec![],
            libraries: vec![],
            ignored_error_codes: vec![
                SolidityErrorCode::SpdxLicenseNotProvided,
                SolidityErrorCode::ContractExceeds24576Bytes,
            ],
            via_ir: false,
            rpc_storage_caching: Default::default(),
            rpc_endpoints: Default::default(),
            no_storage_caching: false,
            bytecode_hash: BytecodeHash::Ipfs,
            revert_strings: None,
            sparse_mode: false,
            build_info: false,
            build_info_path: None,
            __non_exhaustive: (),
        }
    }
}

/// Wrapper for the config's `gas_limit` value necessary because toml-rs can't handle larger number because integers are stored signed: <https://github.com/alexcrichton/toml-rs/issues/256>
///
/// Due to this limitation this type will be serialized/deserialized as String if it's larger than
/// `i64`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GasLimit(pub u64);

impl From<u64> for GasLimit {
    fn from(gas: u64) -> Self {
        Self(gas)
    }
}
impl From<i64> for GasLimit {
    fn from(gas: i64) -> Self {
        Self(gas as u64)
    }
}
impl From<i32> for GasLimit {
    fn from(gas: i32) -> Self {
        Self(gas as u64)
    }
}
impl From<u32> for GasLimit {
    fn from(gas: u32) -> Self {
        Self(gas as u64)
    }
}

impl From<GasLimit> for u64 {
    fn from(gas: GasLimit) -> Self {
        gas.0
    }
}

impl Serialize for GasLimit {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if self.0 > i64::MAX as u64 {
            serializer.serialize_str(&self.0.to_string())
        } else {
            serializer.serialize_u64(self.0)
        }
    }
}

impl<'de> Deserialize<'de> for GasLimit {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de::Error;

        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Gas {
            Number(u64),
            Text(String),
        }

        let gas = match Gas::deserialize(deserializer)? {
            Gas::Number(num) => GasLimit(num),
            Gas::Text(s) => GasLimit(s.parse().map_err(D::Error::custom)?),
        };

        Ok(gas)
    }
}

/// Variants for selecting the [`Solc`] instance
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SolcReq {
    /// Requires a specific solc version, that's either already installed (via `svm`) or will be
    /// auto installed (via `svm`)
    Version(Version),
    /// Path to an existing local solc installation
    Local(PathBuf),
}

impl<T: AsRef<str>> From<T> for SolcReq {
    fn from(s: T) -> Self {
        let s = s.as_ref();
        if let Ok(v) = Version::from_str(s) {
            SolcReq::Version(v)
        } else {
            SolcReq::Local(s.into())
        }
    }
}

/// A convenience provider to retrieve a toml file.
/// This will return an error if the env var is set but the file does not exist
struct TomlFileProvider {
    pub env_var: &'static str,
    pub default: PathBuf,
}

impl TomlFileProvider {
    fn new(env_var: &'static str, default: impl Into<PathBuf>) -> Self {
        Self { env_var, default: default.into() }
    }

    fn file(&self) -> PathBuf {
        Env::var(self.env_var).map(PathBuf::from).unwrap_or_else(|| self.default.clone())
    }

    fn is_missing(&self) -> bool {
        if let Some(file) = Env::var(self.env_var) {
            let path = Path::new(&file);
            if !path.exists() {
                return true
            }
        }
        false
    }
}

impl Provider for TomlFileProvider {
    fn metadata(&self) -> Metadata {
        if self.is_missing() {
            Metadata::named("TOML file provider")
        } else {
            Toml::file(self.file()).nested().metadata()
        }
    }

    fn data(&self) -> Result<Map<Profile, Dict>, Error> {
        use serde::de::Error as _;

        if let Some(file) = Env::var(self.env_var) {
            let path = Path::new(&file);
            if !path.exists() {
                return Err(Error::custom(format!(
                    "Config file `{}` set in env var `{}` does not exist",
                    file, self.env_var
                )))
            }
            Toml::file(file)
        } else {
            Toml::file(&self.default)
        }
        .nested()
        .data()
    }
}

/// A Provider that ensures all keys are snake case
struct ForcedSnakeCaseData<P>(P);

impl<P: Provider> Provider for ForcedSnakeCaseData<P> {
    fn metadata(&self) -> Metadata {
        self.0.metadata()
    }

    fn data(&self) -> Result<Map<Profile, Dict>, Error> {
        let mut map = Map::new();
        for (profile, dict) in self.0.data()? {
            map.insert(profile, dict.into_iter().map(|(k, v)| (k.to_snake_case(), v)).collect());
        }
        Ok(map)
    }
}

/// A Provider that extracts the data for a `parent` profile and emits that as `profile`.
struct InheritProvider<P> {
    provider: P,
    parent: Profile,
    profile: Profile,
}

impl<P: Provider> Provider for InheritProvider<P> {
    fn metadata(&self) -> Metadata {
        self.provider.metadata()
    }

    fn data(&self) -> Result<Map<Profile, Dict>, Error> {
        let mut data = self.provider.data()?;
        if let Some(data) = data.remove(&self.parent) {
            return Ok(Map::from([(self.profile.clone(), data)]))
        }
        Ok(Default::default())
    }
}

/// A Provider that handles breaking changes in toml files
struct BackwardsCompatTomlProvider<P>(P);

impl<P: Provider> Provider for BackwardsCompatTomlProvider<P> {
    fn metadata(&self) -> Metadata {
        self.0.metadata()
    }

    fn data(&self) -> Result<Map<Profile, Dict>, Error> {
        let mut map = Map::new();
        let solc_env = std::env::var("FOUNDRY_SOLC_VERSION")
            .or_else(|_| std::env::var("DAPP_SOLC_VERSION"))
            .map(Value::from)
            .ok();
        for (profile, mut dict) in self.0.data()? {
            if let Some(v) = solc_env.clone().or_else(|| dict.remove("solc_version")) {
                dict.insert("solc".to_string(), v);
            }
            map.insert(profile, dict);
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

        // detect libs folders:
        //   if `lib` _and_ `node_modules` exists: include both
        //   if only `node_modules` exists: include `node_modules`
        //   include `lib` otherwise
        let mut libs = vec![];
        let node_modules = self.0.join("node_modules");
        let lib = self.0.join("lib");
        if node_modules.exists() {
            if lib.exists() {
                libs.push(lib.file_name().unwrap().to_string_lossy().to_string());
            }
            libs.push(node_modules.file_name().unwrap().to_string_lossy().to_string());
        } else {
            libs.push(lib.file_name().unwrap().to_string_lossy().to_string());
        }

        dict.insert("libs".to_string(), libs.into());

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

        // libraries in env vars either as `[..]` or single string separated by comma
        if let Ok(val) = env::var("DAPP_LIBRARIES").or_else(|_| env::var("FOUNDRY_LIBRARIES")) {
            dict.insert("libraries".to_string(), utils::to_array_value(&val)?);
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
        trace!("get all remappings from {:?}", self.root);
        /// prioritizes remappings that are closer: shorter `path`
        ///   - ("a", "1/2") over ("a", "1/2/3")
        fn insert_closest(mappings: &mut HashMap<String, PathBuf>, key: String, path: PathBuf) {
            match mappings.entry(key) {
                Entry::Occupied(mut e) => {
                    if e.get().components().count() > path.components().count() {
                        e.insert(path);
                    }
                }
                Entry::Vacant(e) => {
                    e.insert(path);
                }
            }
        }

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
            let content = fs::read_to_string(remappings_file).map_err(|err| err.to_string())?;
            let remappings_from_file: Result<Vec<_>, _> =
                remappings_from_newline(&content).collect();
            new_remappings
                .extend(remappings_from_file.map_err::<Error, _>(|err| err.to_string().into())?);
        }

        new_remappings.extend(remappings);

        let mut lib_remappings = HashMap::new();
        // find all remappings of from libs that use a foundry.toml
        for r in self.lib_foundry_toml_remappings() {
            insert_closest(&mut lib_remappings, r.name, r.path.into());
        }
        // use auto detection for all libs
        for r in self
            .lib_paths
            .iter()
            .map(|lib| self.root.join(lib))
            .inspect(|lib| {
                trace!("find all remappings in lib path: {:?}", lib);
            })
            .flat_map(Remapping::find_many)
        {
            // this is an additional safety check for weird auto-detected remappings
            if ["lib/", "src/", "contracts/"].contains(&r.name.as_str()) {
                continue
            }
            insert_closest(&mut lib_remappings, r.name, r.path.into());
        }

        new_remappings.extend(
            lib_remappings
                .into_iter()
                .map(|(name, path)| Remapping { name, path: path.to_string_lossy().into() }),
        );

        // remove duplicates at this point
        new_remappings.sort_by(|a, b| a.name.cmp(&b.name));
        new_remappings.dedup_by(|a, b| a.name.eq(&b.name));

        Ok(new_remappings)
    }

    /// Returns all remappings declared in foundry.toml files of libraries
    fn lib_foundry_toml_remappings(&self) -> impl Iterator<Item = Remapping> + '_ {
        self.lib_paths
            .iter()
            .map(|p| self.root.join(p))
            .flat_map(foundry_toml_dirs)
            .inspect(|lib| {
                trace!("find all remappings of nested foundry.toml lib: {:?}", lib);
            })
            .flat_map(|lib: PathBuf| {
                // load config, of the nested lib if it exists
                let config = Config::load_with_root(&lib).sanitized();

                // if the configured _src_ directory is set to something that
                // [Remapping::find_many()] doesn't classify as a src directory (src, contracts,
                // lib), then we need to manually add a remapping here
                let mut src_remapping = None;
                if ![Path::new("src"), Path::new("contracts"), Path::new("lib")]
                    .contains(&config.src.as_path())
                {
                    if let Some(name) = lib.file_name().and_then(|s| s.to_str()) {
                        let mut r = Remapping {
                            name: format!("{}/", name),
                            path: format!("{}", lib.join(&config.src).display()),
                        };
                        if !r.path.ends_with('/') {
                            r.path.push('/')
                        }
                        src_remapping = Some(r);
                    }
                }

                let mut remappings =
                    config.remappings.into_iter().map(|m| m.into()).collect::<Vec<Remapping>>();

                if let Some(r) = src_remapping {
                    remappings.push(r);
                }
                remappings
            })
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
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct BasicConfig {
    /// the profile tag: `[default]`
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
# See more config options https://github.com/foundry-rs/foundry/tree/master/config"#,
            self.profile, s
        ))
    }
}

pub(crate) mod from_str_lowercase {
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

fn canonic(path: impl Into<PathBuf>) -> PathBuf {
    let path = path.into();
    ethers_solc::utils::canonicalize(&path).unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use ethers_solc::artifacts::{ModelCheckerEngine, YulDetails};
    use figment::error::Kind::InvalidType;
    use std::{collections::BTreeMap, str::FromStr};

    use crate::cache::{CachedChains, CachedEndpoints};
    use figment::{value::Value, Figment};
    use pretty_assertions::assert_eq;

    use super::*;

    use crate::rpc::RpcEndpoint;
    use std::{fs::File, io::Write};
    use tempfile::tempdir;

    #[test]
    fn test_install_dir() {
        figment::Jail::expect_with(|jail| {
            let config = Config::load();
            assert_eq!(config.install_lib_dir(), PathBuf::from("lib"));
            jail.create_file(
                "foundry.toml",
                r#"
                [default]
                libs = ['node_modules', 'lib']
            "#,
            )?;
            let config = Config::load();
            assert_eq!(config.install_lib_dir(), PathBuf::from("lib"));

            jail.create_file(
                "foundry.toml",
                r#"
                [default]
                libs = ['custom', 'node_modules', 'lib']
            "#,
            )?;
            let config = Config::load();
            assert_eq!(config.install_lib_dir(), PathBuf::from("custom"));

            Ok(())
        });
    }

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

            jail.create_file(
                "foundry.toml",
                r#"
                [default]
                libs = ['lib']
                [local]
                libs = ['modules']
            "#,
            )?;
            jail.set_env("FOUNDRY_PROFILE", "local");
            let config = Config::load();
            assert_eq!(config.libs, vec![PathBuf::from("modules")]);

            Ok(())
        });
    }

    #[test]
    fn test_default_test_path() {
        figment::Jail::expect_with(|_| {
            let config = Config::default();
            let paths_config = config.project_paths();
            assert_eq!(paths_config.tests, PathBuf::from(r"test"));
            Ok(())
        });
    }

    #[test]
    fn test_default_libs() {
        figment::Jail::expect_with(|jail| {
            let config = Config::load();
            assert_eq!(config.libs, vec![PathBuf::from("lib")]);

            fs::create_dir_all(jail.directory().join("node_modules")).unwrap();
            let config = Config::load();
            assert_eq!(config.libs, vec![PathBuf::from("node_modules")]);

            fs::create_dir_all(jail.directory().join("lib")).unwrap();
            let config = Config::load();
            assert_eq!(config.libs, vec![PathBuf::from("lib"), PathBuf::from("node_modules")]);

            Ok(())
        });
    }

    #[test]
    fn test_inheritance_from_default_test_path() {
        figment::Jail::expect_with(|jail| {
            jail.create_file(
                "foundry.toml",
                r#"
                [default]
                test = "defaulttest"
                src  = "defaultsrc"
                libs = ['lib', 'node_modules']
                
                [custom]
                src = "customsrc"
            "#,
            )?;

            let config = Config::load();
            assert_eq!(config.src, PathBuf::from("defaultsrc"));
            assert_eq!(config.libs, vec![PathBuf::from("lib"), PathBuf::from("node_modules")]);

            jail.set_env("FOUNDRY_PROFILE", "custom");
            let config = Config::load();

            assert_eq!(config.src, PathBuf::from("customsrc"));
            assert_eq!(config.test, PathBuf::from("defaulttest"));
            assert_eq!(config.libs, vec![PathBuf::from("lib"), PathBuf::from("node_modules")]);

            Ok(())
        });
    }

    #[test]
    fn test_custom_test_path() {
        figment::Jail::expect_with(|jail| {
            jail.create_file(
                "foundry.toml",
                r#"
                [default]
                test = "mytest"
            "#,
            )?;

            let config = Config::load();
            let paths_config = config.project_paths();
            assert_eq!(paths_config.tests, PathBuf::from(r"mytest"));
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

            // contains additional remapping to the source dir
            assert_eq!(
                config.get_all_remappings(),
                vec![
                    Remapping::from_str("ds-test/=lib/ds-test/src/").unwrap(),
                    Remapping::from_str("env-lib/=lib/env-lib/").unwrap(),
                    Remapping::from_str("other/=lib/other/").unwrap(),
                    Remapping::from_str("some-source/=some-source/").unwrap(),
                ],
            );

            Ok(())
        });
    }

    #[test]
    fn test_can_update_libs() {
        figment::Jail::expect_with(|jail| {
            jail.create_file(
                "foundry.toml",
                r#"
                [default]
                libs = ["node_modules"]
            "#,
            )?;

            let mut config = Config::load();
            config.libs.push("libs".into());
            config.update_libs().unwrap();

            let config = Config::load();
            assert_eq!(config.libs, vec![PathBuf::from("node_modules"), PathBuf::from("libs"),]);
            Ok(())
        });
    }

    #[test]
    fn test_large_gas_limit() {
        figment::Jail::expect_with(|jail| {
            let gas = u64::MAX;
            jail.create_file(
                "foundry.toml",
                &format!(
                    r#"
                [default]
                gas_limit = "{}"
            "#,
                    gas
                ),
            )?;

            let config = Config::load();
            assert_eq!(config, Config { gas_limit: gas.into(), ..Config::default() });

            Ok(())
        });
    }

    #[test]
    #[should_panic]
    fn test_toml_file_parse_failure() {
        figment::Jail::expect_with(|jail| {
            jail.create_file(
                "foundry.toml",
                r#"
                [default]
                eth_rpc_url = "https://example.com/
            "#,
            )?;

            let _config = Config::load();

            Ok(())
        });
    }

    #[test]
    #[should_panic]
    fn test_toml_file_non_existing_config_var_failure() {
        figment::Jail::expect_with(|jail| {
            jail.set_env("FOUNDRY_CONFIG", "this config does not exist");

            let _config = Config::load();

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
                via_ir = true
                rpc_storage_caching = { chains = [1, "optimism", 999999], endpoints = "all"}
                rpc_endpoints = { optimism = "https://example.com/", mainnet = "${RPC_MAINNET}" }
                bytecode_hash = "ipfs"
                revert_strings = "strip"
                allow_paths = ["allow", "paths"]
                build_info_path = "build-info"
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
                    via_ir: true,
                    rpc_storage_caching: StorageCachingConfig {
                        chains: CachedChains::Chains(vec![
                            Chain::Named(ethers_core::types::Chain::Mainnet),
                            Chain::Named(ethers_core::types::Chain::Optimism),
                            Chain::Id(999999)
                        ]),
                        endpoints: CachedEndpoints::All
                    },
                    bytecode_hash: BytecodeHash::Ipfs,
                    revert_strings: Some(RevertStrings::Strip),
                    allow_paths: vec![PathBuf::from("allow"), PathBuf::from("paths")],
                    rpc_endpoints: RpcEndpoints::new([
                        ("optimism", RpcEndpoint::Url("https://example.com/".to_string())),
                        ("mainnet", RpcEndpoint::Env("RPC_MAINNET".to_string()))
                    ]),
                    build_info_path: Some("build-info".into()),
                    ..Config::default()
                }
            );

            Ok(())
        });
    }

    #[test]
    fn test_load_remappings() {
        figment::Jail::expect_with(|jail| {
            jail.create_file(
                "foundry.toml",
                r#"
                [default]
                remappings = ['nested/=lib/nested/']
            "#,
            )?;

            let config = Config::load_with_root(jail.directory());
            assert_eq!(
                config.remappings,
                vec![Remapping::from_str("nested/=lib/nested/").unwrap().into()]
            );

            Ok(())
        });
    }

    #[test]
    fn test_load_full_toml() {
        figment::Jail::expect_with(|jail| {
            jail.create_file(
                "foundry.toml",
                r#"
                [default]
                auto_detect_solc = true
                block_base_fee_per_gas = 0
                block_coinbase = '0x0000000000000000000000000000000000000000'
                block_difficulty = 0
                block_number = 1
                block_timestamp = 1
                bytecode_hash = 'ipfs'
                cache = true
                cache_path = 'cache'
                evm_version = 'london'
                extra_output = []
                extra_output_files = []
                ffi = false
                force = false
                fuzz_max_global_rejects = 65536
                fuzz_max_local_rejects = 1024
                fuzz_runs = 256
                gas_limit = 9223372036854775807
                gas_price = 0
                gas_reports = ['*']
                ignored_error_codes = [1878]
                initial_balance = '0xffffffffffffffffffffffff'
                libraries = []
                libs = ['lib']
                memory_limit = 33554432
                names = false
                no_storage_caching = false
                offline = false
                optimizer = true
                optimizer_runs = 200
                out = 'out'
                remappings = ['nested/=lib/nested/']
                sender = '0x00a329c0648769a73afac7f9381e08fb43dbea72'
                sizes = false
                sparse_mode = false
                src = 'src'
                test = 'test'
                tx_origin = '0x00a329c0648769a73afac7f9381e08fb43dbea72'
                verbosity = 0
                via_ir = false
                
                [default.rpc_storage_caching]
                chains = 'all'
                endpoints = 'all'

                [default.rpc_endpoints]
                optimism = "https://example.com/"
                mainnet = "${RPC_MAINNET}"

            "#,
            )?;

            let config = Config::load_with_root(jail.directory());
            assert_eq!(
                config.remappings,
                vec![Remapping::from_str("nested/=lib/nested/").unwrap().into()]
            );

            assert_eq!(
                config.rpc_endpoints,
                RpcEndpoints::new([
                    ("optimism", RpcEndpoint::Url("https://example.com/".to_string())),
                    ("mainnet", RpcEndpoint::Env("RPC_MAINNET".to_string()))
                ]),
            );

            Ok(())
        });
    }

    #[test]
    fn test_solc_req() {
        figment::Jail::expect_with(|jail| {
            jail.create_file(
                "foundry.toml",
                r#"
                [default]
                solc_version = "0.8.12"
            "#,
            )?;

            let config = Config::load();
            assert_eq!(config.solc, Some(SolcReq::Version("0.8.12".parse().unwrap())));

            jail.create_file(
                "foundry.toml",
                r#"
                [default]
                solc = "0.8.12"
            "#,
            )?;

            let config = Config::load();
            assert_eq!(config.solc, Some(SolcReq::Version("0.8.12".parse().unwrap())));

            jail.create_file(
                "foundry.toml",
                r#"
                [default]
                solc = "path/to/local/solc"
            "#,
            )?;

            let config = Config::load();
            assert_eq!(config.solc, Some(SolcReq::Local("path/to/local/solc".into())));

            jail.set_env("FOUNDRY_SOLC_VERSION", "0.6.6");
            let config = Config::load();
            assert_eq!(config.solc, Some(SolcReq::Version("0.6.6".parse().unwrap())));
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
    fn test_output_selection() {
        figment::Jail::expect_with(|jail| {
            jail.create_file(
                "foundry.toml",
                r#"
                [default]
                extra_output = ["metadata", "ir-optimized"]
                extra_output_files = ["metadata"]
            "#,
            )?;

            let config = Config::load();

            assert_eq!(
                config.extra_output,
                vec![ContractOutputSelection::Metadata, ContractOutputSelection::IrOptimized]
            );
            assert_eq!(config.extra_output_files, vec![ContractOutputSelection::Metadata]);

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
                    out: "myout".into(),
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
    fn can_parse_libraries() {
        figment::Jail::expect_with(|jail| {
            jail.set_env(
                "DAPP_LIBRARIES",
                "[src/DssSpell.sol:DssExecLib:0x8De6DDbCd5053d32292AAA0D2105A32d108484a6]",
            );
            let config = Config::load();
            assert_eq!(
                config.libraries,
                vec!["src/DssSpell.sol:DssExecLib:0x8De6DDbCd5053d32292AAA0D2105A32d108484a6"
                    .to_string()]
            );

            jail.set_env(
                "DAPP_LIBRARIES",
                "src/DssSpell.sol:DssExecLib:0x8De6DDbCd5053d32292AAA0D2105A32d108484a6",
            );
            let config = Config::load();
            assert_eq!(
                config.libraries,
                vec!["src/DssSpell.sol:DssExecLib:0x8De6DDbCd5053d32292AAA0D2105A32d108484a6"
                    .to_string(),]
            );

            jail.set_env(
                "DAPP_LIBRARIES",
                "src/DssSpell.sol:DssExecLib:0x8De6DDbCd5053d32292AAA0D2105A32d108484a6,src/DssSpell.sol:DssExecLib:0x8De6DDbCd5053d32292AAA0D2105A32d108484a6",
            );
            let config = Config::load();
            assert_eq!(
                config.libraries,
                vec![
                    "src/DssSpell.sol:DssExecLib:0x8De6DDbCd5053d32292AAA0D2105A32d108484a6"
                        .to_string(),
                    "src/DssSpell.sol:DssExecLib:0x8De6DDbCd5053d32292AAA0D2105A32d108484a6"
                        .to_string()
                ]
            );

            Ok(())
        });
    }

    #[test]
    fn test_parse_many_libraries() {
        figment::Jail::expect_with(|jail| {
            jail.create_file(
                "foundry.toml",
                r#"
                [default]
               libraries= [
                        './src/SizeAuctionDiscount.sol:Chainlink:0xffedba5e171c4f15abaaabc86e8bd01f9b54dae5',
                        './src/SizeAuction.sol:ChainlinkTWAP:0xffedba5e171c4f15abaaabc86e8bd01f9b54dae5',
                        './src/SizeAuction.sol:Math:0x902f6cf364b8d9470d5793a9b2b2e86bddd21e0c',
                        './src/test/ChainlinkTWAP.t.sol:ChainlinkTWAP:0xffedba5e171c4f15abaaabc86e8bd01f9b54dae5',
                        './src/SizeAuctionDiscount.sol:Math:0x902f6cf364b8d9470d5793a9b2b2e86bddd21e0c',
                    ]       
            "#,
            )?;
            let config = Config::load();

            let libs = config.parsed_libraries().unwrap().libs;

            pretty_assertions::assert_eq!(
                libs,
                BTreeMap::from([
                    (
                        PathBuf::from("./src/SizeAuctionDiscount.sol"),
                        BTreeMap::from([
                            (
                                "Chainlink".to_string(),
                                "0xffedba5e171c4f15abaaabc86e8bd01f9b54dae5".to_string()
                            ),
                            (
                                "Math".to_string(),
                                "0x902f6cf364b8d9470d5793a9b2b2e86bddd21e0c".to_string()
                            )
                        ])
                    ),
                    (
                        PathBuf::from("./src/SizeAuction.sol"),
                        BTreeMap::from([
                            (
                                "ChainlinkTWAP".to_string(),
                                "0xffedba5e171c4f15abaaabc86e8bd01f9b54dae5".to_string()
                            ),
                            (
                                "Math".to_string(),
                                "0x902f6cf364b8d9470d5793a9b2b2e86bddd21e0c".to_string()
                            )
                        ])
                    ),
                    (
                        PathBuf::from("./src/test/ChainlinkTWAP.t.sol"),
                        BTreeMap::from([(
                            "ChainlinkTWAP".to_string(),
                            "0xffedba5e171c4f15abaaabc86e8bd01f9b54dae5".to_string()
                        )])
                    ),
                ])
            );

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

            Ok(())
        });
    }

    #[test]
    fn test_optimizer_settings_basic() {
        figment::Jail::expect_with(|jail| {
            jail.create_file(
                "foundry.toml",
                r#"
                [default]
                optimizer = true

                [default.optimizer_details]
                yul = false

                [default.optimizer_details.yulDetails]
                stackAllocation = true
            "#,
            )?;
            let loaded = Config::load();
            assert_eq!(
                loaded.optimizer_details,
                Some(OptimizerDetails {
                    yul: Some(false),
                    yul_details: Some(YulDetails {
                        stack_allocation: Some(true),
                        ..Default::default()
                    }),
                    ..Default::default()
                })
            );

            let s = loaded.to_string_pretty().unwrap();
            jail.create_file("foundry.toml", &s)?;

            let reloaded = Config::load();
            assert_eq!(loaded, reloaded);

            Ok(())
        });
    }

    #[test]
    fn test_model_checker_settings_basic() {
        figment::Jail::expect_with(|jail| {
            jail.create_file(
                "foundry.toml",
                r#"
                [default]

                [default.model_checker]
                contracts = { 'a.sol' = [ 'A1', 'A2' ], 'b.sol' = [ 'B1', 'B2' ] }
                engine = 'chc'
                targets = [ 'assert', 'outOfBounds' ]
                timeout = 10000
            "#,
            )?;
            let loaded = Config::load();
            assert_eq!(
                loaded.model_checker,
                Some(ModelCheckerSettings {
                    contracts: BTreeMap::from([
                        ("a.sol".to_string(), vec!["A1".to_string(), "A2".to_string()]),
                        ("b.sol".to_string(), vec!["B1".to_string(), "B2".to_string()]),
                    ]),
                    engine: Some(ModelCheckerEngine::CHC),
                    targets: Some(vec![
                        ModelCheckerTarget::Assert,
                        ModelCheckerTarget::OutOfBounds
                    ]),
                    timeout: Some(10000)
                })
            );

            let s = loaded.to_string_pretty().unwrap();
            jail.create_file("foundry.toml", &s)?;

            let reloaded = Config::load();
            assert_eq!(loaded, reloaded);

            Ok(())
        });
    }

    #[test]
    fn test_model_checker_settings_relative_paths() {
        figment::Jail::expect_with(|jail| {
            jail.create_file(
                "foundry.toml",
                r#"
                [default]

                [default.model_checker]
                contracts = { 'a.sol' = [ 'A1', 'A2' ], 'b.sol' = [ 'B1', 'B2' ] }
                engine = 'chc'
                targets = [ 'assert', 'outOfBounds' ]
                timeout = 10000
            "#,
            )?;
            let loaded = Config::load().sanitized();

            // NOTE(onbjerg): We have to canonicalize the path here using dunce because figment will
            // canonicalize the jail path using the standard library. The standard library *always*
            // transforms Windows paths to some weird extended format, which none of our code base
            // does.
            let dir = ethers_solc::utils::canonicalize(jail.directory())
                .expect("Could not canonicalize jail path");
            assert_eq!(
                loaded.model_checker,
                Some(ModelCheckerSettings {
                    contracts: BTreeMap::from([
                        (
                            format!("{}", dir.join("a.sol").display()),
                            vec!["A1".to_string(), "A2".to_string()]
                        ),
                        (
                            format!("{}", dir.join("b.sol").display()),
                            vec!["B1".to_string(), "B2".to_string()]
                        ),
                    ]),
                    engine: Some(ModelCheckerEngine::CHC),
                    targets: Some(vec![
                        ModelCheckerTarget::Assert,
                        ModelCheckerTarget::OutOfBounds
                    ]),
                    timeout: Some(10000)
                })
            );

            Ok(())
        });
    }

    // a test to print the config, mainly used to update the example config in the README
    #[test]
    #[ignore]
    fn print_config() {
        let config = Config {
            optimizer_details: Some(OptimizerDetails {
                peephole: None,
                inliner: None,
                jumpdest_remover: None,
                order_literals: None,
                deduplicate: None,
                cse: None,
                constant_optimizer: Some(true),
                yul: Some(true),
                yul_details: Some(YulDetails {
                    stack_allocation: None,
                    optimizer_steps: Some("dhfoDgvulfnTUtnIf".to_string()),
                }),
            }),
            ..Default::default()
        };
        println!("{}", config.to_string_pretty().unwrap());
    }

    #[test]
    fn can_use_impl_figment_macro() {
        #[derive(Default, Serialize)]
        struct MyArgs {
            #[serde(skip_serializing_if = "Option::is_none")]
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

    #[test]
    fn list_cached_blocks() -> eyre::Result<()> {
        fn fake_block_cache(chain_path: &Path, block_number: &str, size_bytes: usize) {
            let block_path = chain_path.join(block_number);
            fs::create_dir(block_path.as_path()).unwrap();
            let file_path = block_path.join("storage.json");
            let mut file = File::create(file_path).unwrap();
            writeln!(file, "{}", vec![' '; size_bytes - 1].iter().collect::<String>()).unwrap();
        }

        let chain_dir = tempdir()?;

        fake_block_cache(chain_dir.path(), "1", 100);
        fake_block_cache(chain_dir.path(), "2", 500);
        // Pollution file that should not show up in the cached block
        let mut pol_file = File::create(chain_dir.path().join("pol.txt")).unwrap();
        writeln!(pol_file, "{}", vec![' '; 10].iter().collect::<String>()).unwrap();

        let result = Config::get_cached_blocks(chain_dir.path())?;

        assert_eq!(result.len(), 2);
        let block1 = &result.iter().find(|x| x.0 == "1").unwrap();
        let block2 = &result.iter().find(|x| x.0 == "2").unwrap();
        assert_eq!(block1.0, "1");
        assert_eq!(block1.1, 100);
        assert_eq!(block2.0, "2");
        assert_eq!(block2.1, 500);

        chain_dir.close()?;
        Ok(())
    }

    #[test]
    fn list_etherscan_cache() -> eyre::Result<()> {
        fn fake_etherscan_cache(chain_path: &Path, address: &str, size_bytes: usize) {
            let metadata_path = chain_path.join("sources");
            let abi_path = chain_path.join("abi");
            let _ = fs::create_dir(metadata_path.as_path());
            let _ = fs::create_dir(abi_path.as_path());

            let metadata_file_path = metadata_path.join(address);
            let mut metadata_file = File::create(metadata_file_path).unwrap();
            writeln!(metadata_file, "{}", vec![' '; size_bytes / 2 - 1].iter().collect::<String>())
                .unwrap();

            let abi_file_path = abi_path.join(address);
            let mut abi_file = File::create(abi_file_path).unwrap();
            writeln!(abi_file, "{}", vec![' '; size_bytes / 2 - 1].iter().collect::<String>())
                .unwrap();
        }

        let chain_dir = tempdir()?;

        fake_etherscan_cache(chain_dir.path(), "1", 100);
        fake_etherscan_cache(chain_dir.path(), "2", 500);

        let result = Config::get_cached_block_explorer_data(chain_dir.path())?;

        assert_eq!(result, 600);

        chain_dir.close()?;
        Ok(())
    }
}
