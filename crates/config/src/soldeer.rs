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

/// Location where to store the remappings, either in `remappings.txt` or in the `foundry.toml`
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum RemappingsLocation {
    #[default]
    Txt,
    Config,
}

fn default_true() -> bool {
    true
}

/// Type for Soldeer configs, under soldeer tag in the foundry.toml
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SoldeerConfig {
    #[serde(default = "default_true")]
    pub remappings_generate: bool,

    #[serde(default)]
    pub remappings_regenerate: bool,

    #[serde(default = "default_true")]
    pub remappings_version: bool,

    #[serde(default)]
    pub remappings_prefix: String,

    #[serde(default)]
    pub remappings_location: RemappingsLocation,

    #[serde(default)]
    pub recursive_deps: bool,
}

impl AsRef<Self> for SoldeerConfig {
    fn as_ref(&self) -> &Self {
        self
    }
}
impl Default for SoldeerConfig {
    fn default() -> Self {
        Self {
            remappings_generate: true,
            remappings_regenerate: false,
            remappings_version: true,
            remappings_prefix: String::new(),
            remappings_location: Default::default(),
            recursive_deps: false,
        }
    }
}
