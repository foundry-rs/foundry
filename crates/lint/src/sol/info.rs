use regex::Regex;
use solar_ast::{ItemFunction, ItemStruct, VariableDefinition};
use std::ops::ControlFlow;

use crate::{
    linter::{EarlyLintPass, LintContext},
    sol::{
        FunctionMixedCase, ScreamingSnakeCase, StructPascalCase, VariableMixedCase,
        FUNCTION_MIXED_CASE, SCREAMING_SNAKE_CASE, STRUCT_PASCAL_CASE, VARIABLE_MIXED_CASE,
    },
};

impl<'ast> EarlyLintPass<'ast> for VariableMixedCase {
    fn check_variable_definition(
        &mut self,
        ctx: &LintContext<'_>,
        var: &'ast VariableDefinition<'ast>,
    ) -> ControlFlow<()> {
        if var.mutability.is_none() {
            if let Some(name) = var.name {
                let name = name.as_str();
                if !is_mixed_case(name) {
                    ctx.emit(&VARIABLE_MIXED_CASE, var.span);
                }
            }
        }
        ControlFlow::Continue(())
    }
}

impl<'ast> EarlyLintPass<'ast> for FunctionMixedCase {
    fn check_item_function(
        &mut self,
        ctx: &LintContext<'_>,
        func: &'ast ItemFunction<'ast>,
    ) -> ControlFlow<()> {
        if let Some(name) = func.header.name {
            let name = name.as_str();
            if !is_mixed_case(name) && name.len() > 1 {
                ctx.emit(&FUNCTION_MIXED_CASE, func.body_span);
            }
        }
        ControlFlow::Continue(())
    }
}

impl<'ast> EarlyLintPass<'ast> for ScreamingSnakeCase {
    fn check_variable_definition(
        &mut self,
        ctx: &LintContext<'_>,
        var: &'ast VariableDefinition<'ast>,
    ) -> ControlFlow<()> {
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
        ControlFlow::Continue(())
    }
}

impl<'ast> EarlyLintPass<'ast> for StructPascalCase {
    fn check_item_struct(
        &mut self,
        ctx: &LintContext<'_>,
        strukt: &'ast ItemStruct<'ast>,
    ) -> ControlFlow<()> {
        let name = strukt.name.as_str();
        if !is_pascal_case(name) && name.len() > 1 {
            ctx.emit(&STRUCT_PASCAL_CASE, strukt.name.span);
        }
        ControlFlow::Continue(())
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

    let re = Regex::new(r"^[a-z_][a-zA-Z0-9]*$").unwrap();
    re.is_match(s)
}

/// Check if a string is PascalCase
pub fn is_pascal_case(s: &str) -> bool {
    if s.len() <= 1 {
        return true;
    }

    let re = Regex::new(r"^[A-Z][a-z]+(?:[A-Z][a-z]+)*$").unwrap();
    re.is_match(s)
}

/// Check if a string is SCREAMING_SNAKE_CASE, where
/// numbers must always be preceeded by an underscode.
pub fn is_screaming_snake_case(s: &str) -> bool {
    if s.len() <= 1 {
        return true;
    }

    let re = Regex::new(r"^[A-Z_][A-Z0-9_]*$").unwrap();
    let invalid_re = Regex::new(r"[A-Z][0-9]").unwrap();
    re.is_match(s) && !invalid_re.is_match(s) 
}

#[cfg(test)]
mod test {
    use std::path::Path;

    use super::*;
    use crate::{
        linter::Lint,
        sol::{SolidityLinter, FUNCTION_MIXED_CASE},
    };

    #[test]
    fn test_variable_mixed_case() -> eyre::Result<()> {
        let linter = SolidityLinter::new()
            .with_lints(Some(vec![VARIABLE_MIXED_CASE]))
            .with_buffer_emitter(true);

        let emitted =
            linter.lint_file(Path::new("testdata/MixedCase.sol")).unwrap().to_string();
        let warnings = emitted.matches(&format!("warning: {}", VARIABLE_MIXED_CASE.id())).count();
        let notes = emitted.matches(&format!("note: {}", VARIABLE_MIXED_CASE.id())).count();

        assert_eq!(warnings, 0, "Expected 0 warnings");
        assert_eq!(notes, 5, "Expected 5 notes");

        Ok(())
    }

    #[test]
    fn test_function_mixed_case() -> eyre::Result<()> {
        let linter = SolidityLinter::new()
            .with_lints(Some(vec![FUNCTION_MIXED_CASE]))
            .with_buffer_emitter(true);

        let emitted =
            linter.lint_file(Path::new("testdata/MixedCase.sol")).unwrap().to_string();
        let warnings = emitted.matches(&format!("warning: {}", FUNCTION_MIXED_CASE.id())).count();
        let notes = emitted.matches(&format!("note: {}", FUNCTION_MIXED_CASE.id())).count();

        assert_eq!(warnings, 0, "Expected 0 warnings");
        assert_eq!(notes, 3, "Expected 3 notes");

        Ok(())
    }

    #[test]
    fn test_screaming_snake_case() -> eyre::Result<()> {
        let linter = SolidityLinter::new()
            .with_lints(Some(vec![SCREAMING_SNAKE_CASE]))
            .with_buffer_emitter(true);

        let emitted =
            linter.lint_file(Path::new("testdata/ScreamingSnakeCase.sol")).unwrap().to_string();
        let warnings = emitted.matches(&format!("warning: {}", SCREAMING_SNAKE_CASE.id())).count();
        let notes = emitted.matches(&format!("note: {}", SCREAMING_SNAKE_CASE.id())).count();

        assert_eq!(warnings, 0, "Expected 0 warnings");
        assert_eq!(notes, 9, "Expected 9 notes");

        Ok(())
    }

    #[test]
    fn test_struct_pascal_case() -> eyre::Result<()> {
        let linter = SolidityLinter::new()
            .with_lints(Some(vec![STRUCT_PASCAL_CASE]))
            .with_buffer_emitter(true);

        let emitted =
            linter.lint_file(Path::new("testdata/StructPascalCase.sol")).unwrap().to_string();
        let warnings = emitted.matches(&format!("warning: {}", STRUCT_PASCAL_CASE.id())).count();
        let notes = emitted.matches(&format!("note: {}", STRUCT_PASCAL_CASE.id())).count();

        assert_eq!(warnings, 0, "Expected 0 warnings");
        assert_eq!(notes, 7, "Expected 7 notes");

        Ok(())
    }
}
