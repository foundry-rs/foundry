use super::LowLevelCalls;
use crate::{
    linter::{EarlyLintPass, LintContext},
    sol::{Severity, SolLint, calls::is_low_level_call},
};
use solar::ast::{Expr, ItemFunction, visit::Visit};
use std::ops::ControlFlow;

declare_forge_lint!(
    LOW_LEVEL_CALLS,
    Severity::Info,
    "low-level-calls",
    "Low-level calls should be avoided"
);

impl<'ast> EarlyLintPass<'ast> for LowLevelCalls {
    fn check_item_function(&mut self, ctx: &LintContext, func: &'ast ItemFunction<'ast>) {
        if let Some(body) = &func.body {
            let mut checker = LowLevelCallsChecker { ctx };
            let _ = checker.visit_block(body);
        }
    }
}

struct LowLevelCallsChecker<'a, 's> {
    ctx: &'a LintContext<'s, 'a>,
}

impl<'ast> Visit<'ast> for LowLevelCallsChecker<'_, '_> {
    type BreakValue = ();

    fn visit_expr(&mut self, expr: &'ast Expr<'ast>) -> ControlFlow<Self::BreakValue> {
        if is_low_level_call(expr) {
            self.ctx.emit(&LOW_LEVEL_CALLS, expr.span);
        }
        self.walk_expr(expr)
    }
}
