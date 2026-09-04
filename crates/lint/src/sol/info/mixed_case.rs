use crate::{
    linter::{EarlyLintPass, LintContext},
    sol::{
        Severity, SolLint,
        naming::{
            check_mixed_case as check_mixed_case_pure, check_screaming_snake_case, emit_rename,
            has_acronym_exception,
        },
    },
};
use foundry_config::lint::LintSpecificConfig;
use solar::ast::{FunctionHeader, ItemFunction, VariableDefinition, Visibility};
use std::sync::Arc;

declare_forge_lint!(
    MIXED_CASE_FUNCTION,
    Severity::Info,
    "mixed-case-function",
    "function names should use mixedCase"
);

declare_forge_lint!(
    MIXED_CASE_VARIABLE,
    Severity::Info,
    "mixed-case-variable",
    "mutable variables should use mixedCase"
);

/// Checks function names when `FUNCTIONS` is set, mutable variable names otherwise.
#[derive(Debug)]
pub(super) struct MixedCasePass<const FUNCTIONS: bool> {
    config: Arc<LintSpecificConfig>,
}

pub(super) type MixedCaseFunctionPass = MixedCasePass<true>;
pub(super) type MixedCaseVariablePass = MixedCasePass<false>;

impl<const FUNCTIONS: bool> MixedCasePass<FUNCTIONS> {
    pub(super) const fn new(config: Arc<LintSpecificConfig>) -> Self {
        Self { config }
    }
}

impl<'ast, const FUNCTIONS: bool> EarlyLintPass<'ast> for MixedCasePass<FUNCTIONS> {
    fn check_item_function(&mut self, ctx: &LintContext, func: &'ast ItemFunction<'ast>) {
        if FUNCTIONS
            && let Some(name) = func.header.name
            && let Some(expected) =
                check_mixed_case(name.as_str(), true, &self.config.mixed_case_exceptions)
            && !is_constant_getter(&func.header)
        {
            emit_rename(ctx, &MIXED_CASE_FUNCTION, name.span, expected);
        }
    }

    fn check_variable_definition(
        &mut self,
        ctx: &LintContext,
        var: &'ast VariableDefinition<'ast>,
    ) {
        if !FUNCTIONS
            && var.mutability.is_none()
            && let Some(name) = var.name
            && let Some(expected) =
                check_mixed_case(name.as_str(), false, &self.config.mixed_case_exceptions)
        {
            emit_rename(ctx, &MIXED_CASE_VARIABLE, name.span, expected);
        }
    }
}

/// Wraps [`check_mixed_case_pure`] with two domain exceptions: foundry test-function prefixes
/// and user-defined infix patterns, which split the name into a lowerCamelCase prefix and an
/// UpperCamelCase suffix (allowing leading digits).
fn check_mixed_case(s: &str, is_fn: bool, allowed_patterns: &[String]) -> Option<String> {
    if is_fn && ["test", "invariant_", "statefulFuzz"].iter().any(|prefix| s.starts_with(prefix)) {
        return None;
    }
    if has_acronym_exception(s, allowed_patterns, |pre| {
        pre == heck::AsLowerCamelCase(pre).to_string()
    }) {
        return None;
    }
    check_mixed_case_pure(s)
}

/// Heuristic for a getter of a constant: `SCREAMING_SNAKE_CASE` name, `external view`, no
/// parameters and exactly one elementary or custom-typed return value.
fn is_constant_getter(header: &FunctionHeader<'_>) -> bool {
    matches!(header.visibility(), Some(Visibility::External))
        && header.state_mutability().is_view()
        && header.parameters.is_empty()
        && matches!(header.returns(), [ret] if ret.ty.kind.is_elementary() || ret.ty.kind.is_custom())
        && header.name.is_some_and(|name| check_screaming_snake_case(name.as_str()).is_none())
}
