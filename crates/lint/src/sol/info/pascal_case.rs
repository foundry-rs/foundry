use super::PascalCaseStruct;
use crate::{
    linter::{EarlyLintPass, LintContext},
    sol::{
        Severity, SolLint,
        naming::{check_pascal_case, emit_rename},
    },
};
use solar::ast::ItemStruct;

declare_forge_lint!(
    PASCAL_CASE_STRUCT,
    Severity::Info,
    "pascal-case-struct",
    "structs should use PascalCase"
);

impl<'ast> EarlyLintPass<'ast> for PascalCaseStruct {
    fn check_item_struct(&mut self, ctx: &LintContext, strukt: &'ast ItemStruct<'ast>) {
        if let Some(expected) = check_pascal_case(strukt.name.as_str()) {
            emit_rename(ctx, &PASCAL_CASE_STRUCT, strukt.name.span, expected);
        }
    }
}
