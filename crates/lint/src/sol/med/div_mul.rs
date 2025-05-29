use super::DivideBeforeMultiply;
use crate::{
    declare_forge_lint,
    linter::{EarlyLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar_ast::{BinOp, BinOpKind, Expr, ExprKind};

declare_forge_lint!(
    DIVIDE_BEFORE_MULTIPLY,
    Severity::Med,
    "divide-before-multiply",
    "multiplication should occur before division to avoid loss of precision"
);

impl<'ast> EarlyLintPass<'ast> for DivideBeforeMultiply {
    fn check_expr(&mut self, ctx: &LintContext<'_>, expr: &'ast Expr<'ast>) {
        if let ExprKind::Binary(left_expr, BinOp { kind: BinOpKind::Mul, .. }, _) = &expr.kind {
            if contains_division(left_expr) {
                ctx.emit(&DIVIDE_BEFORE_MULTIPLY, expr.span);
            }
        }
    }
}

fn contains_division<'ast>(expr: &'ast Expr<'ast>) -> bool {
    match &expr.kind {
        ExprKind::Binary(_, BinOp { kind: BinOpKind::Div, .. }, _) => true,
        ExprKind::Tuple(inner_exprs) => inner_exprs.iter().any(|opt_expr| {
            if let Some(inner_expr) = opt_expr {
                contains_division(inner_expr)
            } else {
                false
            }
        }),
        _ => false,
    }
}
