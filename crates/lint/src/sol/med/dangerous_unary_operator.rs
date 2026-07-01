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
        // Because the parsed node matches the legitimate spaced form, only flag when the
        // source fuses `=` to the unary operator (`=-` / `=~`), never `= -`. `=+` never
        // reaches here: unary `+` was removed in Solidity 0.5.0, and solar drops it during
        // parsing without producing a node.
        if let ExprKind::Assign(lhs, None, rhs) = &expr.kind
            && let ExprKind::Unary(op, _) = &rhs.kind
            && matches!(op.kind, UnOpKind::Neg | UnOpKind::BitNot)
            && ctx
                .span_to_snippet(lhs.span.between(op.span))
                .is_some_and(|gap| gap.trim_start() == "=")
        {
            ctx.emit(&DANGEROUS_UNARY_OPERATOR, expr.span);
        }
    }
}
