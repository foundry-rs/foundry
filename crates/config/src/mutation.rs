//! Configuration for mutation testing.

use serde::{Deserialize, Serialize};
use strum::IntoEnumIterator;

/// Represents each available mutation operator.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    strum::Display,
    strum::EnumString,
    strum::EnumIter,
)]
#[serde(rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case")]
pub enum MutatorType {
    Assembly,
    Assignment,
    BinaryOp,
    DeleteExpression,
    ElimDelegate,
    Require,
    UnaryOp,
}

impl MutatorType {
    /// Returns a list of all available mutator types.
    pub fn all() -> Vec<Self> {
        Self::iter().collect()
    }

    /// Returns the operators that are excluded by default.
    pub const fn default_excluded() -> Vec<Self> {
        Vec::new()
    }
}

/// Configuration for mutation testing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MutationConfig {
    /// Re-enable operators that are excluded by default.
    pub include_operators: Vec<MutatorType>,
    /// Exclude additional operators beyond the defaults.
    pub exclude_operators: Vec<MutatorType>,
    /// Per-mutant wall-clock timeout, in seconds.
    ///
    /// When set, each mutant's compile-and-test work is bounded by this
    /// duration; mutants that exceed it are recorded as `TimedOut`. This is
    /// the analog of `invariant.timeout` for mutation campaigns.
    ///
    /// Note: enforcement is best-effort. Background work for a timed-out
    /// mutant may continue briefly until the underlying compile / test loop
    /// reaches a checkpoint, but the worker slot is freed immediately so
    /// other mutants can proceed. Cleanup backlog is bounded by the configured
    /// mutation worker count.
    pub timeout: Option<u32>,
    /// Override `optimizer_runs` for mutation testing compile-and-test runs.
    ///
    /// This lets mutation campaigns use a faster compiler profile without
    /// changing the project's normal build settings.
    pub optimizer_runs: Option<u32>,
    /// Override `via_ir` for mutation testing compile-and-test runs.
    ///
    /// This lets mutation campaigns disable the IR pipeline without changing
    /// the project's normal build settings.
    pub via_ir: Option<bool>,
}

impl MutationConfig {
    /// Returns the list of operators that are currently enabled.
    ///
    /// Effective set: `all() - default_excluded - exclude_operators + include_operators`
    pub fn enabled_operators(&self) -> Vec<MutatorType> {
        let default_excluded = MutatorType::default_excluded();
        MutatorType::all()
            .into_iter()
            .filter(|op| {
                let excluded = default_excluded.contains(op) || self.exclude_operators.contains(op);
                let included = self.include_operators.contains(op);
                !excluded || included
            })
            .collect()
    }
}
