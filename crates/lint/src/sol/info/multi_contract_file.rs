use crate::{
    linter::{EarlyLintPass, Lint, LintContext},
    sol::{Severity, SolLint, info::MultiContractFile},
};

use solar::ast::{self as ast};

declare_forge_lint!(
    MULTI_CONTRACT_FILE,
    Severity::Info,
    "multi-contract-file",
    "prefer having only one contract, interface or library per file"
);

impl<'ast> EarlyLintPass<'ast> for MultiContractFile {
    fn check_full_source_unit(
        &mut self,
        ctx: &LintContext<'ast, '_>,
        unit: &'ast ast::SourceUnit<'ast>,
    ) {
        if !ctx.is_lint_enabled(MULTI_CONTRACT_FILE.id()) {
            return;
        }

        // Collect spans of all contract-like items, skipping those that are exempted
        let relevant_spans: Vec<_> = unit
            .items
            .iter()
            .filter_map(|item| match &item.kind {
                ast::ItemKind::Contract(c) => {
                    (!ctx.config.lint_specific.is_exempted(&c.kind)).then_some(c.name.span)
                }
                _ => None,
            })
            .collect();

        // Flag all if there's more than one
        if relevant_spans.len() > 1 {
            relevant_spans.into_iter().for_each(|span| ctx.emit(&MULTI_CONTRACT_FILE, span));
        }
    }
}
