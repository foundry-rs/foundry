use super::MappingDeletion;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::sema::{
    Gcx,
    hir::{self, ExprKind},
};

declare_forge_lint!(
    MAPPING_DELETION,
    Severity::Med,
    "mapping-deletion",
    "`delete` on a value containing a mapping does not clear the mapping"
);

impl<'gcx> LateLintPass<'gcx> for MappingDeletion {
    fn check_expr(&mut self, ctx: &LintContext, gcx: Gcx<'gcx>, expr: &'gcx hir::Expr<'gcx>) {
        if let ExprKind::Delete(operand) = &expr.kind
            && let Some(ty) = gcx.type_of_expr(operand.peel_parens().id)
            && ty.has_mapping(gcx)
        {
            ctx.emit(&MAPPING_DELETION, expr.span);
        }
    }
}
