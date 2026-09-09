use super::IncorrectShift;
use crate::{
    linter::{EarlyLintPass, LintContext},
    sol::{Severity, SolLint},
};
use alloy_primitives::U256;
use solar::{
    ast::{LitKind, Stmt, StmtKind, visit::Visit, yul},
    data_structures::Never,
    interface::kw,
};
use std::ops::ControlFlow;

declare_forge_lint!(INCORRECT_SHIFT, Severity::High, "incorrect-shift");

impl<'ast> EarlyLintPass<'ast> for IncorrectShift {
    fn check_stmt(&mut self, ctx: &LintContext, stmt: &'ast Stmt<'ast>) {
        if let StmtKind::Assembly(assembly) = &stmt.kind {
            let _ = ShiftChecker { ctx }.visit_yul_block(&assembly.block);
        }
    }
}

struct ShiftChecker<'a, 's> {
    ctx: &'a LintContext<'s, 'a>,
}

impl<'ast> Visit<'ast> for ShiftChecker<'_, '_> {
    type BreakValue = Never;

    fn visit_yul_expr(&mut self, expr: &'ast yul::Expr<'ast>) -> ControlFlow<Self::BreakValue> {
        // A computed shift of a literal suggests swapped arguments, except `shl(n, 1)`,
        // which constructs a single-bit mask.
        if let yul::ExprKind::Call(call) = &expr.kind
            && matches!(call.name.name, kw::Shl | kw::Shr | kw::Sar)
            && let [left, right] = call.arguments.as_ref()
            && !matches!(left.kind, yul::ExprKind::Lit(_))
            && let yul::ExprKind::Lit(lit) = &right.kind
            && !(call.name.name == kw::Shl
                && matches!(lit.kind, LitKind::Number(value) if value == U256::ONE))
        {
            self.ctx.span_lint(&INCORRECT_SHIFT, expr.span, |diag| {
                diag.primary_message("the order of args in a shift operation is incorrect");
            });
        }
        self.walk_yul_expr(expr)
    }
}
