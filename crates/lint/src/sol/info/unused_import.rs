use solar_ast::{Expr, ExprKind, ImportItems, PathSlice, Symbol, TypeKind, UsingList};

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
    /// Collects all the file imports and caches them in `LintContext`.
    fn check_import_directive(
        &mut self,
        ctx: &mut LintContext<'_>,
        import: &'ast solar_ast::ImportDirective<'ast>,
    ) {
        match import.items {
            ImportItems::Aliases(ref items) => {
                for item in &**items {
                    let (name, span) = if let Some(ref i) = &item.1 {
                        (&i.name, &i.span)
                    } else {
                        (&item.0.name, &item.0.span)
                    };

                    ctx.add_import((*name, *span));
                }
            }
            ImportItems::Glob(ref ident) => {
                ctx.add_import((ident.name, ident.span));
            }
            ImportItems::Plain(ref maybe) => match maybe {
                Some(ident) => ctx.add_import((ident.name, ident.span)),
                None => {
                    let path = import.path.value.to_string();
                    let len = path.len() - 4;
                    ctx.add_import((Symbol::intern(&path[..len]), import.path.span));
                }
            },
        }
    }

    /// Marks contract modifiers as used, effectively removing them from the `LintContext` cache.
    fn check_item_contract(
        &mut self,
        ctx: &mut LintContext<'_>,
        contract: &'ast solar_ast::ItemContract<'ast>,
    ) {
        for modifier in &*contract.bases {
            use_import_type(ctx, &modifier.name);
        }
    }

    /// Marks variable definitions (both, variable type and initializer name) as used,
    /// effectively removing them from the `LintContext` cache.
    fn check_variable_definition(
        &mut self,
        ctx: &mut LintContext<'_>,
        var: &'ast solar_ast::VariableDefinition<'ast>,
    ) {
        if let TypeKind::Custom(ty) = &var.ty.kind {
            use_import_type(ctx, ty);
        }

        if let Some(expr) = &var.initializer {
            use_import_expr(ctx, expr);
        }
    }

    /// Marks the types in a using directive as used, effectively removing them from the
    /// `LintContext` cache.
    fn check_using_directive(
        &mut self,
        ctx: &mut LintContext<'_>,
        using: &'ast solar_ast::UsingDirective<'ast>,
    ) {
        match &using.list {
            UsingList::Single(ty) => use_import_type(ctx, ty),
            UsingList::Multiple(types) => {
                for (ty, _operator) in &**types {
                    use_import_type(ctx, ty);
                }
            }
        }
    }
}

/// Marks the type as used.
/// If the type has more than one segment, it marks both, the first, and the last one.
fn use_import_type(ctx: &mut LintContext<'_>, ty: &&mut PathSlice) {
    ctx.use_import(ty.last().name);
    if ty.segments().len() != 1 {
        ctx.use_import(ty.first().name);
    }
}

/// Marks the type as used.
/// If the type has more than one segment, it marks both, the first, and the last one.
fn use_import_expr<'ast>(ctx: &mut LintContext<'_>, expr: &&mut Expr<'ast>) {
    match &expr.kind {
        ExprKind::Ident(ident) => ctx.use_import(ident.name),
        ExprKind::Member(ref expr, _) => use_import_expr(ctx, expr),
        ExprKind::Call(ref expr, _) => use_import_expr(ctx, expr),
        _ => (),
    }
}
