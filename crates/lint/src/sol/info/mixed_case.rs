use super::{MixedCaseFunction, MixedCaseVariable};
use crate::{
    linter::{EarlyLintPass, LintContext},
    sol::{Severity, SolLint, info::screaming_snake_case::is_screaming_snake_case},
};
use solar::ast::{FunctionHeader, ItemFunction, VariableDefinition, Visibility};

declare_forge_lint!(
    MIXED_CASE_FUNCTION,
    Severity::Info,
    "mixed-case-function",
    "function names should use mixedCase"
);

impl<'ast> EarlyLintPass<'ast> for MixedCaseFunction {
    fn check_item_function(&mut self, ctx: &LintContext, func: &'ast ItemFunction<'ast>) {
        if let Some(name) = func.header.name
            && !is_mixed_case(name.as_str(), true, ctx.config.mixed_case_exceptions)
            && !is_constant_getter(&func.header)
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
        ctx: &LintContext,
        var: &'ast VariableDefinition<'ast>,
    ) {
        if var.mutability.is_none()
            && let Some(name) = var.name
            && !is_mixed_case(name.as_str(), false, ctx.config.mixed_case_exceptions)
        {
            ctx.emit(&MIXED_CASE_VARIABLE, name.span);
        }
    }
}

/// Checks if a string is mixedCase.
///
/// To avoid false positives like `fn increment()` or `uint256 counter`,
/// lowercase strings are treated as mixedCase.
pub fn is_mixed_case(s: &str, is_fn: bool, allowed_patterns: &[String]) -> bool {
    if s.len() <= 1 {
        return true;
    }

    // Remove leading/trailing underscores like `heck` does.
    if check_lower_mixed_case(s.trim_matches('_')) {
        return true;
    }

    // Ignore user-defined infixes.
    for pattern in allowed_patterns {
        if let Some(pos) = s.find(pattern.as_str())
            && check_lower_mixed_case(&s[..pos])
            && check_upper_mixed_case_post_pattern(&s[pos + pattern.len()..])
        {
            return true;
        }
    }

    // Ignore `fn test*`, `fn invariant_*`, and `fn statefulFuzz*` patterns, as they usually contain
    // (allowed) underscores.
    is_fn && (s.starts_with("test") || s.starts_with("invariant_") || s.starts_with("statefulFuzz"))
}

fn check_lower_mixed_case(s: &str) -> bool {
    s == heck::AsLowerCamelCase(s).to_string().as_str()
}

fn check_upper_mixed_case_post_pattern(s: &str) -> bool {
    // Find the index of the first character that is not a numeric digit.
    let Some(split_idx) = s.find(|c: char| !c.is_numeric()) else {
        return true;
    };

    // Validate the characters preceding the initial numbers have the correct format.
    let trimmed = &s[split_idx..];
    if let Some(c) = trimmed.chars().next()
        && !c.is_alphabetic()
    {
        return false;
    }
    trimmed == heck::AsUpperCamelCase(trimmed).to_string().as_str()
}

/// Checks if a function getter is a valid constant getter with a heuristic:
///  * name is `SCREAMING_SNAKE_CASE`
///  * external view visibility and mutability.
///  * zero parameters.
///  * exactly one return value.
///  * return value is an elementary or a custom type
fn is_constant_getter(header: &FunctionHeader<'_>) -> bool {
    header.visibility().is_some_and(|v| matches!(v, Visibility::External))
        && header.state_mutability().is_view()
        && header.parameters.is_empty()
        && header.returns().len() == 1
        && header
            .returns()
            .first()
            .is_some_and(|ret| ret.ty.kind.is_elementary() || ret.ty.kind.is_custom())
        && is_screaming_snake_case(header.name.unwrap().as_str())
}
