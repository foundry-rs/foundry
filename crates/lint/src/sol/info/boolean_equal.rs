use super::BooleanEqual;
use crate::{
    linter::{EarlyLintPass, LintContext, Suggestion},
    sol::{Severity, SolLint},
};
use solar::{
    ast::{BinOp, BinOpKind, Expr, ExprKind, LitKind},
    interface::diagnostics::Applicability,
};

declare_forge_lint!(
    BOOLEAN_EQUAL,
    Severity::Info,
    "boolean-equal",
    "boolean comparisons to constants should be simplified"
);

impl<'ast> EarlyLintPass<'ast> for BooleanEqual {
    fn check_expr(&mut self, ctx: &LintContext, expr: &'ast Expr<'ast>) {
        if let ExprKind::Binary(
            left,
            op @ BinOp { kind: BinOpKind::Eq | BinOpKind::Ne, .. },
            right,
        ) = &expr.kind
        {
            match bool_comparison_suggestion(ctx, left, op.kind, right) {
                BoolComparison::WithSuggestion(simplified) => {
                    ctx.emit_with_suggestion(
                        &BOOLEAN_EQUAL,
                        expr.span,
                        Suggestion::fix(simplified, Applicability::MachineApplicable)
                            .with_desc("consider simplifying to"),
                    );
                }
                BoolComparison::WithoutSuggestion => ctx.emit(&BOOLEAN_EQUAL, expr.span),
                BoolComparison::None => {}
            }
        }
    }
}

enum BoolComparison {
    WithSuggestion(String),
    WithoutSuggestion,
    None,
}

fn bool_comparison_suggestion(
    ctx: &LintContext,
    left: &Expr<'_>,
    op: BinOpKind,
    right: &Expr<'_>,
) -> BoolComparison {
    let left_bool = bool_literal(left);
    let right_bool = bool_literal(right);

    match (left_bool, right_bool) {
        (Some(value), None) => simplify_expr(ctx, right, op, value),
        (None, Some(value)) => simplify_expr(ctx, left, op, value),
        (Some(_), Some(_)) => BoolComparison::WithoutSuggestion,
        (None, None) => BoolComparison::None,
    }
}

fn bool_literal(expr: &Expr<'_>) -> Option<bool> {
    let expr = expr.peel_parens();
    if let ExprKind::Lit(lit, _) = &expr.kind
        && let LitKind::Bool(value) = lit.kind
    {
        Some(value)
    } else {
        None
    }
}

fn simplify_expr(
    ctx: &LintContext,
    expr: &Expr<'_>,
    op: BinOpKind,
    constant: bool,
) -> BoolComparison {
    let Some(snippet) = ctx.span_to_snippet(expr.span) else {
        return BoolComparison::WithoutSuggestion;
    };

    let simplified = match (op, constant) {
        (BinOpKind::Eq, true) | (BinOpKind::Ne, false) => snippet,
        (BinOpKind::Eq, false) | (BinOpKind::Ne, true) if can_negate_without_parens(expr) => {
            format!("!{snippet}")
        }
        (BinOpKind::Eq, false) | (BinOpKind::Ne, true) => format!("!({snippet})"),
        _ => return BoolComparison::None,
    };

    BoolComparison::WithSuggestion(simplified)
}

fn can_negate_without_parens(expr: &Expr<'_>) -> bool {
    matches!(
        expr.peel_parens().kind,
        ExprKind::Call(..)
            | ExprKind::CallOptions(..)
            | ExprKind::Ident(_)
            | ExprKind::Index(..)
            | ExprKind::Lit(..)
            | ExprKind::Member(..)
    )
}
