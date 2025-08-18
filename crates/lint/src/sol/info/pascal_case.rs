use super::PascalCaseStruct;
use crate::{
    linter::{EarlyLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar_ast::ItemStruct;

declare_forge_lint!(
    PASCAL_CASE_STRUCT,
    Severity::Info,
    "pascal-case-struct",
    "structs should use PascalCase"
);

impl<'ast> EarlyLintPass<'ast> for PascalCaseStruct {
    fn check_item_struct(&mut self, ctx: &LintContext<'_>, strukt: &'ast ItemStruct<'ast>) {
        let name = strukt.name.as_str();
        if name.len() > 1 && !is_pascal_case(name) {
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
