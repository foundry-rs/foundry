use ethers_solc::remappings::RelativeRemapping;
use semver::Version;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Contains all the information about a foundry project, as loaded from a `foundry.toml`.
///
/// Compared to the core `Config` type, this type represents the `foundry.toml` as is where `Config`
/// represents a single profile and all resolved settings, this includes settings from global
/// foundry.toml, env vars
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Manifest {
    /// All dependencies declared in the `foundry.toml`
    pub dependencies: Option<BTreeMap<String, FoundryDependency>>,
    /// All declared profiles
    pub profiles: Option<FoundryProfiles>,
    // TODO add standalone entries, like rpc_endpoints
}

/// Represents a dependency entry
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum FoundryDependency {
    /// `dependency = "org/name@tag"`
    Simple(String),
    /// Provides additional settings, such as path
    Detailed(DetailedFoundryDependency),
}

/// Represents detailed settings for a dependency
#[derive(Deserialize, Serialize, Clone, Debug, Default)]
pub struct DetailedFoundryDependency {
    /// The version of the dependency
    pub version: Option<Version>,
    /// The _relative_ path to a  dependency.
    pub path: Option<String>,
    /// URL where this dependency can be found
    pub git: Option<String>,
    /// branch of the `git` repository
    pub branch: Option<String>,
    /// tag of the `git` repository
    pub tag: Option<String>,
    /// commit of the `git` repository
    pub rev: Option<String>,
    /// The remappings to use for this repository
    #[serde(alias = "remappings")]
    pub remappings: Option<TomlRemappings>,
}

/// Represents the remappings a dependency provides
#[derive(Deserialize, Serialize, Clone, Debug, Eq, PartialEq)]
#[serde(untagged)]
pub enum TomlRemappings {
    /// A single remapping for the dependency
    Single(RelativeRemapping),
    /// Multiple remappings, to account for nested submodules
    Multiple(Vec<RelativeRemapping>),
}

/// Represents a set of profiles in a `foundry.toml`
#[derive(Deserialize, Serialize, Clone, Debug, Default)]
pub struct FoundryProfiles(BTreeMap<String, FoundryProfile>);

/// A single profile in a `foundry.toml`
///
/// This is essentially an excerpt of `crate::Config`
#[derive(Deserialize, Serialize, Clone, Debug, Default, Eq, PartialEq)]
#[serde(default, rename_all = "kebab-case")]
pub struct FoundryProfile {
    // TODO basically an excerpt of `Config` but everything optional
}
