use crate::{
    linter::{EarlyLintPass, Lint, LintContext},
    sol::{Severity, SolLint},
};

use foundry_config::lint::LintSpecificConfig;
use solar::ast;
use std::sync::Arc;

declare_forge_lint!(
    MULTI_CONTRACT_FILE,
    Severity::Info,
    "multi-contract-file",
    "prefer having only one contract, interface or library per file"
);

#[derive(Debug)]
pub(super) struct MultiContractFilePass {
    config: Arc<LintSpecificConfig>,
}

impl MultiContractFilePass {
    pub(super) const fn new(config: Arc<LintSpecificConfig>) -> Self {
        Self { config }
    }
}

impl<'ast> EarlyLintPass<'ast> for MultiContractFilePass {
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
                    (!self.config.is_exempted(&c.kind)).then_some(c.name.span)
                }
                _ => None,
            })
            .collect();

        // Flag all if there's more than one
        if relevant_spans.len() > 1 {
            for span in relevant_spans {
                ctx.emit(&MULTI_CONTRACT_FILE, span);
            }
        }
    }
}
