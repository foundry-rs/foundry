use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Strategy for extending configuration from a base file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExtendStrategy {
    /// Uses `admerge` figment strategy.
    /// Arrays are concatenated (base elements + local elements).
    /// Other values are replaced (local values override base values).
    #[default]
    ExtendArrays,

    /// Uses `merge` figment strategy.
    /// Arrays are replaced entirely (local arrays replace base arrays).
    /// Other values are replaced (local values override base values).
    ReplaceArrays,

    /// Throws an error if any of the keys in the inherited toml file are also in `foundry.toml`.
    NoCollision,
}

/// Configuration for extending from a base file.
///
/// Supports two formats:
/// - String: `extends = "base.toml"`
/// - Object: `extends = { path = "base.toml", strategy = "no-collision" }`
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Extends {
    /// Simple string path to base file
    Path(String),
    /// Detailed configuration with path and strategy
    Config(ExtendConfig),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExtendConfig {
    pub path: String,
    #[serde(default)]
    pub strategy: Option<ExtendStrategy>,
}

impl Extends {
    /// Get the path to the base file
    pub fn path(&self) -> &str {
        match self {
            Self::Path(path) => path,
            Self::Config(config) => &config.path,
        }
    }

    /// Get the strategy to use for extending
    pub fn strategy(&self) -> ExtendStrategy {
        match self {
            Self::Path(_) => ExtendStrategy::default(),
            Self::Config(config) => config.strategy.unwrap_or_default(),
        }
    }
}

// -- HELPERS -----------------------------------------------------------------

// Helper structs to only extract the 'extends' field and its strategy from the profiles
#[derive(Deserialize, Default)]
pub(crate) struct ExtendsPartialConfig {
    #[serde(default)]
    pub profile: Option<HashMap<String, ExtendsHelper>>,
}

#[derive(Deserialize, Default)]
pub(crate) struct ExtendsHelper {
    #[serde(default)]
    pub extends: Option<Extends>,
}
