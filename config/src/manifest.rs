use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Contains all the information about a foundry project, as loaded from a `foundry.toml`.
///
/// Compared to the core `Config` type, this type represents the `foundry.toml` as is where `Config`
/// represents a single profile and all resolved settings, this includes settings from global
/// foundry.toml, env vars
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Manifest {
    pub dependencies: Option<BTreeMap<String, FoundryDependency>>,
    pub profiles: Option<FoundryProfiles>,
    // TODO add standalone entries, like rpc_endpoints
}

/// Represents a dependency entry
#[derive(Clone, Debug, Serialize)]
#[serde(untagged)]
pub enum FoundryDependency {
    /// `dependency = "org/name@tag"`
    Simple(String),
    /// Provides additional settings, such as path
    Detailed(DetailedFoundryDependency),
}

#[derive(Deserialize, Serialize, Clone, Debug, Default)]
#[serde(rename_all = "kebab-case")]
pub struct DetailedFoundryDependency {
    version: Option<String>,
    registry: Option<String>,
    /// The _relative_ path to a  dependency.
    path: Option<String>,
    git: Option<String>,
    branch: Option<String>,
    tag: Option<String>,
    rev: Option<String>,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default)]
pub struct FoundryProfiles(BTreeMap<String, FoundryProfile>);

#[derive(Deserialize, Serialize, Clone, Debug, Default, Eq, PartialEq)]
#[serde(default, rename_all = "kebab-case")]
pub struct FoundryProfile {
    // TODO basically an excerpt of `Config` but everything optional
}
