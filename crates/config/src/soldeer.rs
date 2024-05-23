//! Configuration specific to the `forge soldeer` command and the `forge_soldeer` package

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Soldeer dependencies config structure
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoldeerDependency {
    /// The version of the dependency
    pub version: String,

    /// The url from where the dependency was retrieved
    pub url: String,
}

/// Type for Soldeer configs, under dependencies tag in the foundry.toml
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SoldeerConfig(BTreeMap<String, SoldeerDependency>);
impl AsRef<SoldeerConfig> for SoldeerConfig {
    fn as_ref(&self) -> &SoldeerConfig {
        self
    }
}
