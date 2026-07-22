use super::{MsgValueLoop, payable_loop::visit_payable_loop_expressions};
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::sema::{
    Gcx,
    builtins::Builtin,
    hir::{FunctionId, Hir},
};
use std::{cell::RefCell, collections::HashSet};

declare_forge_lint!(
    MSG_VALUE_LOOP,
    Severity::Low,
    "msg-value-loop",
    "payable functions should not use `msg.value` inside a loop"
);

impl<'hir> LateLintPass<'hir> for MsgValueLoop {
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
                if gcx.builtin_member(expr.id) == Some(Builtin::MsgValue)
                    && emitted.borrow_mut().insert(expr.span)
                {
                    ctx.emit(&MSG_VALUE_LOOP, expr.span);
                }
            },
            |ctx, _, _, call| {
                if emitted.borrow_mut().insert(call.span) {
                    ctx.emit(&MSG_VALUE_LOOP, call.span);
                }
            },
        );
    }
}
