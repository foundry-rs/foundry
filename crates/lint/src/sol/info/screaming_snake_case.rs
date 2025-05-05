use solar_ast::VariableDefinition;

use super::ScreamingSnakeCase;
use crate::{
    declare_forge_lint,
    linter::{EarlyLintPass, LintContext},
    sol::{Severity, SolLint},
};

declare_forge_lint!(
    SCREAMING_SNAKE_CASE,
    Severity::Info,
    "screaming-snake-case",
    "constants and immutables should use SCREAMING_SNAKE_CASE",
    "https://docs.soliditylang.org/en/latest/style-guide.html#contract-and-library-names"
);

impl<'ast> EarlyLintPass<'ast> for ScreamingSnakeCase {
    fn check_variable_definition(
        &mut self,
        ctx: &LintContext<'_>,
        var: &'ast VariableDefinition<'ast>,
    ) {
        if let Some(mutability) = var.mutability {
            if mutability.is_constant() || mutability.is_immutable() {
                if let Some(name) = var.name {
                    let name = name.as_str();
                    if !is_screaming_snake_case(name) && name.len() > 1 {
                        ctx.emit(&SCREAMING_SNAKE_CASE, var.span);
                    }
                }
            }
        }
    }
}

/// Check if a string is SCREAMING_SNAKE_CASE. Numbers don't need to be preceeded by an underscore.
pub fn is_screaming_snake_case(s: &str) -> bool {
    if s.len() <= 1 {
        return true;
    }

    // Remove leading/trailing underscores like `heck` does
    s.trim_matches('_') == format!("{}", heck::AsShoutySnakeCase(s)).as_str()
}

#[cfg(test)]
mod test {
    use std::path::Path;

    use super::*;
    use crate::{linter::Lint, sol::SolidityLinter};

    #[test]
    fn test_screaming_snake_case() -> eyre::Result<()> {
        let linter = SolidityLinter::new()
            .with_lints(Some(vec![SCREAMING_SNAKE_CASE]))
            .with_buffer_emitter(true);

        let emitted =
            linter.lint_file(Path::new("testdata/ScreamingSnakeCase.sol")).unwrap().to_string();
        let warnings = emitted.matches(&format!("warning[{}]", SCREAMING_SNAKE_CASE.id())).count();
        let notes = emitted.matches(&format!("note[{}]", SCREAMING_SNAKE_CASE.id())).count();

        assert_eq!(warnings, 0, "Expected 0 warnings");
        assert_eq!(notes, 8, "Expected 8 notes");

        Ok(())
    }
}
