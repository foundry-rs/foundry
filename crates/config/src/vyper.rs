//! Vyper specific configuration types.

use foundry_compilers::artifacts::vyper::{
    VyperOptimizationLevel, VyperOptimizationMode, VyperVenomSettings,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Default, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VyperConfig {
    /// Vyper optimization mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub optimize: Option<VyperOptimizationMode>,
    /// Vyper numeric optimization level.
    #[serde(default, alias = "optLevel", skip_serializing_if = "Option::is_none")]
    pub opt_level: Option<VyperOptimizationLevel>,
    /// The Vyper instance to use if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    /// Enables Vyper's experimental code generation pipeline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub experimental_codegen: Option<bool>,
    /// Enables Vyper's Venom pipeline through the `venomExperimental` standard-json setting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub venom_experimental: Option<bool>,
    /// Compile in debug mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debug: Option<bool>,
    /// Re-enable the Vyper decimal type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enable_decimals: Option<bool>,
    /// Fine-grained Venom optimizer settings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub venom: Option<VyperVenomSettings>,
}
