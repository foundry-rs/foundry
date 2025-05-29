use solar_ast::{ImportItems, TypeKind, UsingList};

use super::UnusedImport;
use crate::{
    declare_forge_lint,
    linter::{EarlyLintPass, LintContext},
    sol::{Severity, SolLint},
};

declare_forge_lint!(
    UNUSED_IMPORT,
    Severity::Info,
    "unused-import",
    "unused imports should be removed"
);

impl<'ast> EarlyLintPass<'ast> for UnusedImport {
    fn check_import_directive(
        &mut self,
        ctx: &mut LintContext<'_>,
        import: &'ast solar_ast::ImportDirective<'ast>,
    ) {
        // to begin with, only check explicit imports
        if let ImportItems::Aliases(ref items) = import.items {
            for item in &**items {
                let (name, span) = if let Some(ref i) = &item.1 {
                    (&i.name, &i.span)
                } else {
                    (&item.0.name, &item.0.span)
                };

                ctx.add_import((name.clone(), span.clone()));
            }
        }
    }

    fn check_item_contract(
        &mut self,
        ctx: &mut LintContext<'_>,
        contract: &'ast solar_ast::ItemContract<'ast>,
    ) {
        for modifier in &*contract.bases {
            ctx.use_import(modifier.name.last().name.clone());
        }
    }

    fn check_variable_definition(
        &mut self,
        ctx: &mut LintContext<'_>,
        var: &'ast solar_ast::VariableDefinition<'ast>,
    ) {
        if let TypeKind::Custom(ty) = &var.ty.kind {
            ctx.use_import(ty.last().name.clone());
        }
    }

    fn check_using_directive(
        &mut self,
        ctx: &mut LintContext<'_>,
        using: &'ast solar_ast::UsingDirective<'ast>,
    ) {
        match &using.list {
            UsingList::Single(ty) => ctx.use_import(ty.last().name.clone()),
            UsingList::Multiple(types) => {
                for (ty, _operator) in &**types {
                    ctx.use_import(ty.last().name.clone());
                }
            }
        }
    }
}

#[cfg(test)]
mod test {

    #[test]
    fn test_unused_imports() {}
}
