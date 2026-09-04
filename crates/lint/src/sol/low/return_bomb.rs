use super::ReturnBomb;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{
        Severity, SolLint,
        analysis::{is_address_like, is_call_with_gas_limit},
    },
};
use solar::{
    interface::kw,
    sema::{
        Gcx, Ty,
        hir::{self, ExprKind},
        ty::TyKind,
    },
};

declare_forge_lint!(
    RETURN_BOMB,
    Severity::Low,
    "return-bomb",
    "external calls with a gas limit should not consume unbounded return data"
);

impl<'hir> LateLintPass<'hir> for ReturnBomb {
    fn check_expr(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'hir>,
        _hir: &'hir hir::Hir<'hir>,
        expr: &'hir hir::Expr<'hir>,
    ) {
        // Flag gas-limited calls that can force the caller to copy unbounded returndata: a
        // low-level call on an address, or a call returning dynamic data.
        let expr = expr.peel_parens();
        if !is_call_with_gas_limit(expr) {
            return;
        }
        let ExprKind::Call(callee, ..) = &expr.kind else { return };
        let low_level = matches!(&callee.peel_parens().kind, ExprKind::Member(receiver, member)
            if matches!(member.name, kw::Call | kw::Delegatecall | kw::Staticcall)
                && is_address_like(gcx, receiver));
        if low_level || gcx.type_of_expr(expr.id).is_some_and(|ty| is_dynamic_ty(gcx, ty)) {
            ctx.emit(&RETURN_BOMB, expr.span);
        }
    }
}

fn is_dynamic_ty<'hir>(gcx: Gcx<'hir>, ty: Ty<'hir>) -> bool {
    let ty = ty.peel_refs();
    match ty.kind {
        TyKind::Struct(id) => {
            ty.is_dynamically_encoded(gcx)
                || gcx.struct_field_types(id).iter().any(|ty| is_dynamic_ty(gcx, *ty))
        }
        TyKind::Tuple(elements) => elements.iter().any(|ty| is_dynamic_ty(gcx, *ty)),
        _ => ty.is_dynamically_encoded(gcx),
    }
}
