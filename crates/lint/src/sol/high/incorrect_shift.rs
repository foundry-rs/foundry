use super::IncorrectShift;
use crate::{
    linter::{EarlyLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar_ast::{BinOp, BinOpKind, Expr, ExprKind};

declare_forge_lint!(
    INCORRECT_SHIFT,
    Severity::High,
    "incorrect-shift",
    "the order of args in a shift operation is incorrect"
);

impl<'ast> EarlyLintPass<'ast> for IncorrectShift {
    fn check_expr(&mut self, ctx: &LintContext<'_>, expr: &'ast Expr<'ast>) {
        if let ExprKind::Binary(
            left_expr,
            BinOp { kind: BinOpKind::Shl | BinOpKind::Shr, .. },
            right_expr,
        ) = &expr.kind
            && contains_incorrect_shift(left_expr, right_expr)
        {
            ctx.emit(&INCORRECT_SHIFT, expr.span);
        }
    }
}

// TODO: come up with a better heuristic. Treat initial impl as a PoC.
// Checks if the left operand is a literal and the right operand is not, indicating a potential
// reversed shift operation.
fn contains_incorrect_shift<'ast>(
    left_expr: &'ast Expr<'ast>,
    right_expr: &'ast Expr<'ast>,
) -> bool {
    let is_left_literal = matches!(left_expr.kind, ExprKind::Lit(..));
    let is_right_not_literal = !matches!(right_expr.kind, ExprKind::Lit(..));

    is_left_literal && is_right_not_literal
}
