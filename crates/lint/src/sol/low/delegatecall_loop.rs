use super::{
    DelegatecallLoop,
    payable_loop::{expr_ty, is_address_ty, is_this_or_super, visit_payable_loop_expressions},
};
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    interface::kw,
    sema::{
        Gcx,
        hir::{Expr, ExprKind, Function, Hir},
    },
};
use std::collections::HashSet;

declare_forge_lint!(
    DELEGATECALL_LOOP,
    Severity::Low,
    "delegatecall-loop",
    "payable functions should not use `delegatecall` inside a loop"
);

impl<'hir> LateLintPass<'hir> for DelegatecallLoop {
    fn check_function(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'hir>,
        hir: &'hir Hir<'hir>,
        func: &'hir Function<'hir>,
    ) {
        let mut emitted = HashSet::new();
        visit_payable_loop_expressions(ctx, gcx, hir, func, |ctx, gcx, hir, expr| {
            if is_delegatecall(gcx, hir, expr) && emitted.insert(expr.span) {
                ctx.emit(&DELEGATECALL_LOOP, expr.span);
            }
        });
    }
}

fn is_delegatecall<'hir>(gcx: Gcx<'hir>, hir: &'hir Hir<'hir>, expr: &'hir Expr<'hir>) -> bool {
    let ExprKind::Call(call_expr, _, _) = &expr.kind else {
        return false;
    };
    let ExprKind::Member(receiver, member) = &call_expr.peel_parens().kind else {
        return false;
    };
    if member.name != kw::Delegatecall {
        return false;
    }
    if is_this_or_super(receiver) {
        return false;
    }

    expr_ty(gcx, hir, receiver).is_some_and(is_address_ty)
}
