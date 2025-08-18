use super::ScreamingSnakeCase;
use crate::{
    linter::{EarlyLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar_ast::{VarMut, VariableDefinition};

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
        ctx: &LintContext<'_>,
        var: &'ast VariableDefinition<'ast>,
    ) {
        if let (Some(name), Some(mutability)) = (var.name, var.mutability) {
            let name_str = name.as_str();
            if name_str.len() < 2 || is_screaming_snake_case(name_str) {
                return;
            }

            match mutability {
                VarMut::Constant => ctx.emit(&SCREAMING_SNAKE_CASE_CONSTANT, name.span),
                VarMut::Immutable => ctx.emit(&SCREAMING_SNAKE_CASE_IMMUTABLE, name.span),
            }
        }
    }
}

/// Check if a string is SCREAMING_SNAKE_CASE. Numbers don't need to be preceded by an underscore.
pub fn is_screaming_snake_case(s: &str) -> bool {
    if s.len() <= 1 {
        return true;
    }

    // Remove leading/trailing underscores like `heck` does
    s.trim_matches('_') == format!("{}", heck::AsShoutySnakeCase(s)).as_str()
}
