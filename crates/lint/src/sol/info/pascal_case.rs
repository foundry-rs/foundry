use solar_ast::ItemStruct;

use super::PascalCaseStruct;
use crate::{
    declare_forge_lint,
    linter::{EarlyLintPass, LintContext},
    sol::{Severity, SolLint},
};

declare_forge_lint!(
    PASCAL_CASE_STRUCT,
    Severity::Info,
    "struct-pascal-case",
    "structs should use PascalCase.",
    "https://docs.soliditylang.org/en/latest/style-guide.html#struct-names"
);

impl<'ast> EarlyLintPass<'ast> for PascalCaseStruct {
    fn check_item_struct(&mut self, ctx: &LintContext<'_>, strukt: &'ast ItemStruct<'ast>) {
        let name = strukt.name.as_str();
        if !is_pascal_case(name) && name.len() > 1 {
            ctx.emit(&PASCAL_CASE_STRUCT, strukt.name.span);
        }
    }
}

/// Check if a string is PascalCase
pub fn is_pascal_case(s: &str) -> bool {
    if s.len() <= 1 {
        return true;
    }

    s == format!("{}", heck::AsPascalCase(s)).as_str()
}

#[cfg(test)]
mod test {
    use std::path::Path;

    use super::*;
    use crate::{linter::Lint, sol::SolidityLinter};

    #[test]
    fn test_struct_pascal_case() -> eyre::Result<()> {
        let linter = SolidityLinter::new().with_lints(Some(vec![PASCAL_CASE_STRUCT]));

        let emitted =
            linter.lint_test(Path::new("testdata/StructPascalCase.sol")).unwrap().to_string();
        let warnings = emitted.matches(&format!("warning[{}]", PASCAL_CASE_STRUCT.id())).count();
        let notes = emitted.matches(&format!("note[{}]", PASCAL_CASE_STRUCT.id())).count();

        assert_eq!(warnings, 0, "Expected 0 warnings");
        assert_eq!(notes, 6, "Expected 7 notes");

        Ok(())
    }
}
