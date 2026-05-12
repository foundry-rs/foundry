use super::PascalCaseStruct;
use crate::{
    linter::{EarlyLintPass, LintContext, Suggestion},
    sol::{Severity, SolLint, naming::check_pascal_case},
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
        let name = strukt.name.as_str();
        if let Some(expected) = check_pascal_case(name) {
            ctx.emit_with_suggestion(
                &PASCAL_CASE_STRUCT,
                strukt.name.span,
                Suggestion::fix(
                    expected,
                    solar::interface::diagnostics::Applicability::MachineApplicable,
                )
                .with_desc("consider using"),
            );
        }
    }
}
