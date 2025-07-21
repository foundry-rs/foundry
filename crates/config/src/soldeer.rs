//! Configuration specific to the `forge soldeer` command and the `forge_soldeer` package
use serde::{Deserialize, Serialize};
pub use soldeer_core::config::SoldeerConfig;
use std::collections::BTreeMap;

/// Soldeer dependencies config structure when it's defined as a map
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapDependency {
    /// The version of the dependency
    pub version: String,

    /// The url from where the dependency was retrieved
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,

    /// The git URL for the source repo
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git: Option<String>,

    /// The commit in case git is used as dependency source
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rev: Option<String>,

    /// The branch in case git is used as dependency source
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,

    /// The git tag in case git is used as dependency source
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
}

/// Type for Soldeer configs, under dependencies tag in the foundry.toml
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoldeerDependencyConfig(BTreeMap<String, SoldeerDependencyValue>);

impl AsRef<Self> for SoldeerDependencyConfig {
    fn as_ref(&self) -> &Self {
        self
    }
}

/// Enum to cover both available formats for defining a dependency
/// `dep = { version = "1.1", url = "https://my-dependency" }`
/// or
/// `dep = "1.1"`
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SoldeerDependencyValue {
    Map(MapDependency),
    Str(String),
}
