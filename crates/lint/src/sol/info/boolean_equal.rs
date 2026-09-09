use super::BooleanEqual;
use crate::{
    linter::{EarlyLintPass, LintContext},
    sol::{Severity, SolLint, analysis::ast_bool_literal},
};
use solar::{
    ast::{BinOpKind, Expr, ExprKind},
    interface::diagnostics::Applicability,
};

declare_forge_lint!(BOOLEAN_EQUAL, Severity::Info, "boolean-equal");

impl<'ast> EarlyLintPass<'ast> for BooleanEqual {
    fn check_expr(&mut self, ctx: &LintContext, expr: &'ast Expr<'ast>) {
        let ExprKind::Binary(left, op, right) = &expr.kind else { return };
        if !matches!(op.kind, BinOpKind::Eq | BinOpKind::Ne) {
            return;
        }
        let simplified = match (ast_bool_literal(left), ast_bool_literal(right)) {
            (None, None) => return,
            (Some(_), Some(_)) => None,
            (Some(constant), None) => simplify(ctx, right, op.kind, constant),
            (None, Some(constant)) => simplify(ctx, left, op.kind, constant),
        };
        match simplified {
            Some(simplified) => ctx.span_lint(&BOOLEAN_EQUAL, expr.span, |diag| {
                diag.primary_message("boolean comparison to a constant is redundant");
                diag.span_suggestion(
                    expr.span,
                    "consider simplifying to",
                    simplified,
                    Applicability::MachineApplicable,
                );
            }),
            None => ctx.span_lint(&BOOLEAN_EQUAL, expr.span, |diag| {
                diag.primary_message("boolean comparison to a constant is redundant");
            }),
        }
    }
}

/// `x == true` / `x != false` simplify to `x`, the other two forms to `!x`.
fn simplify(ctx: &LintContext, expr: &Expr<'_>, op: BinOpKind, constant: bool) -> Option<String> {
    let snippet = ctx.span_to_snippet(expr.span)?;
    let negate = (op == BinOpKind::Eq) != constant;
    let atomic = matches!(
        expr.peel_parens().kind,
        ExprKind::Call(..)
            | ExprKind::CallOptions(..)
            | ExprKind::Ident(_)
            | ExprKind::Index(..)
            | ExprKind::Lit(..)
            | ExprKind::Member(..)
    );
    Some(match (negate, atomic) {
        (false, _) => snippet,
        (true, true) => format!("!{snippet}"),
        (true, false) => format!("!({snippet})"),
    })
}
