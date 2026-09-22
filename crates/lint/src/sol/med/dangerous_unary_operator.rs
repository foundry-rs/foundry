use super::DangerousUnaryOperator;
use crate::{
    linter::{EarlyLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::ast::{Expr, ExprKind, UnOpKind};

declare_forge_lint!(
    DANGEROUS_UNARY_OPERATOR,
    Severity::Med,
    "dangerous-unary-operator",
    "unary operator fused to `=`: `x =- 1` parses as `x = -1`, not `x -= 1`"
);

impl<'ast> EarlyLintPass<'ast> for DangerousUnaryOperator {
    fn check_expr(&mut self, ctx: &LintContext, expr: &'ast Expr<'ast>) {
        // `x =- 1` parses exactly like the intentional `x = -1`, so the AST cannot tell them
        // apart: only flag when the source fuses `=` to the unary. The gap between the LHS and
        // `rhs.span` (which starts at the leading unary) holds only whitespace, comments and the
        // `=` token, and no comment can end with `=`, so the gap ends with `=` exactly when the
        // pair is fused. Solidity has no `~=` either, so `=~` is the same trap; unary `+` was
        // removed in 0.5.0 and never produces a node.
        if let ExprKind::Assign(lhs, None, rhs) = &expr.kind
            && leads_with_fusable_unary(rhs)
            && ctx.span_to_snippet(lhs.span.between(rhs.span)).is_some_and(|gap| gap.ends_with('='))
        {
            ctx.emit(&DANGEROUS_UNARY_OPERATOR, expr.span);
        }
    }
}

/// Whether the leftmost operand of `expr` is a `-` or `~` unary, following the left spine of
/// binary and ternary expressions so `x =- a + 1` (`x = (-a) + 1`) is caught as well.
fn leads_with_fusable_unary(expr: &Expr<'_>) -> bool {
    match &expr.kind {
        ExprKind::Unary(op, _) => matches!(op.kind, UnOpKind::Neg | UnOpKind::BitNot),
        ExprKind::Binary(lhs, _, _) | ExprKind::Ternary(lhs, _, _) => leads_with_fusable_unary(lhs),
        _ => false,
    }
}
