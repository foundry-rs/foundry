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
    "`delete` on a struct containing a mapping does not clear the mapping"
);

impl<'hir> LateLintPass<'hir> for MappingDeletion {
    fn check_expr(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'hir>,
        hir: &'hir hir::Hir<'hir>,
        expr: &'hir hir::Expr<'hir>,
    ) {
        // `delete <expr>` where the operand's type reaches a `mapping`. Deleting a whole mapping is
        // not valid Solidity, so the operand is a struct/array that holds one.
        if let ExprKind::Delete(operand) = &expr.kind
            && let Some(ty) = gcx.type_of_expr(operand.peel_parens().id)
            && ty_contains_mapping(gcx, hir, ty, &mut Vec::new())
        {
            ctx.emit(&MAPPING_DELETION, expr.span);
        }
    }
}

/// Returns `true` if `ty` is, or transitively contains, a `mapping`.
///
/// `delete` zeroes each member of a storage value, but it cannot enumerate a mapping's keys, so any
/// mapping reachable from `ty` keeps its entries. Recurses through structs and arrays; `seen`
/// guards against recursive struct definitions (only reachable through mapping/array members, which
/// Solidity permits).
fn ty_contains_mapping<'hir>(
    gcx: Gcx<'hir>,
    hir: &'hir hir::Hir<'hir>,
    ty: Ty<'hir>,
    seen: &mut Vec<hir::StructId>,
) -> bool {
    match ty.peel_refs().kind {
        TyKind::Mapping(..) => true,
        TyKind::Array(elem, _) | TyKind::DynArray(elem) | TyKind::Slice(elem) => {
            ty_contains_mapping(gcx, hir, elem, seen)
        }
        TyKind::Struct(id) => {
            if seen.contains(&id) {
                return false;
            }
            seen.push(id);
            hir.strukt(id)
                .fields
                .iter()
                .any(|&field| ty_contains_mapping(gcx, hir, gcx.type_of_item(field.into()), seen))
        }
        _ => false,
    }
}
