use crate::{
    linter::{EarlyLintPass, Lint, LintContext},
    sol::{Severity, SolLint, info::InterfaceFileNaming},
};

use solar::ast::{self as ast};

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

        // Get first item in file and exit if the unit contains no items
        let Some(first_item) = unit.items.first() else { return };

        // Get file from first item
        let file = ctx.session().source_map().lookup_source_file(first_item.span.lo());

        // Get file name from file
        let Some(file_name) = file.name.as_real().and_then(|path| path.file_name()?.to_str())
        else {
            return;
        };

        // If file name starts with 'I', skip lint
        if file_name.starts_with('I') {
            return;
        }

        let mut first_interface_span = None;
        for item in unit.items.iter() {
            if let ast::ItemKind::Contract(c) = &item.kind {
                match c.kind {
                    ast::ContractKind::Interface => {
                        first_interface_span.get_or_insert(c.name.span);
                    }
                    _ => return, // Mixed file, skip lint
                }
            }
        }

        // Emit if file contains ONLY interfaces. Emit only on the first interface.
        if let Some(span) = first_interface_span {
            ctx.emit(&INTERFACE_FILE_NAMING, span);
        }
    }

    fn check_item_contract(&mut self, ctx: &LintContext, contract: &'ast ast::ItemContract<'ast>) {
        if !ctx.is_lint_enabled(INTERFACE_NAMING.id()) {
            return;
        }

        // Only check interfaces
        if contract.kind != ast::ContractKind::Interface {
            return;
        }

        // Check if interface name starts with 'I'
        let name = contract.name.as_str();
        if !name.starts_with('I') {
            ctx.emit(&INTERFACE_NAMING, contract.name.span);
        }
    }
}
