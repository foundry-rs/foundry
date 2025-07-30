use super::{MixedCaseFunction, MixedCaseVariable};
use crate::{
    linter::{EarlyLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar_ast::{ItemFunction, VariableDefinition};

declare_forge_lint!(
    MIXED_CASE_FUNCTION,
    Severity::Info,
    "mixed-case-function",
    "function names should use mixedCase"
);

impl<'ast> EarlyLintPass<'ast> for MixedCaseFunction {
    fn check_item_function(&mut self, ctx: &LintContext<'_>, func: &'ast ItemFunction<'ast>) {
        if let Some(name) = func.header.name
            && !is_mixed_case(name.as_str(), true)
        {
            ctx.emit(&MIXED_CASE_FUNCTION, name.span);
        }
    }
}

declare_forge_lint!(
    MIXED_CASE_VARIABLE,
    Severity::Info,
    "mixed-case-variable",
    "mutable variables should use mixedCase"
);

impl<'ast> EarlyLintPass<'ast> for MixedCaseVariable {
    fn check_variable_definition(
        &mut self,
        ctx: &LintContext<'_>,
        var: &'ast VariableDefinition<'ast>,
    ) {
        if var.mutability.is_none()
            && let Some(name) = var.name
            && !is_mixed_case(name.as_str(), false)
        {
            ctx.emit(&MIXED_CASE_VARIABLE, name.span);
        }
    }
}

/// Check if a string is mixedCase
///
/// To avoid false positives like `fn increment()` or `uint256 counter`,
/// lowercase strings are treated as mixedCase.
pub fn is_mixed_case(s: &str, is_fn: bool) -> bool {
    if s.len() <= 1 {
        return true;
    }

    // Remove leading/trailing underscores like `heck` does
    if s.trim_matches('_') == format!("{}", heck::AsLowerCamelCase(s)).as_str() {
        return true;
    }

    // Ignore `fn test*`, `fn invariant_*`, and `fn statefulFuzz*` patterns, as they usually contain
    // (allowed) underscores.
    is_fn && (s.starts_with("test") || s.starts_with("invariant_") || s.starts_with("statefulFuzz"))
}
