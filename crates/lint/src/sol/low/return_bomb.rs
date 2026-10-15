use super::ReturnBomb;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint, analysis::is_call_with_gas_limit},
};
use solar::sema::{
    Gcx, Ty,
    builtins::Builtin,
    hir::{self, ExprKind},
    ty::TyKind,
};

declare_forge_lint!(
    RETURN_BOMB,
    Severity::Low,
    "return-bomb",
    "external call with a gas limit may copy unbounded return data"
);

impl<'gcx> LateLintPass<'gcx> for ReturnBomb {
    fn check_expr(&mut self, ctx: &LintContext, gcx: Gcx<'gcx>, expr: &'gcx hir::Expr<'gcx>) {
        // Flag gas-limited calls that can force the caller to copy unbounded returndata: a
        // low-level call on an address, or a call returning dynamic data.
        let expr = expr.peel_parens();
        if !is_call_with_gas_limit(expr) {
            return;
        }
        let ExprKind::Call(callee, ..) = &expr.kind else { return };
        let low_level = matches!(
            gcx.resolved_builtin(callee),
            Some(Builtin::AddressCall | Builtin::AddressDelegatecall | Builtin::AddressStaticcall)
        );
        if low_level || gcx.type_of_expr(expr.id).is_some_and(|ty| is_dynamic_ty(gcx, ty)) {
            ctx.emit(&RETURN_BOMB, expr.span);
        }
    }
}

fn is_dynamic_ty<'gcx>(gcx: Gcx<'gcx>, ty: Ty<'gcx>) -> bool {
    let ty = ty.peel_refs();
    match ty.kind {
        TyKind::Tuple(elements) => elements.iter().any(|ty| is_dynamic_ty(gcx, *ty)),
        _ => ty.is_dynamically_encoded(gcx),
    }
}
