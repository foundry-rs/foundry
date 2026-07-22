use super::{DelegatecallLoop, payable_loop::visit_payable_loop_expressions};
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::sema::{
    Gcx,
    hir::{FunctionId, Hir},
    ty::CallKind,
};
use std::{cell::RefCell, collections::HashSet};

declare_forge_lint!(
    DELEGATECALL_LOOP,
    Severity::Low,
    "delegatecall-loop",
    "payable functions should not use `delegatecall` inside a loop"
);

impl<'hir> LateLintPass<'hir> for DelegatecallLoop {
    fn check_nested_function(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'hir>,
        hir: &'hir Hir<'hir>,
        function: FunctionId,
    ) {
        let emitted = RefCell::new(HashSet::new());
        visit_payable_loop_expressions(
            ctx,
            gcx,
            hir,
            function,
            |ctx, gcx, _, expr| {
                if gcx.call_info(expr).is_some_and(|info| info.kind() == CallKind::DelegateCall)
                    && emitted.borrow_mut().insert(expr.span)
                {
                    ctx.emit(&DELEGATECALL_LOOP, expr.span);
                }
            },
            |ctx, _, _, call| {
                if emitted.borrow_mut().insert(call.span) {
                    ctx.emit(&DELEGATECALL_LOOP, call.span);
                }
            },
        );
    }
}
