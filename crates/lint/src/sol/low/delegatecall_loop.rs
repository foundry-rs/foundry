use super::{DelegatecallLoop, payable_loop::for_each_payable_loop_expr};
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::sema::{
    Gcx,
    builtins::Builtin,
    hir::{ExprKind, Function},
};

declare_forge_lint!(
    DELEGATECALL_LOOP,
    Severity::Low,
    "delegatecall-loop",
    "payable function uses `delegatecall` inside a loop"
);

impl<'gcx> LateLintPass<'gcx> for DelegatecallLoop {
    fn check_function(&mut self, ctx: &LintContext, gcx: Gcx<'gcx>, func: &'gcx Function<'gcx>) {
        for_each_payable_loop_expr(gcx, func, |expr| {
            if let ExprKind::Call(callee, ..) = &expr.kind
                && gcx.resolved_builtin(callee) == Some(Builtin::AddressDelegatecall)
            {
                ctx.emit(&DELEGATECALL_LOOP, expr.span);
            }
        });
    }
}
