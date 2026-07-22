use crate::linter::LintContext;
use solar::{
    ast::{StateMutability, Visibility},
    sema::{
        Gcx,
        hir::{
            EffectiveBodyCx, EffectiveBodyVisitor, Expr, Function, FunctionId, FunctionKind, Hir,
            Stmt, visit_effective_body_dispatches,
        },
    },
};
use std::ops::ControlFlow;

pub(super) fn visit_payable_loop_expressions<'ctx, 's, 'hir, 'cb>(
    ctx: &'ctx LintContext<'s, 'ctx>,
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    function: FunctionId,
    f: impl FnMut(&'ctx LintContext<'s, 'ctx>, Gcx<'hir>, &'hir Hir<'hir>, &'hir Expr<'hir>) + 'cb,
    opaque_f: impl FnMut(&'ctx LintContext<'s, 'ctx>, Gcx<'hir>, &'hir Hir<'hir>, &'hir Expr<'hir>)
    + 'cb,
) {
    if !is_payable_entry_point(hir.function(function)) {
        return;
    }

    visit_loop_nodes(ctx, gcx, hir, function, true, true, |_, _, _, _| {}, f, opaque_f);
}

pub(super) fn visit_loop_statements_and_expressions<'ctx, 's, 'hir, 'cb>(
    ctx: &'ctx LintContext<'s, 'ctx>,
    gcx: Gcx<'hir>,
    hir: &'hir Hir<'hir>,
    function: FunctionId,
    stmt_f: impl FnMut(&'ctx LintContext<'s, 'ctx>, Gcx<'hir>, &'hir Hir<'hir>, &'hir Stmt<'hir>) + 'cb,
    expr_f: impl FnMut(&'ctx LintContext<'s, 'ctx>, Gcx<'hir>, &'hir Hir<'hir>, &'hir Expr<'hir>) + 'cb,
) {
    visit_loop_nodes(ctx, gcx, hir, function, false, false, stmt_f, expr_f, |_, _, _, _| {});
}

#[allow(clippy::too_many_arguments)]
fn visit_loop_nodes<'ctx, 's, 'hir, 'cb>(
    ctx: &'ctx LintContext<'s, 'ctx>,
    gcx: Gcx<'hir>,
    _hir: &'hir Hir<'hir>,
    function: FunctionId,
    follow_calls_outside_loop: bool,
    report_local_loops_in_internal_calls: bool,
    mut stmt_f: impl FnMut(&'ctx LintContext<'s, 'ctx>, Gcx<'hir>, &'hir Hir<'hir>, &'hir Stmt<'hir>)
    + 'cb,
    mut expr_f: impl FnMut(&'ctx LintContext<'s, 'ctx>, Gcx<'hir>, &'hir Hir<'hir>, &'hir Expr<'hir>)
    + 'cb,
    mut opaque_f: impl FnMut(&'ctx LintContext<'s, 'ctx>, Gcx<'hir>, &'hir Hir<'hir>, &'hir Expr<'hir>)
    + 'cb,
) {
    let mut visitor = LoopVisitor {
        ctx,
        follow_calls_outside_loop,
        report_local_loops_in_internal_calls,
        stmt_f: &mut stmt_f,
        expr_f: &mut expr_f,
        opaque_f: &mut opaque_f,
    };
    let _ = visit_effective_body_dispatches(gcx, function, &mut visitor);
}

fn is_payable_entry_point(func: &Function<'_>) -> bool {
    !matches!(func.kind, FunctionKind::Constructor | FunctionKind::Modifier)
        && func.state_mutability == StateMutability::Payable
        && matches!(func.visibility, Visibility::Public | Visibility::External)
}

type LoopExprCallback<'ctx, 's, 'hir, 'cb> =
    dyn FnMut(&'ctx LintContext<'s, 'ctx>, Gcx<'hir>, &'hir Hir<'hir>, &'hir Expr<'hir>) + 'cb;
type LoopStmtCallback<'ctx, 's, 'hir, 'cb> =
    dyn FnMut(&'ctx LintContext<'s, 'ctx>, Gcx<'hir>, &'hir Hir<'hir>, &'hir Stmt<'hir>) + 'cb;
type OpaqueCallCallback<'ctx, 's, 'hir, 'cb> =
    dyn FnMut(&'ctx LintContext<'s, 'ctx>, Gcx<'hir>, &'hir Hir<'hir>, &'hir Expr<'hir>) + 'cb;

struct LoopVisitor<'ctx, 's, 'hir, 'cb> {
    ctx: &'ctx LintContext<'s, 'ctx>,
    follow_calls_outside_loop: bool,
    report_local_loops_in_internal_calls: bool,
    stmt_f: &'cb mut LoopStmtCallback<'ctx, 's, 'hir, 'cb>,
    expr_f: &'cb mut LoopExprCallback<'ctx, 's, 'hir, 'cb>,
    opaque_f: &'cb mut OpaqueCallCallback<'ctx, 's, 'hir, 'cb>,
}

impl LoopVisitor<'_, '_, '_, '_> {
    fn reportable(&self, cx: EffectiveBodyCx<'_>) -> bool {
        if self.report_local_loops_in_internal_calls {
            cx.in_loop()
        } else {
            cx.in_enclosing_loop()
        }
    }
}

impl<'hir> EffectiveBodyVisitor<'hir> for LoopVisitor<'_, '_, 'hir, '_> {
    type BreakValue = ();

    fn visit_stmt(
        &mut self,
        cx: EffectiveBodyCx<'hir>,
        stmt: &'hir Stmt<'hir>,
    ) -> ControlFlow<Self::BreakValue> {
        if self.reportable(cx) {
            (self.stmt_f)(self.ctx, cx.gcx(), cx.hir(), stmt);
        }
        ControlFlow::Continue(())
    }

    fn visit_expr(
        &mut self,
        cx: EffectiveBodyCx<'hir>,
        expr: &'hir Expr<'hir>,
    ) -> ControlFlow<Self::BreakValue> {
        if self.reportable(cx) {
            (self.expr_f)(self.ctx, cx.gcx(), cx.hir(), expr);
        }
        ControlFlow::Continue(())
    }

    fn follow_internal_call(
        &mut self,
        cx: EffectiveBodyCx<'hir>,
        _call: &'hir Expr<'hir>,
        _callee: FunctionId,
    ) -> bool {
        self.follow_calls_outside_loop || self.reportable(cx)
    }

    fn visit_opaque_internal_call(
        &mut self,
        cx: EffectiveBodyCx<'hir>,
        call: &'hir Expr<'hir>,
    ) -> ControlFlow<Self::BreakValue> {
        if self.reportable(cx) {
            (self.opaque_f)(self.ctx, cx.gcx(), cx.hir(), call);
        }
        ControlFlow::Continue(())
    }
}
