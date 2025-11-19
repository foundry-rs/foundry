use crate::{
    linter::{EarlyLintPass, Lint, LintContext},
    sol::{Severity, SolLint, info::MultiContractFile},
};

use foundry_config::lint::ContractException;
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

        // Check which types are exempted
        let exceptions = &ctx.config.lint_specific.multi_contract_file_exceptions;
        let allow_interfaces = exceptions.contains(&ContractException::Interface);
        let allow_libraries = exceptions.contains(&ContractException::Library);
        let allow_abstract = exceptions.contains(&ContractException::AbstractContract);

        // Collect spans of all contract-like items, skipping those that are exempted
        let relevant_spans: Vec<_> = unit
            .items
            .iter()
            .filter_map(|item| match &item.kind {
                ast::ItemKind::Contract(c) => {
                    let should_skip = match c.kind {
                        ast::ContractKind::Interface => allow_interfaces,
                        ast::ContractKind::Library => allow_libraries,
                        ast::ContractKind::AbstractContract => allow_abstract,
                        ast::ContractKind::Contract => false, // Regular contracts are never skipped
                    };
                    (!should_skip).then_some(c.name.span)
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
