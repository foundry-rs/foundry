//! foundry configuration.
use ethers_core::types::Address;
use figment::{
    error::Result,
    providers::{Env, Format, Serialized, Toml},
    value::{magic::RelativePathBuf, Dict, Map},
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
    /// concrete solc version to use if any,
    pub solc_version: Option<Version>,
    /// Whether to activate optimizer
    pub optimizer: bool,
    /// Sets the optimizer runs
    pub optimizer_runs: usize,
    /// Settings to pass to the `solc` compiler input
    pub solc_settings: serde_json::Value,
    /// url of the rpc server that should be used for any rpc calls
    pub eth_rpc_url: Option<String>,
    /// verbosity to use
    pub verbosity: u8,
    /// `Remappings` to use for this repo
    pub remappings: Vec<String>,
    /// library addresses to link
    pub libraries: Vec<Address>,
}

impl Config {
    /// The default profile: "default"
    pub const DEFAULT_PROFILE: Profile = Profile::const_new("default");

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
        Figment::from(Config::default())
            .merge(Toml::file(Env::var_or("FOUNDRY_CONFIG", "foundry.toml")).nested())
            .merge(Env::prefixed("FOUNDRY_").ignore(&["PROFILE"]).global())
            .select(Profile::from_env_or("FOUNDRY_PROFILE", Self::DEFAULT_PROFILE))
    }
}

impl Default for Config {
    fn default() -> Self {
        todo!()
    }
}
