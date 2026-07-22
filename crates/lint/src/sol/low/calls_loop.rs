use super::CallsLoop;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::sema::{
    Gcx,
    hir::{
        EffectiveBodyCx, EffectiveBodyVisitor, Expr, FunctionId, Hir,
        visit_effective_body_dispatches,
    },
};
use std::{collections::HashSet, ops::ControlFlow};

declare_forge_lint!(CALLS_LOOP, Severity::Low, "calls-loop", "external call inside a loop");

impl<'hir> LateLintPass<'hir> for CallsLoop {
    fn check_nested_function(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'hir>,
        _hir: &'hir Hir<'hir>,
        function: FunctionId,
    ) {
        let mut visitor = CallsLoopVisitor { ctx, emitted: HashSet::new() };
        let _ = visit_effective_body_dispatches(gcx, function, &mut visitor);
    }
}

struct CallsLoopVisitor<'ctx, 's, 'c> {
    ctx: &'ctx LintContext<'s, 'c>,
    emitted: HashSet<solar::interface::Span>,
}

impl<'hir> EffectiveBodyVisitor<'hir> for CallsLoopVisitor<'_, '_, '_> {
    type BreakValue = ();

    fn visit_expr_post(
        &mut self,
        cx: EffectiveBodyCx<'hir>,
        expr: &'hir Expr<'hir>,
    ) -> ControlFlow<Self::BreakValue> {
        if cx.in_loop()
            && cx.call_info(expr).is_some_and(|info| info.is_external_interaction())
            && self.emitted.insert(expr.span)
        {
            self.ctx.emit(&CALLS_LOOP, expr.span);
        }
        ControlFlow::Continue(())
    }

    fn follow_internal_call(
        &mut self,
        cx: EffectiveBodyCx<'hir>,
        _call: &'hir Expr<'hir>,
        _callee: FunctionId,
    ) -> bool {
        cx.in_loop()
    }

    fn visit_opaque_internal_call(
        &mut self,
        cx: EffectiveBodyCx<'hir>,
        call: &'hir Expr<'hir>,
    ) -> ControlFlow<Self::BreakValue> {
        if cx.in_loop() && self.emitted.insert(call.span) {
            self.ctx.emit(&CALLS_LOOP, call.span);
        }
        ControlFlow::Continue(())
    }
}
