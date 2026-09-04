use super::ScreamingSnakeCase;
use crate::{
    linter::{EarlyLintPass, LintContext},
    sol::{
        Severity, SolLint,
        naming::{check_screaming_snake_case, emit_rename},
    },
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
            let lint = match mutability {
                VarMut::Constant => &SCREAMING_SNAKE_CASE_CONSTANT,
                VarMut::Immutable => &SCREAMING_SNAKE_CASE_IMMUTABLE,
            };
            emit_rename(ctx, lint, name.span, expected);
        }
    }
}
