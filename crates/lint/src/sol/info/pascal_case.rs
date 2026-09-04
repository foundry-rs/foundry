use crate::{
    linter::{EarlyLintPass, LintContext},
    sol::{
        Severity, SolLint,
        naming::{check_pascal_case, emit_rename, has_acronym_exception},
    },
};
use foundry_config::lint::LintSpecificConfig;
use solar::ast::ItemStruct;
use std::sync::Arc;

declare_forge_lint!(
    PASCAL_CASE_STRUCT,
    Severity::Info,
    "pascal-case-struct",
    "structs should use PascalCase"
);

#[derive(Debug)]
pub(super) struct PascalCaseStructPass {
    config: Arc<LintSpecificConfig>,
}

impl PascalCaseStructPass {
    pub(super) const fn new(config: Arc<LintSpecificConfig>) -> Self {
        Self { config }
    }
}

impl<'ast> EarlyLintPass<'ast> for PascalCaseStructPass {
    fn check_item_struct(&mut self, ctx: &LintContext, strukt: &'ast ItemStruct<'ast>) {
        let name = strukt.name.as_str();
        // The acronym exceptions shared with the `mixed-case-*` lints keep `ERC20Data` valid.
        if has_acronym_exception(name, &self.config.mixed_case_exceptions, |pre| {
            pre == heck::AsUpperCamelCase(pre).to_string()
        }) {
            return;
        }
        if let Some(expected) = check_pascal_case(name) {
            emit_rename(ctx, &PASCAL_CASE_STRUCT, strukt.name.span, expected);
        }
    }
}
