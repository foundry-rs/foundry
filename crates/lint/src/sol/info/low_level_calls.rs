use super::LowLevelCalls;
use crate::{
    linter::{EarlyLintPass, LintContext},
    sol::{Severity, SolLint, analysis::is_low_level_call},
};
use solar::ast::Expr;

declare_forge_lint!(
    LOW_LEVEL_CALLS,
    Severity::Info,
    "low-level-calls",
    "Low-level calls should be avoided"
);

impl<'ast> EarlyLintPass<'ast> for LowLevelCalls {
    fn check_expr(&mut self, ctx: &LintContext, expr: &'ast Expr<'ast>) {
        if is_low_level_call(expr) {
            ctx.emit(&LOW_LEVEL_CALLS, expr.span);
        }
    }
}
