use super::IncorrectShift;
use crate::{
    linter::{EarlyLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::{Stmt, StmtKind, visit::Visit, yul},
    data_structures::Never,
    interface::kw,
};
use std::ops::ControlFlow;

declare_forge_lint!(
    INCORRECT_SHIFT,
    Severity::High,
    "incorrect-shift",
    "the order of args in a shift operation is incorrect"
);

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
        // `shl(x, 2)`: the shift amount comes first in Yul, so a literal in the value position
        // and a computed amount betray swapped arguments.
        if let yul::ExprKind::Call(call) = &expr.kind
            && matches!(call.name.name, kw::Shl | kw::Shr | kw::Sar)
            && let [left, right] = call.arguments.as_ref()
            && !matches!(left.kind, yul::ExprKind::Lit(_))
            && matches!(right.kind, yul::ExprKind::Lit(_))
        {
            self.ctx.emit(&INCORRECT_SHIFT, expr.span);
        }
        self.walk_yul_expr(expr)
    }
}
