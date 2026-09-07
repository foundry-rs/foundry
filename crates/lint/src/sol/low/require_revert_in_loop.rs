use super::{
    RequireRevertInLoop,
    payable_loop::{LoopItem, for_each_loop_item},
};
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint, analysis::builtins},
};
use solar::sema::{
    Gcx,
    builtins::Builtin,
    hir::{Expr, ExprKind, Function, StmtKind},
};

declare_forge_lint!(
    REQUIRE_REVERT_IN_LOOP,
    Severity::Low,
    "require-revert-in-loop",
    "`require` or `revert` inside a loop"
);

impl<'gcx> LateLintPass<'gcx> for RequireRevertInLoop {
    fn check_function(&mut self, ctx: &LintContext, gcx: Gcx<'gcx>, func: &'gcx Function<'gcx>) {
        for_each_loop_item(gcx, func, false, |item| {
            let reported = match item {
                LoopItem::Stmt(stmt) => match stmt.kind {
                    StmtKind::Revert(expr) => Some(expr),
                    _ => None,
                },
                LoopItem::Expr(expr) => is_require_or_revert_call(expr).then_some(expr),
            };
            if let Some(expr) = reported {
                ctx.emit(&REQUIRE_REVERT_IN_LOOP, expr.span);
            }
        });
    }
}

/// `require(..)`, `revert(..)` or the Yul `revert(..)` builtin.
fn is_require_or_revert_call(expr: &Expr<'_>) -> bool {
    let ExprKind::Call(callee, ..) = &expr.peel_parens().kind else { return false };
    builtins(callee).any(|b| {
        matches!(b, Builtin::Require | Builtin::Revert | Builtin::RevertMsg | Builtin::YulRevert)
    })
}
