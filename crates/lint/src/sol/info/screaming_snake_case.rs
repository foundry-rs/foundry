use super::ScreamingSnakeCase;
use crate::{
    linter::{EarlyLintPass, LintContext},
    sol::{
        Severity, SolLint,
        naming::{check_screaming_snake_case, suggest_rename},
    },
};
use solar::ast::{VarMut, VariableDefinition};

declare_forge_lint!(SCREAMING_SNAKE_CASE_CONSTANT, Severity::Info, "screaming-snake-case-const");

declare_forge_lint!(
    SCREAMING_SNAKE_CASE_IMMUTABLE,
    Severity::Info,
    "screaming-snake-case-immutable"
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
            let (lint, message) = match mutability {
                VarMut::Constant => {
                    (&SCREAMING_SNAKE_CASE_CONSTANT, "constant name is not `SCREAMING_SNAKE_CASE`")
                }
                VarMut::Immutable => (
                    &SCREAMING_SNAKE_CASE_IMMUTABLE,
                    "immutable name is not `SCREAMING_SNAKE_CASE`",
                ),
            };
            ctx.span_lint(lint, name.span, |diag| {
                diag.primary_message(message);
                suggest_rename(diag, name.span, expected);
            });
        }
    }
}
