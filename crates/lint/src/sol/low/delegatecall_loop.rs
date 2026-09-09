use super::{DelegatecallLoop, payable_loop::for_each_payable_loop_expr};
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint, analysis::expr_is_address},
};
use solar::{
    interface::kw,
    sema::{
        Gcx,
        hir::{ExprKind, Function},
    },
};

declare_forge_lint!(DELEGATECALL_LOOP, Severity::Low, "delegatecall-loop");

impl<'gcx> LateLintPass<'gcx> for DelegatecallLoop {
    fn check_function(&mut self, ctx: &LintContext, gcx: Gcx<'gcx>, func: &'gcx Function<'gcx>) {
        for_each_payable_loop_expr(gcx, func, |expr| {
            // Only `<address>.delegatecall(..)`: user functions named `delegatecall` on
            // contract-typed receivers are ordinary calls.
            if let ExprKind::Call(callee, ..) = &expr.kind
                && let ExprKind::Member(receiver, member) = &callee.peel_parens().kind
                && member.name == kw::Delegatecall
                && expr_is_address(gcx, receiver)
            {
                ctx.span_lint(&DELEGATECALL_LOOP, expr.span, |diag| {
                    diag.primary_message("payable function uses `delegatecall` inside a loop");
                });
            }
        });
    }
}
