use super::UnusedReturn;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{
        Severity, SolLint,
        analysis::{is_elementary, tuple_elems},
    },
};
use solar::sema::{
    Gcx,
    hir::{Expr, ExprKind, Stmt, StmtKind},
    ty::{TyFnKind, TyKind},
};

declare_forge_lint!(
    UNUSED_RETURN,
    Severity::Med,
    "unused-return",
    "return value of an external call is not used"
);

impl<'gcx> LateLintPass<'gcx> for UnusedReturn {
    fn check_stmt(&mut self, ctx: &LintContext, gcx: Gcx<'gcx>, stmt: &'gcx Stmt<'gcx>) {
        let (call, span) = match &stmt.kind {
            StmtKind::Expr(expr) => match &expr.peel_parens().kind {
                // `(x, ) = call()` with an ignored slot.
                ExprKind::Assign(lhs, None, rhs)
                    if tuple_elems(lhs).is_some_and(|e| e.iter().any(Option::is_none)) =>
                {
                    (rhs, expr.span)
                }
                _ => (expr, expr.span),
            },
            StmtKind::DeclMulti(vars, expr) if vars.iter().any(Option::is_none) => {
                (expr, expr.span)
            }
            _ => return,
        };
        if is_unused_return_call(gcx, call) {
            ctx.emit(&UNUSED_RETURN, span);
        }
    }
}

/// True if `expr` is an external member call whose selected function has return values,
/// excluding ERC20 `transfer`/`transferFrom` (covered by
/// `erc20-unchecked-transfer`).
fn is_unused_return_call<'gcx>(gcx: Gcx<'gcx>, expr: &Expr<'gcx>) -> bool {
    let ExprKind::Call(callee, ..) = &expr.peel_parens().kind else { return false };
    let ExprKind::Member(_, name) = &callee.peel_parens().kind else { return false };
    let Some(ty) = gcx.type_of_expr(callee.peel_parens().id) else { return false };
    if !matches!(ty.kind, TyKind::Fn(f) if matches!(f.kind(), TyFnKind::External | TyFnKind::DelegateCall))
    {
        return false;
    }
    let Some(fid) = gcx.resolved_function(callee) else { return false };
    let f = gcx.hir.function(fid);

    let sig = |vars: &[_], expected: &[&str]| {
        vars.len() == expected.len()
            && vars.iter().zip(expected).all(|(&id, &ty)| is_elementary(&gcx.hir, id, ty))
    };
    let is_erc20_transfer = sig(f.returns, &["bool"])
        && match name.as_str() {
            "transfer" => sig(f.parameters, &["address", "uint256"]),
            "transferFrom" => sig(f.parameters, &["address", "address", "uint256"]),
            _ => false,
        };
    !f.returns.is_empty() && !is_erc20_transfer
}
