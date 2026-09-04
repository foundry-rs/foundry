use super::MappingDeletion;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::sema::{
    Gcx,
    hir::{self, ExprKind},
    ty::{Ty, TyKind},
};

declare_forge_lint!(
    MAPPING_DELETION,
    Severity::Med,
    "mapping-deletion",
    "`delete` on a value containing a mapping does not clear the mapping"
);

impl<'hir> LateLintPass<'hir> for MappingDeletion {
    fn check_expr(&mut self, ctx: &LintContext, gcx: Gcx<'hir>, expr: &'hir hir::Expr<'hir>) {
        if let ExprKind::Delete(operand) = &expr.kind
            && let Some(ty) = gcx.type_of_expr(operand.peel_parens().id)
            && ty_contains_mapping(gcx, ty, &mut Vec::new())
        {
            ctx.emit(&MAPPING_DELETION, expr.span);
        }
    }
}

/// True if `ty` is, or transitively contains, a `mapping`. `delete` cannot enumerate a mapping's
/// keys, so the entries survive. `seen` guards against recursive struct definitions.
fn ty_contains_mapping<'hir>(gcx: Gcx<'hir>, ty: Ty<'hir>, seen: &mut Vec<hir::StructId>) -> bool {
    match ty.peel_refs().kind {
        TyKind::Mapping(..) => true,
        TyKind::Array(elem, _) | TyKind::DynArray(elem) | TyKind::Slice(elem) => {
            ty_contains_mapping(gcx, elem, seen)
        }
        TyKind::Struct(id) if !seen.contains(&id) => {
            seen.push(id);
            gcx.hir
                .strukt(id)
                .fields
                .iter()
                .any(|&field| ty_contains_mapping(gcx, gcx.type_of_item(field.into()), seen))
        }
        _ => false,
    }
}
