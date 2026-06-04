use super::ReturnBomb;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint, calls::is_call_with_gas_limit},
};
use solar::{
    ast::ElementaryType,
    interface::{Symbol, kw, sym},
    sema::{
        Gcx, Ty,
        hir::{self, ExprKind, TypeKind},
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
        // Flag gas-limited calls that can force the caller to copy unbounded returndata.
        if low_level_call_with_gas_consumes_unbounded_return_data(gcx, expr)
            || call_with_gas_returns_dynamic_data(gcx, expr)
        {
            ctx.emit(&RETURN_BOMB, expr.span);
        }
    }
}

/// Returns true for gas-limited calls that return dynamic data.
fn call_with_gas_returns_dynamic_data<'hir>(gcx: Gcx<'hir>, expr: &'hir hir::Expr<'hir>) -> bool {
    is_call_with_gas_limit(expr)
        && gcx.type_of_expr(expr.peel_parens().id).is_some_and(|ty| is_dynamic_ty(gcx, ty))
}

/// Returns true for gas-limited low-level calls that copy unbounded returndata.
fn low_level_call_with_gas_consumes_unbounded_return_data<'hir>(
    gcx: Gcx<'hir>,
    expr: &'hir hir::Expr<'hir>,
) -> bool {
    if !is_call_with_gas_limit(expr) {
        return false;
    }

    let ExprKind::Call(callee, _, _) = &expr.peel_parens().kind else { return false };
    let ExprKind::Member(receiver, member) = &callee.peel_parens().kind else { return false };
    matches!(member.name, kw::Call | kw::Delegatecall | kw::Staticcall)
        && expr_is_address(gcx, receiver)
}
/// Returns true if an expression is known to have an address type.
fn expr_is_address<'hir>(gcx: Gcx<'hir>, expr: &'hir hir::Expr<'hir>) -> bool {
    match &expr.peel_parens().kind {
        ExprKind::Payable(_) => true,
        ExprKind::Call(callee, _, _) => {
            matches!(
                &callee.peel_parens().kind,
                ExprKind::Type(hir::Type {
                    kind: TypeKind::Elementary(ElementaryType::Address(_)),
                    ..
                })
            ) || callee_is_address_returning_builtin(callee)
                || gcx.type_of_expr(expr.peel_parens().id).is_some_and(ty_is_address)
        }
        ExprKind::Member(base, member) if member_is_builtin_address(base, member.name) => true,
        _ => gcx.type_of_expr(expr.peel_parens().id).is_some_and(ty_is_address),
    }
}

fn callee_is_address_returning_builtin(callee: &hir::Expr<'_>) -> bool {
    let ExprKind::Ident(reses) = &callee.peel_parens().kind else { return false };
    reses
        .iter()
        .any(|res| matches!(res, hir::Res::Builtin(builtin) if builtin.name() == sym::ecrecover))
}

fn member_is_builtin_address(base: &hir::Expr<'_>, member: Symbol) -> bool {
    let ExprKind::Ident(reses) = &base.peel_parens().kind else { return false };
    reses.iter().any(|res| {
        let hir::Res::Builtin(builtin) = res else { return false };
        matches!(
            (builtin.name(), member),
            (sym::msg, sym::sender) | (sym::block, kw::Coinbase) | (sym::tx, kw::Origin)
        )
    })
}

fn ty_is_address(ty: Ty<'_>) -> bool {
    matches!(ty.peel_refs().kind, TyKind::Elementary(ElementaryType::Address(_)))
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
