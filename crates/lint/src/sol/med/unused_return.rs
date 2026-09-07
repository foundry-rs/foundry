use super::UnusedReturn;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{
        Severity, SolLint,
        analysis::{is_elementary, receiver_contract_id, tuple_elems},
    },
};
use solar::sema::{
    Gcx,
    hir::{Expr, ExprKind, Stmt, StmtKind},
};

declare_forge_lint!(
    UNUSED_RETURN,
    Severity::Med,
    "unused-return",
    "Return value of an external call is not used"
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

/// True if `expr` is a member call on a contract whose every candidate function (same name and
/// arity) has return values, excluding ERC20 `transfer`/`transferFrom` (covered by
/// `erc20-unchecked-transfer`).
fn is_unused_return_call<'gcx>(gcx: Gcx<'gcx>, expr: &Expr<'gcx>) -> bool {
    let ExprKind::Call(callee, args, ..) = &expr.peel_parens().kind else { return false };
    let ExprKind::Member(receiver, name) = &callee.peel_parens().kind else { return false };
    let Some(cid) = receiver_contract_id(gcx, receiver) else { return false };

    let sig = |vars: &[_], expected: &[&str]| {
        vars.len() == expected.len()
            && vars.iter().zip(expected).all(|(&id, &ty)| is_elementary(&gcx.hir, id, ty))
    };
    let mut candidates = gcx
        .hir
        .contract_item_ids(cid)
        .filter_map(|item| item.as_function())
        .map(|fid| gcx.hir.function(fid))
        .filter(|f| {
            f.kind.is_function()
                && f.name.is_some_and(|n| n.name == name.name)
                && f.parameters.len() == args.kind.len()
        })
        .peekable();
    candidates.peek().is_some()
        && candidates.all(|f| {
            let is_erc20_transfer = sig(f.returns, &["bool"])
                && match name.as_str() {
                    "transfer" => sig(f.parameters, &["address", "uint256"]),
                    "transferFrom" => sig(f.parameters, &["address", "address", "uint256"]),
                    _ => false,
                };
            !f.returns.is_empty() && !is_erc20_transfer
        })
}
