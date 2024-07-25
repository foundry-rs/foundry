//! Configuration specific to the `forge soldeer` command and the `forge_soldeer` package

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Soldeer dependencies config structure when it's defined as a map
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapDependency {
    /// The version of the dependency
    pub version: String,

    /// The url from where the dependency was retrieved
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,

    /// The commit in case git is used as dependency retrieval
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rev: Option<String>,
}

/// Type for Soldeer configs, under dependencies tag in the foundry.toml
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoldeerConfig(BTreeMap<String, SoldeerDependencyValue>);

impl AsRef<Self> for SoldeerConfig {
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
