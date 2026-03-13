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
    pub fn default_excluded() -> Vec<Self> {
        vec![]
    }
}

/// Configuration for mutation testing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MutationConfig {
    /// Re-enable operators that are excluded by default.
    pub include_operators: Vec<MutatorType>,
    /// Exclude additional operators beyond the defaults.
    pub exclude_operators: Vec<MutatorType>,
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
