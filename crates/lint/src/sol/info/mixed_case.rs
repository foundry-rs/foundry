use super::{MixedCaseFunction, MixedCaseVariable};
use crate::{
    linter::{EarlyLintPass, LintContext, Suggestion},
    sol::{
        Severity, SolLint,
        naming::{check_mixed_case as check_mixed_case_pure, check_screaming_snake_case},
    },
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
            && let Some(expected) = check_mixed_case(
                name.as_str(),
                true,
                &ctx.config.lint_specific.mixed_case_exceptions,
            )
            && !is_constant_getter(&func.header)
        {
            ctx.emit_with_suggestion(
                &MIXED_CASE_FUNCTION,
                name.span,
                Suggestion::fix(
                    expected,
                    solar::interface::diagnostics::Applicability::MachineApplicable,
                )
                .with_desc("consider using"),
            );
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
            && let Some(expected) = check_mixed_case(
                name.as_str(),
                false,
                &ctx.config.lint_specific.mixed_case_exceptions,
            )
        {
            ctx.emit_with_suggestion(
                &MIXED_CASE_VARIABLE,
                name.span,
                Suggestion::fix(
                    expected,
                    solar::interface::diagnostics::Applicability::MachineApplicable,
                )
                .with_desc("consider using"),
            );
        }
    }
}

/// Wraps [`check_mixed_case_pure`] with two domain exceptions:
/// foundry test-function prefixes and user-defined infix patterns.
fn check_mixed_case(s: &str, is_fn: bool, allowed_patterns: &[String]) -> Option<String> {
    if s.len() <= 1 {
        return None;
    }

    // Exception for test, invariant, and stateful fuzzing functions.
    if is_fn
        && (s.starts_with("test") || s.starts_with("invariant_") || s.starts_with("statefulFuzz"))
    {
        return None;
    }

    // Exception for user-defined infix patterns.
    for pattern in allowed_patterns {
        if let Some(pos) = s.find(pattern.as_str()) {
            let (pre, post) = s.split_at(pos);
            let post = &post[pattern.len()..];

            // Pre-pattern must be valid lowerCamelCase.
            let is_pre_valid = pre == heck::AsLowerCamelCase(pre).to_string();

            // Post-pattern must be valid UpperCamelCase (allowing leading numbers).
            let post_trimmed = post.trim_start_matches(|c: char| c.is_numeric());
            let is_post_valid = post_trimmed == heck::AsUpperCamelCase(post_trimmed).to_string();

            if is_pre_valid && is_post_valid {
                return None;
            }
        }
    }

    check_mixed_case_pure(s)
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
        && check_screaming_snake_case(header.name.unwrap().as_str()).is_none()
}
