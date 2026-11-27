//! Vyper specific configuration types.

use foundry_compilers::artifacts::vyper::{
    VyperOptimizationLevel, VyperOptimizationMode, VyperVenomSettings,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Configuration for compiling Vyper sources.
#[derive(Default, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VyperConfig {
    /// Vyper optimization mode, passed to the compiler as `optimize`.
    ///
    /// - `gas`: optimize for gas.
    /// - `codesize`: optimize for code size.
    /// - `none`: disable optimizations.
    /// - `O1`, `O2`, `O3`, `Os`: numeric optimization levels (`1`, `2`, `3` and `s`, or lowercase
    ///   forms, are accepted too).
    ///
    /// Ignored when `opt_level` is set. When unset, the compiler's default applies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub optimize: Option<VyperOptimizationMode>,
    /// Vyper optimization level, passed to the compiler as `optLevel`.
    ///
    /// Accepts the same values as `optimize` and takes precedence over it, so a profile can
    /// override an inherited `optimize`. `optLevel` is accepted as an alias.
    #[serde(default, alias = "optLevel", skip_serializing_if = "Option::is_none")]
    pub opt_level: Option<VyperOptimizationLevel>,
    /// Path to the Vyper compiler executable.
    ///
    /// When unset, Forge uses `vyper` from `PATH` if it is installed. Vyper is only needed when
    /// the project has Vyper sources.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    /// Whether to enable Vyper's experimental code generation pipeline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub experimental_codegen: Option<bool>,
    /// Whether to enable Vyper's experimental Venom pipeline.
    ///
    /// Passed to the compiler as the `venomExperimental` standard JSON setting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub venom_experimental: Option<bool>,
    /// Whether to compile in debug mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debug: Option<bool>,
    /// Whether to re-enable the Vyper `decimal` type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enable_decimals: Option<bool>,
    /// Fine-grained settings for the Venom optimizer, passed to the compiler as `venom`.
    ///
    /// Supported keys (snake_case or the compiler's camelCase): `disable_inlining`,
    /// `disable_cse`, `disable_sccp`, `disable_load_elimination`,
    /// `disable_dead_store_elimination`, `disable_algebraic_optimization`,
    /// `disable_branch_optimization`, `disable_assert_elimination`, `disable_mem2var`,
    /// `disable_simplify_cfg`, `disable_remove_unused_variables` (booleans) and `inline_threshold`
    /// (integer).
    ///
    /// ```toml
    /// [profile.default.vyper.venom]
    /// disable_inlining = true
    /// inline_threshold = 15
    /// ```
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub venom: Option<VyperVenomSettings>,
}
