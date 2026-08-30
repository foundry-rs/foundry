use super::{
    MsgValueLoop,
    payable_loop::{is_builtin, visit_payable_loop_expressions},
};
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    interface::sym,
    sema::{
        Gcx,
        hir::{Expr, ExprKind, Function, Hir},
    },
};
use std::collections::HashSet;

declare_forge_lint!(
    MSG_VALUE_LOOP,
    Severity::Low,
    "msg-value-loop",
    "payable functions should not use `msg.value` inside a loop"
);

impl<'hir> LateLintPass<'hir> for MsgValueLoop {
    fn check_function(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'hir>,
        hir: &'hir Hir<'hir>,
        func: &'hir Function<'hir>,
    ) {
        let mut emitted = HashSet::new();
        visit_payable_loop_expressions(ctx, gcx, hir, func, |ctx, _, _, expr| {
            if is_msg_value(expr) && emitted.insert(expr.span) {
                ctx.emit(&MSG_VALUE_LOOP, expr.span);
            }
        });
    }
}

fn is_msg_value(expr: &Expr<'_>) -> bool {
    let ExprKind::Member(base, member) = &expr.peel_parens().kind else {
        return false;
    };
    member.name == sym::value && is_builtin(base, sym::msg)
}
