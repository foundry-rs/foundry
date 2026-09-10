use super::AssertStateChange;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint, analysis::for_each_lhs_var},
};
use solar::{
    ast::{DataLocation, StateMutability},
    sema::{
        Gcx,
        builtins::Builtin,
        hir::{Expr, ExprKind},
    },
};
use std::ops::ControlFlow;

declare_forge_lint!(
    ASSERT_STATE_CHANGE,
    Severity::Med,
    "assert-state-change",
    "`assert()` contains a state-modifying expression"
);

impl<'gcx> LateLintPass<'gcx> for AssertStateChange {
    fn check_expr(&mut self, ctx: &LintContext, gcx: Gcx<'gcx>, expr: &'gcx Expr<'gcx>) {
        let ExprKind::Call(callee, args, _) = &expr.kind else { return };
        if gcx.resolved_builtin(callee) != Some(Builtin::Assert) {
            return;
        }
        for arg in args.exprs() {
            // Point the diagnostic at the first sub-expression that mutates state.
            if let ControlFlow::Break(span) = arg.visit(&mut |e| {
                if is_state_change(gcx, e) {
                    ControlFlow::Break(e.span)
                } else {
                    ControlFlow::Continue(())
                }
            }) {
                ctx.emit_with_msg(
                    &ASSERT_STATE_CHANGE,
                    span,
                    "`assert()` argument contains a state-modifying expression; \
                     `assert()` is for invariants, hoist the mutation before the `assert`, \
                     or use `require()` for validation",
                );
            }
        }
    }
}

fn is_state_change<'gcx>(gcx: Gcx<'gcx>, expr: &Expr<'gcx>) -> bool {
    match &expr.kind {
        ExprKind::Assign(lhs, ..) | ExprKind::Delete(lhs) => is_storage_lvalue(gcx, lhs),
        ExprKind::Unary(op, lhs) => op.kind.has_side_effects() && is_storage_lvalue(gcx, lhs),
        ExprKind::Call(callee, ..) => is_mutating_call(gcx, callee),
        _ => false,
    }
}

/// True if the lvalue is rooted in contract storage: a state variable or a local declared
/// `storage`, which aliases contract storage.
fn is_storage_lvalue(gcx: Gcx<'_>, expr: &Expr<'_>) -> bool {
    let mut found = false;
    for_each_lhs_var(gcx, expr, &mut |v| {
        let v = gcx.hir.variable(v);
        found |= v.is_state_variable() || v.data_location == Some(DataLocation::Storage);
    });
    found
}

fn is_mutating_call<'gcx>(gcx: Gcx<'gcx>, callee: &Expr<'gcx>) -> bool {
    if let ExprKind::Member(base, _) = &callee.kind {
        // `arr.push(..)` / `arr.pop()` on a storage array or `bytes`. The type check keeps
        // contract methods that happen to be named push/pop out of this heuristic.
        if matches!(
            gcx.resolved_builtin(callee),
            Some(Builtin::ArrayPush0 | Builtin::ArrayPush | Builtin::ArrayPop)
        ) && is_storage_lvalue(gcx, base)
        {
            return true;
        }
        // Low-level address calls always transfer value or execute foreign code. The receiver
        // must be address-like so contract methods named send/call/transfer are not caught.
        if matches!(
            gcx.resolved_builtin(callee),
            Some(
                Builtin::AddressCall
                    | Builtin::AddressDelegatecall
                    | Builtin::AddressPayableSend
                    | Builtin::AddressPayableTransfer
            )
        ) {
            return true;
        }
    }
    gcx.type_of_expr(callee.peel_parens().id)
        .and_then(|ty| ty.state_mutability())
        .is_some_and(|mutability| mutability > StateMutability::View)
}
