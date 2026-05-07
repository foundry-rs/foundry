//! Commonly used traits.

use alloy_json_abi::Function;
use alloy_primitives::Bytes;
use alloy_sol_types::SolError;
use std::{
    fmt::{self, Display, Formatter},
    path::Path,
};

/// Test filter.
pub trait TestFilter: Send + Sync {
    /// Returns whether the test should be included.
    fn matches_test(&self, test_signature: &str) -> bool;

    /// Returns whether the contract should be included.
    fn matches_contract(&self, contract_name: &str) -> bool;

    /// Returns a contract with the given path should be included.
    fn matches_path(&self, path: &Path) -> bool;
}

impl<'a> dyn TestFilter + 'a {
    /// Returns `true` if the function is a test function that matches the given filter.
    pub fn matches_test_function(&self, func: &Function) -> bool {
        func.is_any_test() && self.matches_test(&func.signature())
    }
}

/// A test filter that filters out nothing.
#[derive(Clone, Debug, Default)]
pub struct EmptyTestFilter(());
impl TestFilter for EmptyTestFilter {
    fn matches_test(&self, _test_signature: &str) -> bool {
        true
    }

    fn matches_contract(&self, _contract_name: &str) -> bool {
        true
    }

    fn matches_path(&self, _path: &Path) -> bool {
        true
    }
}

/// Extension trait for `Function`.
pub trait TestFunctionExt {
    /// Returns the kind of test function.
    fn test_function_kind(&self) -> TestFunctionKind {
        TestFunctionKind::classify(self.tfe_as_str(), self.tfe_has_inputs())
    }

    /// Returns `true` if this function is a `setUp` function.
    fn is_setup(&self) -> bool {
        self.test_function_kind().is_setup()
    }

    /// Returns `true` if this function is a unit, fuzz, or invariant test.
    fn is_any_test(&self) -> bool {
        self.test_function_kind().is_any_test()
    }

    /// Returns `true` if this function is a test that should fail.
    fn is_any_test_fail(&self) -> bool {
        self.test_function_kind().is_any_test_fail()
    }

    /// Returns `true` if this function is a unit test.
    fn is_unit_test(&self) -> bool {
        matches!(self.test_function_kind(), TestFunctionKind::UnitTest { .. })
    }

    /// Returns `true` if this function is a `beforeTestSetup` function.
    fn is_before_test_setup(&self) -> bool {
        self.tfe_as_str().eq_ignore_ascii_case("beforetestsetup")
    }

    /// Returns `true` if this function is a fuzz test.
    fn is_fuzz_test(&self) -> bool {
        self.test_function_kind().is_fuzz_test()
    }

    /// Returns `true` if this function is an invariant test.
    fn is_invariant_test(&self) -> bool {
        self.test_function_kind().is_invariant_test()
    }

    /// Returns `true` if this function is a symbolic test (`check_*` or `prove_*`).
    fn is_symbolic_test(&self) -> bool {
        self.test_function_kind().is_symbolic_test()
    }

    /// Returns `true` if this function is an `afterInvariant` function.
    fn is_after_invariant(&self) -> bool {
        self.test_function_kind().is_after_invariant()
    }

    /// Returns `true` if this function is a `fixture` function.
    fn is_fixture(&self) -> bool {
        self.test_function_kind().is_fixture()
    }

    /// Returns `true` if this function is test reserved function.
    fn is_reserved(&self) -> bool {
        self.is_any_test()
            || self.is_setup()
            || self.is_before_test_setup()
            || self.is_after_invariant()
            || self.is_fixture()
    }

    #[doc(hidden)]
    fn tfe_as_str(&self) -> &str;
    #[doc(hidden)]
    fn tfe_has_inputs(&self) -> bool;
}

impl TestFunctionExt for Function {
    fn tfe_as_str(&self) -> &str {
        self.name.as_str()
    }

    fn tfe_has_inputs(&self) -> bool {
        !self.inputs.is_empty()
    }
}

impl TestFunctionExt for String {
    fn tfe_as_str(&self) -> &str {
        self
    }

    fn tfe_has_inputs(&self) -> bool {
        false
    }
}

impl TestFunctionExt for str {
    fn tfe_as_str(&self) -> &str {
        self
    }

    fn tfe_has_inputs(&self) -> bool {
        false
    }
}

/// Test function kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TestFunctionKind {
    /// `setUp`.
    Setup,
    /// `test*`. `should_fail` is `true` for `testFail*`.
    UnitTest { should_fail: bool },
    /// `test*`, with arguments. `should_fail` is `true` for `testFail*`.
    FuzzTest { should_fail: bool },
    /// `invariant*` or `statefulFuzz*`.
    InvariantTest,
    /// `table*`, with arguments.
    TableTest,
    /// `check_*` / `prove_*` (symbolic test).
    ///
    /// `should_fail` is `true` for `checkFail_*` / `proveFail_*`.
    /// `mode` distinguishes the soundness contract (see [`SymbolicMode`]).
    SymbolicTest { mode: SymbolicMode, should_fail: bool },
    /// `afterInvariant`.
    AfterInvariant,
    /// `fixture*`.
    Fixture,
    /// Unknown kind.
    Unknown,
}

/// The soundness mode of a [`TestFunctionKind::SymbolicTest`].
///
/// `check_*` is permissive: a partial proof (loop bound hit, depth bound, etc.) is reported as
/// success with a `[BOUNDED]` annotation. `prove_*` is strict: any partial result is a failure.
///
/// Verdict-acceptance is defined in `docs/symbolic`; this enum only carries the
/// classification.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SymbolicMode {
    /// `check_*` — permissive: bounded proofs count as success.
    Check,
    /// `prove_*` — strict: bounded/timeout/unsupported are failures.
    Prove,
}

impl SymbolicMode {
    /// Returns the canonical prefix for this mode (without the trailing `_`).
    pub const fn prefix(self) -> &'static str {
        match self {
            Self::Check => "check",
            Self::Prove => "prove",
        }
    }

    /// Returns the canonical "should-fail" prefix for this mode (without the trailing `_`).
    pub const fn fail_prefix(self) -> &'static str {
        match self {
            Self::Check => "checkFail",
            Self::Prove => "proveFail",
        }
    }
}

impl Display for SymbolicMode {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(self.prefix())
    }
}

impl TestFunctionKind {
    /// Classify a function.
    pub fn classify(name: &str, has_inputs: bool) -> Self {
        match () {
            _ if name.starts_with("test") => {
                let should_fail = name.starts_with("testFail");
                if has_inputs {
                    Self::FuzzTest { should_fail }
                } else {
                    Self::UnitTest { should_fail }
                }
            }
            _ if name.starts_with("invariant") || name.starts_with("statefulFuzz") => {
                Self::InvariantTest
            }
            _ if name.starts_with("table") => Self::TableTest,
            // Symbolic tests. The trailing `_` is required to avoid collisions with regular
            // identifiers that happen to start with "check"/"prove" (e.g. `checkout`).
            _ if name.starts_with("checkFail_") => {
                Self::SymbolicTest { mode: SymbolicMode::Check, should_fail: true }
            }
            _ if name.starts_with("check_") => {
                Self::SymbolicTest { mode: SymbolicMode::Check, should_fail: false }
            }
            _ if name.starts_with("proveFail_") => {
                Self::SymbolicTest { mode: SymbolicMode::Prove, should_fail: true }
            }
            _ if name.starts_with("prove_") => {
                Self::SymbolicTest { mode: SymbolicMode::Prove, should_fail: false }
            }
            _ if name.eq_ignore_ascii_case("setup") && !has_inputs => Self::Setup,
            _ if name.eq_ignore_ascii_case("afterinvariant") => Self::AfterInvariant,
            _ if name.starts_with("fixture") => Self::Fixture,
            _ => Self::Unknown,
        }
    }

    /// Returns the name of the function kind.
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Setup => "setUp",
            Self::UnitTest { should_fail: false } => "test",
            Self::UnitTest { should_fail: true } => "testFail",
            Self::FuzzTest { should_fail: false } => "fuzz",
            Self::FuzzTest { should_fail: true } => "fuzz fail",
            Self::InvariantTest => "invariant",
            Self::TableTest => "table",
            Self::SymbolicTest { mode: SymbolicMode::Check, should_fail: false } => "check",
            Self::SymbolicTest { mode: SymbolicMode::Check, should_fail: true } => "check fail",
            Self::SymbolicTest { mode: SymbolicMode::Prove, should_fail: false } => "prove",
            Self::SymbolicTest { mode: SymbolicMode::Prove, should_fail: true } => "prove fail",
            Self::AfterInvariant => "afterInvariant",
            Self::Fixture => "fixture",
            Self::Unknown => "unknown",
        }
    }

    /// Returns `true` if this function is a `setUp` function.
    #[inline]
    pub const fn is_setup(&self) -> bool {
        matches!(self, Self::Setup)
    }

    /// Returns `true` if this function is a unit, fuzz, invariant, table, or symbolic test.
    #[inline]
    pub const fn is_any_test(&self) -> bool {
        matches!(
            self,
            Self::UnitTest { .. }
                | Self::FuzzTest { .. }
                | Self::TableTest
                | Self::InvariantTest
                | Self::SymbolicTest { .. }
        )
    }

    /// Returns `true` if this function is a test that should fail.
    #[inline]
    pub const fn is_any_test_fail(&self) -> bool {
        matches!(
            self,
            Self::UnitTest { should_fail: true }
                | Self::FuzzTest { should_fail: true }
                | Self::SymbolicTest { should_fail: true, .. }
        )
    }

    /// Returns `true` if this function is a unit test.
    #[inline]
    pub const fn is_unit_test(&self) -> bool {
        matches!(self, Self::UnitTest { .. })
    }

    /// Returns `true` if this function is a fuzz test.
    #[inline]
    pub const fn is_fuzz_test(&self) -> bool {
        matches!(self, Self::FuzzTest { .. })
    }

    /// Returns `true` if this function is an invariant test.
    #[inline]
    pub const fn is_invariant_test(&self) -> bool {
        matches!(self, Self::InvariantTest)
    }

    /// Returns `true` if this function is a symbolic test (`check_*` or `prove_*`).
    #[inline]
    pub const fn is_symbolic_test(&self) -> bool {
        matches!(self, Self::SymbolicTest { .. })
    }

    /// Returns the [`SymbolicMode`] if this is a symbolic test, otherwise `None`.
    #[inline]
    pub const fn symbolic_mode(&self) -> Option<SymbolicMode> {
        match self {
            Self::SymbolicTest { mode, .. } => Some(*mode),
            _ => None,
        }
    }

    /// Returns `true` if this function is a table test.
    #[inline]
    pub const fn is_table_test(&self) -> bool {
        matches!(self, Self::TableTest)
    }

    /// Returns `true` if this function is an `afterInvariant` function.
    #[inline]
    pub const fn is_after_invariant(&self) -> bool {
        matches!(self, Self::AfterInvariant)
    }

    /// Returns `true` if this function is a `fixture` function.
    #[inline]
    pub const fn is_fixture(&self) -> bool {
        matches!(self, Self::Fixture)
    }

    /// Returns `true` if this function kind is known.
    #[inline]
    pub const fn is_known(&self) -> bool {
        !matches!(self, Self::Unknown)
    }

    /// Returns `true` if this function kind is unknown.
    #[inline]
    pub const fn is_unknown(&self) -> bool {
        matches!(self, Self::Unknown)
    }
}

impl Display for TestFunctionKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        self.name().fmt(f)
    }
}

/// An extension trait for `std::error::Error` for ABI encoding.
pub trait ErrorExt: std::error::Error {
    /// ABI-encodes the error using `Revert(string)`.
    fn abi_encode_revert(&self) -> Bytes;
}

impl<T: std::error::Error> ErrorExt for T {
    fn abi_encode_revert(&self) -> Bytes {
        alloy_sol_types::Revert::from(self.to_string()).abi_encode().into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_setup_classification() {
        // setUp() with no params should be classified as Setup
        assert_eq!(TestFunctionKind::classify("setUp", false), TestFunctionKind::Setup);

        // setUp(bytes memory) with params should NOT be classified as Setup
        // This is common in Gnosis Safe/Zodiac modules
        assert_eq!(TestFunctionKind::classify("setUp", true), TestFunctionKind::Unknown);
    }

    #[test]
    fn test_symbolic_classification() {
        use TestFunctionKind::SymbolicTest;
        // Plain prefixes.
        assert_eq!(
            TestFunctionKind::classify("check_invariant", false),
            SymbolicTest { mode: SymbolicMode::Check, should_fail: false }
        );
        assert_eq!(
            TestFunctionKind::classify("prove_overflow", true),
            SymbolicTest { mode: SymbolicMode::Prove, should_fail: false }
        );
        // Fail variants.
        assert_eq!(
            TestFunctionKind::classify("checkFail_revert", false),
            SymbolicTest { mode: SymbolicMode::Check, should_fail: true }
        );
        assert_eq!(
            TestFunctionKind::classify("proveFail_revert", true),
            SymbolicTest { mode: SymbolicMode::Prove, should_fail: true }
        );

        // Trailing `_` is required: bare `check`/`prove` and look-alikes don't classify as
        // symbolic tests, to avoid hijacking unrelated identifiers.
        assert_eq!(TestFunctionKind::classify("checkout", false), TestFunctionKind::Unknown);
        assert_eq!(TestFunctionKind::classify("proven", false), TestFunctionKind::Unknown);
        assert_eq!(TestFunctionKind::classify("check", false), TestFunctionKind::Unknown);

        // Symbolic tests are tests and are reserved.
        let kind = TestFunctionKind::classify("check_x", false);
        assert!(kind.is_any_test());
        assert!(kind.is_symbolic_test());
        assert_eq!(kind.symbolic_mode(), Some(SymbolicMode::Check));
        assert!(!kind.is_any_test_fail());

        // Fail-variant flips is_any_test_fail.
        let kind_fail = TestFunctionKind::classify("proveFail_x", false);
        assert!(kind_fail.is_any_test_fail());
        assert_eq!(kind_fail.symbolic_mode(), Some(SymbolicMode::Prove));

        // `invariant_*` stays an invariant test (not symbolic) — open question
        // resolved in favor of "no silent behavior changes".
        assert_eq!(
            TestFunctionKind::classify("invariant_total_supply", false),
            TestFunctionKind::InvariantTest
        );
        assert!(!TestFunctionKind::classify("invariant_total_supply", false).is_symbolic_test());
    }
}
