//! Configuration specific to the `forge soldeer` command and the `forge_soldeer` package

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Soldeer dependencies config structure
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapDependency {
    /// The version of the dependency
    pub version: String,

    /// The url from where the dependency was retrieved
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// Type for Soldeer configs, under dependencies tag in the foundry.toml
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoldeerConfig(BTreeMap<String, SoldeerDependencyValue>);

impl AsRef<Self> for SoldeerConfig {
    fn as_ref(&self) -> &Self {
        self
    }
}


#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SoldeerDependencyValue {
    Map(MapDependency),
    Str(String),
}
