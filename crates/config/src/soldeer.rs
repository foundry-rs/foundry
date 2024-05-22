//! Configuration specific to the `forge soldeer` command and the `forge_soldeer` package

use serde::{Deserialize, Serialize};

/// Soldeer dependencies config structure
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoldeerDependency {
    /// The version of the dependency
    pub version: String,

    /// The url from where the dependency was retrieved
    pub url: String,
}

