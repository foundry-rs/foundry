use super::ScreamingSnakeCase;
use crate::{
    linter::{EarlyLintPass, LintContext, Suggestion},
    sol::{Severity, SolLint},
};
use solar::ast::{VarMut, VariableDefinition};

declare_forge_lint!(
    SCREAMING_SNAKE_CASE_CONSTANT,
    Severity::Info,
    "screaming-snake-case-const",
    "constants should use SCREAMING_SNAKE_CASE"
);

declare_forge_lint!(
    SCREAMING_SNAKE_CASE_IMMUTABLE,
    Severity::Info,
    "screaming-snake-case-immutable",
    "immutables should use SCREAMING_SNAKE_CASE"
);

impl<'ast> EarlyLintPass<'ast> for ScreamingSnakeCase {
    fn check_variable_definition(
        &mut self,
        ctx: &LintContext,
        var: &'ast VariableDefinition<'ast>,
    ) {
        if let (Some(name), Some(mutability)) = (var.name, var.mutability)
            && let Some(expected) = check_screaming_snake_case(name.as_str())
        {
            let suggestion = Suggestion::fix(
                expected,
                solar::interface::diagnostics::Applicability::MachineApplicable,
            )
            .with_desc("consider using");

            match mutability {
                VarMut::Constant => {
                    ctx.emit_with_suggestion(&SCREAMING_SNAKE_CASE_CONSTANT, name.span, suggestion)
                }
                VarMut::Immutable => {
                    ctx.emit_with_suggestion(&SCREAMING_SNAKE_CASE_IMMUTABLE, name.span, suggestion)
                }
            }
        }
    }
}

/// If the string `s` is not SCREAMING_SNAKE_CASE, returns a `Some(String)` with the suggested
/// conversion. Otherwise, returns `None`.
pub fn check_screaming_snake_case(s: &str) -> Option<String> {
    if s.len() <= 1 {
        return None;
    }

    // Handle leading/trailing underscores like `heck` does
    let expected = format!(
        "{prefix}{name}{suffix}",
        prefix = if s.starts_with("_") { "_" } else { "" },
        name = heck::AsShoutySnakeCase(s),
        suffix = if s.ends_with("_") { "_" } else { "" }
    );
    if s == expected { None } else { Some(expected) }
}
