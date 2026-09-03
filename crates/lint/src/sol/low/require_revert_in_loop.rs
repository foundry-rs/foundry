use super::{RequireRevertInLoop, payable_loop::visit_loop_statements_and_expressions};
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::sema::{
    Gcx, Hir,
    builtins::Builtin,
    hir::{Expr, ExprKind, Function, Res, StmtKind},
};
use std::{cell::RefCell, collections::HashSet};

declare_forge_lint!(
    REQUIRE_REVERT_IN_LOOP,
    Severity::Low,
    "require-revert-in-loop",
    "`require` or `revert` inside a loop"
);

impl<'hir> LateLintPass<'hir> for RequireRevertInLoop {
    fn check_function(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'hir>,
        hir: &'hir Hir<'hir>,
        func: &'hir Function<'hir>,
    ) {
        let emitted = RefCell::new(HashSet::new());

        visit_loop_statements_and_expressions(
            ctx,
            gcx,
            hir,
            func,
            |ctx, _, _, stmt| {
                if let StmtKind::Revert(expr) = stmt.kind {
                    let mut emitted = emitted.borrow_mut();
                    emit_once(ctx, &mut emitted, expr);
                }
            },
            |ctx, _, _, expr| {
                if is_require_or_revert_call(expr) {
                    let mut emitted = emitted.borrow_mut();
                    emit_once(ctx, &mut emitted, expr);
                }
            },
        );
    }
}

fn emit_once(ctx: &LintContext, emitted: &mut HashSet<solar::interface::Span>, expr: &Expr<'_>) {
    if emitted.insert(expr.span) {
        ctx.emit(&REQUIRE_REVERT_IN_LOOP, expr.span);
    }
}

fn is_require_or_revert_call(expr: &Expr<'_>) -> bool {
    let ExprKind::Call(callee, _, _) = &expr.peel_parens().kind else { return false };
    let ExprKind::Ident(reses) = &callee.peel_parens().kind else { return false };

    reses.iter().any(|res| {
        matches!(
            res,
            Res::Builtin(
                Builtin::Require | Builtin::Revert | Builtin::RevertMsg | Builtin::YulRevert
            )
        )
    })
}
