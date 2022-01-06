//! foundry configuration.
use ethers_core::types::Address;
use ethers_solc::{remappings::Remapping, ProjectPathsConfig};
use figment::{
    providers::{Env, Format, Serialized, Toml},
    value::{Dict, Map},
    Figment, Metadata, Profile, Provider,
};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Foundry configuration
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
    /// whether to enable cache
    pub cache: bool,
    /// concrete solc version to use if any,
    pub solc_version: Option<Version>,
    /// Whether to activate optimizer
    pub optimizer: bool,
    /// Sets the optimizer runs
    pub optimizer_runs: usize,
    /// Settings to pass to the `solc` compiler input
    // TODO make this more flexible https://stackoverflow.com/questions/48998034/does-toml-support-nested-arrays-of-objects-tables
    pub solc_settings: serde_json::Value,
    /// url of the rpc server that should be used for any rpc calls
    pub eth_rpc_url: Option<String>,
    /// verbosity to use
    pub verbosity: u8,
    /// `Remappings` to use for this repo
    pub remappings: Vec<Remapping>,
    /// library addresses to link
    pub libraries: Vec<Address>,
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
    pub fn load() -> Result<Self, figment::Error> {
        Config::figment().extract()
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
        let mut config = Config::default();
        // autodetect paths
        let paths = ProjectPathsConfig::builder().build_with_root(root);

        config.src = paths.sources.file_name().unwrap().into();
        config.out = paths.artifacts.file_name().unwrap().into();
        config.libs =
            paths.libraries.into_iter().map(|lib| lib.file_name().unwrap().into()).collect();
        config.remappings = paths.remappings;

        config
    }

    /// Returns the default config but with hardhat paths
    pub fn hardhat() -> Self {
        let mut config = Config::default();
        config.src = "contracts".into();
        config.out = "artifacts".into();
        config.libs = vec!["node_modules".into()];

        config
    }
}

impl Into<Figment> for Config {
    fn into(self) -> Figment {
        Figment::from(self)
            .merge(Toml::file(Env::var_or("FOUNDRY_CONFIG", Self::FILE_NAME)).nested())
            .merge(Env::prefixed("FOUNDRY_").ignore(&["PROFILE"]).global())
            .select(Profile::from_env_or("FOUNDRY_PROFILE", Self::DEFAULT_PROFILE))
    }
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
            src: "src".into(),
            test: "test".into(),
            out: "out".into(),
            libs: vec!["lib".into()],
            cache: true,
            solc_version: None,
            optimizer: false,
            optimizer_runs: 200,
            solc_settings: serde_json::json!({
               "*":{
                  "*":[
                     "abi",
                     "evm.bytecode",
                     "evm.deployedBytecode",
                     "evm.methodIdentifiers"
                  ],
                  "":[
                     "ast"
                  ]
               }
            }),
            eth_rpc_url: None,
            verbosity: 0,
            remappings: vec![],
            libraries: vec![],
        }
    }
}
