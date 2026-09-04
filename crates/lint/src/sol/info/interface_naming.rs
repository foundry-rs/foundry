use super::InterfaceFileNaming;
use crate::{
    linter::{EarlyLintPass, Lint, LintContext},
    sol::{Severity, SolLint},
};
use solar::ast;

declare_forge_lint!(
    INTERFACE_FILE_NAMING,
    Severity::Info,
    "interface-file-naming",
    "interface file names should be prefixed with 'I'"
);

declare_forge_lint!(
    INTERFACE_NAMING,
    Severity::Info,
    "interface-naming",
    "interface names should be prefixed with 'I'"
);

impl<'ast> EarlyLintPass<'ast> for InterfaceFileNaming {
    fn check_full_source_unit(
        &mut self,
        ctx: &LintContext<'ast, '_>,
        unit: &'ast ast::SourceUnit<'ast>,
    ) {
        if !ctx.is_lint_enabled(INTERFACE_FILE_NAMING.id()) {
            return;
        }
        // A file whose contract-like items are all interfaces is named after the first one.
        let mut contracts = unit.items.iter().filter_map(|item| match &item.kind {
            ast::ItemKind::Contract(c) => Some(c),
            _ => None,
        });
        if let Some(first) = contracts.next()
            && std::iter::once(first).chain(contracts).all(|c| c.kind == ast::ContractKind::Interface)
            && let Some(file_name) = file_name(ctx, unit)
            && !file_name.starts_with('I')
        {
            ctx.emit(&INTERFACE_FILE_NAMING, first.name.span);
        }
    }

    fn check_item_contract(&mut self, ctx: &LintContext, contract: &'ast ast::ItemContract<'ast>) {
        if contract.kind == ast::ContractKind::Interface && !contract.name.as_str().starts_with('I') {
            ctx.emit(&INTERFACE_NAMING, contract.name.span);
        }
    }
}

fn file_name(ctx: &LintContext, unit: &ast::SourceUnit<'_>) -> Option<String> {
    let first_item_span = unit.items.first()?.span;
    let file = ctx.session().source_map().lookup_source_file(first_item_span.lo());
    Some(file.name.as_real()?.file_name()?.to_str()?.to_string())
}
