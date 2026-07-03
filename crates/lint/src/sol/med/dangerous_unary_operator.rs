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
        // `x =- 1` lexes as `x` `=` `-` `1` and parses as a plain assignment of `-1`, identical to
        // the intentional `x = -1`, yet it reads like the compound `x -= 1` it was probably meant
        // to be. Solidity has no `~=` operator either, so `x =~ y` is the same trap.
        // The fused unary can also lead a larger RHS: `x =- a + 1` parses as `x = (-a) + 1`, whose
        // RHS is a `Binary` (or `Ternary`) with the `-a` unary as its leftmost operand. Follow the
        // left spine to that leading unary so those forms are caught too, not only `x =- a`.
        // Because the parsed node matches the legitimate spaced form, only flag when the source
        // fuses `=` to the leading unary (`=-` / `=~`), never `= -`. The gap between the LHS and
        // `rhs.span` (which starts at that unary) holds only whitespace, comments and the `=`
        // token, and no comment can end with `=` (block comments end with `*/`, line comments
        // with a newline), so the gap ends with `=` exactly when the pair is fused. `=+` never
        // reaches here: unary `+` was removed in Solidity 0.5.0, and solar drops it during
        // parsing without producing a node.
        if let ExprKind::Assign(lhs, None, rhs) = &expr.kind
            && leads_with_fusable_unary(rhs)
            && ctx.span_to_snippet(lhs.span.between(rhs.span)).is_some_and(|gap| gap.ends_with('='))
        {
            ctx.emit(&DANGEROUS_UNARY_OPERATOR, expr.span);
        }
    }
}

/// Whether the leftmost operand of `expr` is a `-` or `~` unary, following the left spine of
/// binary and ternary expressions. `rhs.span` begins at that leading unary, so a fused `=-` / `=~`
/// is caught whether the unary is the whole RHS (`x =- a`) or only leads it (`x =- a + 1`).
fn leads_with_fusable_unary(expr: &Expr<'_>) -> bool {
    match &expr.kind {
        ExprKind::Unary(op, _) => matches!(op.kind, UnOpKind::Neg | UnOpKind::BitNot),
        ExprKind::Binary(lhs, _, _) => leads_with_fusable_unary(lhs),
        ExprKind::Ternary(cond, _, _) => leads_with_fusable_unary(cond),
        _ => false,
    }
}
