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

        // Count contract-like items (contracts, interfaces, libraries) in this source unit.
        let count = unit.count_contracts();

        if count > 1 {
            // Point at the second contract's name to make the diagnostic actionable.
            unit.items
                .iter()
                .filter_map(|item| match &item.kind {
                    ast::ItemKind::Contract(c) => Some(c.name.span),
                    _ => None,
                })
                .skip(1)
                .for_each(|span| ctx.emit(&MULTI_CONTRACT_FILE, span));
        }
    }
}
