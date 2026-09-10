use super::{MsgValueLoop, payable_loop::for_each_payable_loop_expr};
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::sema::{Gcx, builtins::Builtin, hir::Function};

declare_forge_lint!(
    MSG_VALUE_LOOP,
    Severity::Low,
    "msg-value-loop",
    "payable function uses `msg.value` inside a loop"
);

impl<'gcx> LateLintPass<'gcx> for MsgValueLoop {
    fn check_function(&mut self, ctx: &LintContext, gcx: Gcx<'gcx>, func: &'gcx Function<'gcx>) {
        for_each_payable_loop_expr(gcx, func, |expr| {
            if gcx.resolved_builtin(expr) == Some(Builtin::MsgValue) {
                ctx.emit(&MSG_VALUE_LOOP, expr.span);
            }
        });
    }
}
