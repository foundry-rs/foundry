use solar_ast::{ItemFunction, VariableDefinition};

use super::{MixedCaseFunction, MixedCaseVariable};
use crate::{
    declare_forge_lint,
    linter::{EarlyLintPass, LintContext},
    sol::{Severity, SolLint},
};

declare_forge_lint!(
    MIXED_CASE_FUNCTION,
    Severity::Info,
    "mixed-case-function",
    "function names should use mixedCase.",
    "https://docs.soliditylang.org/en/latest/style-guide.html#function-names"
);

impl<'ast> EarlyLintPass<'ast> for MixedCaseFunction {
    fn check_item_function(&mut self, ctx: &LintContext<'_>, func: &'ast ItemFunction<'ast>) {
        if let Some(name) = func.header.name {
            let name = name.as_str();
            if !is_mixed_case(name) && name.len() > 1 {
                ctx.emit(&MIXED_CASE_FUNCTION, func.body_span);
            }
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
        if var.mutability.is_none() {
            if let Some(name) = var.name {
                let name = name.as_str();
                if !is_mixed_case(name) {
                    ctx.emit(&MIXED_CASE_VARIABLE, var.span);
                }
            }
        }
    }
}

/// Check if a string is mixedCase
///
/// To avoid false positives like `fn increment()` or `uin256 counter`,
/// lowercase strings are treated as mixedCase.
pub fn is_mixed_case(s: &str) -> bool {
    if s.len() <= 1 {
        return true;
    }

    // Remove leading/trailing underscores like `heck` does
    s.trim_matches('_') == format!("{}", heck::AsLowerCamelCase(s)).as_str()
}

#[cfg(test)]
mod test {
    use std::path::Path;

    use super::*;
    use crate::{linter::Lint, sol::SolidityLinter};

    #[test]
    fn test_variable_mixed_case() -> eyre::Result<()> {
        let linter = SolidityLinter::new().with_lints(Some(vec![MIXED_CASE_VARIABLE]));

        let emitted = linter.lint_test(Path::new("testdata/MixedCase.sol")).unwrap().to_string();
        let warnings = emitted.matches(&format!("warning[{}]", MIXED_CASE_VARIABLE.id())).count();
        let notes = emitted.matches(&format!("note[{}]", MIXED_CASE_VARIABLE.id())).count();

        assert_eq!(warnings, 0, "Expected 0 warnings");
        assert_eq!(notes, 6, "Expected 6 notes");

        Ok(())
    }

    #[test]
    fn test_function_mixed_case() -> eyre::Result<()> {
        let linter = SolidityLinter::new().with_lints(Some(vec![MIXED_CASE_FUNCTION]));

        let emitted = linter.lint_test(Path::new("testdata/MixedCase.sol")).unwrap().to_string();
        let warnings = emitted.matches(&format!("warning[{}]", MIXED_CASE_FUNCTION.id())).count();
        let notes = emitted.matches(&format!("note[{}]", MIXED_CASE_FUNCTION.id())).count();

        assert_eq!(warnings, 0, "Expected 0 warnings");
        assert_eq!(notes, 3, "Expected 3 notes");

        Ok(())
    }
}
