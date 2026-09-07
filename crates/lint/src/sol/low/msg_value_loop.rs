use super::{MsgValueLoop, payable_loop::for_each_payable_loop_expr};
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint, analysis::is_builtin},
};
use solar::{
    interface::sym,
    sema::{
        Gcx,
        hir::{ExprKind, Function},
    },
};

declare_forge_lint!(
    MSG_VALUE_LOOP,
    Severity::Low,
    "msg-value-loop",
    "payable functions should not use `msg.value` inside a loop"
);

impl<'gcx> LateLintPass<'gcx> for MsgValueLoop {
    fn check_function(&mut self, ctx: &LintContext, gcx: Gcx<'gcx>, func: &'gcx Function<'gcx>) {
        for_each_payable_loop_expr(gcx, func, |expr| {
            if let ExprKind::Member(base, member) = &expr.peel_parens().kind
                && member.name == sym::value
                && is_builtin(base, sym::msg)
            {
                ctx.emit(&MSG_VALUE_LOOP, expr.span);
            }
        });
    }
}
