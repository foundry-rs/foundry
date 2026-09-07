use super::{UncheckedCall, UncheckedTransferERC20};
use crate::{
    linter::{EarlyLintPass, LateLintPass, LintContext},
    sol::{
        Severity, SolLint,
        analysis::{is_elementary, is_low_level_call, receiver_contract_id},
    },
};
use solar::{
    ast::{ExprKind, Stmt, StmtKind},
    sema::{Gcx, hir},
};

declare_forge_lint!(
    UNCHECKED_CALL,
    Severity::High,
    "unchecked-call",
    "Low-level calls should check the success return value"
);

declare_forge_lint!(
    ERC20_UNCHECKED_TRANSFER,
    Severity::High,
    "erc20-unchecked-transfer",
    "ERC20 'transfer' and 'transferFrom' calls should check the return value"
);

/// Checks that calls to functions with the same signature as the ERC20 transfer methods, and which
/// return a boolean, are not ignored.
///
/// WARN: can issue false positives, as it doesn't check that the contract being called sticks to
/// the full ERC20 specification.
impl<'gcx> LateLintPass<'gcx> for UncheckedTransferERC20 {
    fn check_stmt(&mut self, ctx: &LintContext, gcx: Gcx<'gcx>, stmt: &'gcx hir::Stmt<'gcx>) {
        // Only expression statements can contain unchecked transfers.
        if let hir::StmtKind::Expr(expr) = &stmt.kind
            && is_erc20_transfer_call(gcx, expr)
        {
            ctx.emit(&ERC20_UNCHECKED_TRANSFER, expr.span);
        }
    }
}

/// Checks if an expression is a call to a contract member matching the ERC20 signature of
/// * `function transfer(address to, uint256 amount) external returns (bool);`
/// * `function transferFrom(address from, address to, uint256 amount) external returns (bool);`
fn is_erc20_transfer_call<'gcx>(gcx: Gcx<'gcx>, expr: &hir::Expr<'gcx>) -> bool {
    let hir::ExprKind::Call(callee, call_args, ..) = &expr.kind else { return false };
    let hir::ExprKind::Member(receiver, func_ident) = &callee.kind else { return false };
    let params: &[&str] = match (func_ident.as_str(), call_args.len()) {
        ("transfer", 2) => &["address", "uint256"],
        ("transferFrom", 3) => &["address", "address", "uint256"],
        _ => return false,
    };
    let Some(cid) = receiver_contract_id(gcx, receiver) else { return false };
    gcx.hir.contract_item_ids(cid).filter_map(|item| item.as_function()).any(|fid| {
        let func = gcx.hir.function(fid);
        func.name.is_some_and(|name| name.name == func_ident.name)
            && func.kind.is_function()
            && func.mutates_state()
            && func.parameters.len() == params.len()
            && func.parameters.iter().zip(params).all(|(id, ty)| is_elementary(&gcx.hir, *id, ty))
            && matches!(func.returns, [ret] if is_elementary(&gcx.hir, *ret, "bool"))
    })
}

/// Unchecked low-level calls appear as standalone expression statements, or with the success
/// value discarded in a tuple. When the success value is checked (in require, if, etc.), the
/// call is part of a larger expression and is not flagged.
impl<'ast> EarlyLintPass<'ast> for UncheckedCall {
    fn check_stmt(&mut self, ctx: &LintContext, stmt: &'ast Stmt<'ast>) {
        let span = match &stmt.kind {
            // `target.call(data);` and `(, existingVar) = target.call(data);`
            StmtKind::Expr(expr)
                if is_low_level_call(expr)
                    || matches!(&expr.kind, ExprKind::Assign(lhs, _, rhs)
                        if is_low_level_call(rhs)
                            && matches!(&lhs.kind, ExprKind::Tuple(elements)
                                if elements.first().is_none_or(|e| e.is_none()))) =>
            {
                expr.span
            }
            // `(, bytes memory data) = target.call(data);`
            StmtKind::DeclMulti(vars, expr)
                if is_low_level_call(expr) && vars.first().is_none_or(|v| v.is_none()) =>
            {
                stmt.span
            }
            _ => return,
        };
        ctx.emit(&UNCHECKED_CALL, span);
    }
}
