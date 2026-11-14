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
    FunctionBody,
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
    /// Mutation operators to enable even when they are excluded.
    ///
    /// An operator listed here is enabled even if `exclude_operators` also lists it. No operator
    /// is excluded by default, so this only matters together with `exclude_operators`.
    ///
    /// Valid operators:
    ///
    /// - `assembly`: replaces opcodes in inline assembly with related opcodes.
    /// - `assignment`: replaces assigned values, such as flipping a boolean literal or replacing a
    ///   number with zero or its negation.
    /// - `binary-op`: replaces binary operators with other operators.
    /// - `delete-expression`: removes `delete` expressions.
    /// - `elim-delegate`: replaces `delegatecall` with `call`.
    /// - `function-body`: replaces a function body with an empty body or a constant return value.
    /// - `require`: replaces `require` and `assert` conditions with `true`, `false` or their
    ///   negation.
    /// - `unary-op`: replaces or removes unary operators.
    pub include_operators: Vec<MutatorType>,
    /// Mutation operators to disable.
    ///
    /// The enabled set is every operator, minus those listed here, plus those listed in
    /// `include_operators`. Changing either list invalidates cached mutation results. Valid
    /// operators are listed under `include_operators`.
    pub exclude_operators: Vec<MutatorType>,
    /// Wall-clock timeout for each mutant's compile-and-test run, in seconds.
    ///
    /// Mutants that exceed it are recorded as timed out, like `invariant.timeout` for invariant
    /// campaigns. Enforcement is best-effort: background work for a timed-out mutant may continue
    /// briefly until its compile or test loop reaches a checkpoint, but its worker slot is freed
    /// at once so other mutants can proceed, and the cleanup backlog is bounded by the number of
    /// mutation workers. When unset, mutants run without a timeout. The `--mutation-timeout` flag
    /// overrides it.
    pub timeout: Option<u32>,
    /// Optimizer runs to compile with during mutation testing, in place of `optimizer_runs`.
    ///
    /// This lets mutation campaigns use a faster compiler profile without changing the project's
    /// normal build settings. If the project uses the default optimizer settings (optimizer off,
    /// 200 runs), a value above zero also enables the optimizer for mutation runs. The
    /// `--mutation-optimizer-runs` flag overrides it.
    pub optimizer_runs: Option<u32>,
    /// Whether to compile with `via_ir` during mutation testing, in place of the project's value.
    ///
    /// This lets mutation campaigns disable the IR pipeline without changing the project's normal
    /// build settings. When unset, the project's `via_ir` is used. The `--mutation-via-ir` flag
    /// overrides it.
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
