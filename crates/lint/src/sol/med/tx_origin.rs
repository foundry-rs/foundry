use super::TxOrigin;
use crate::{
    linter::{EarlyLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::{Expr, ExprKind, IndexKind, Stmt, StmtKind},
    interface::SpannedOption,
};

declare_forge_lint!(
    TX_ORIGIN,
    Severity::Med,
    "tx-origin",
    "`tx.origin` should not be used for authorization"
);

impl<'ast> EarlyLintPass<'ast> for TxOrigin {
    fn check_stmt(&mut self, ctx: &LintContext, stmt: &'ast Stmt<'ast>) {
        match &stmt.kind {
            StmtKind::If(cond, ..) | StmtKind::DoWhile(_, cond) => {
                emit_if_contains_tx_origin(ctx, cond);
            }
            StmtKind::While(cond, _) => {
                emit_if_contains_tx_origin(ctx, cond);
            }
            StmtKind::For { cond: Some(cond), .. } => {
                emit_if_contains_tx_origin(ctx, cond);
            }
            _ => {}
        }
    }

    fn check_expr(&mut self, ctx: &LintContext, expr: &'ast Expr<'ast>) {
        if let ExprKind::Call(callee, args) = &expr.kind
            && is_require_or_assert_call(callee)
            && let Some(cond) = args.exprs().next()
        {
            emit_if_contains_tx_origin(ctx, cond);
        }
    }
}

fn emit_if_contains_tx_origin(ctx: &LintContext, expr: &Expr<'_>) {
    if contains_tx_origin(expr) {
        ctx.emit(&TX_ORIGIN, expr.span);
    }
}

fn contains_tx_origin(expr: &Expr<'_>) -> bool {
    if is_tx_origin(expr) {
        return true;
    }
    match &expr.kind {
        ExprKind::Unary(_, inner) => contains_tx_origin(inner),
        ExprKind::Binary(lhs, _, rhs) => contains_tx_origin(lhs) || contains_tx_origin(rhs),
        ExprKind::Index(base, index) => {
            contains_tx_origin(base)
                || match index {
                    IndexKind::Index(Some(index)) => contains_tx_origin(index),
                    IndexKind::Range(start, end) => {
                        start.as_ref().is_some_and(|start| contains_tx_origin(start))
                            || end.as_ref().is_some_and(|end| contains_tx_origin(end))
                    }
                    _ => false,
                }
        }
        ExprKind::Tuple(elems) => elems.iter().any(|elem| {
            if let SpannedOption::Some(inner) = elem.as_ref() {
                contains_tx_origin(inner)
            } else {
                false
            }
        }),
        ExprKind::Call(callee, args) => {
            contains_tx_origin(callee) || args.exprs().any(contains_tx_origin)
        }
        ExprKind::Ternary(cond, then_expr, else_expr) => {
            contains_tx_origin(cond)
                || contains_tx_origin(then_expr)
                || contains_tx_origin(else_expr)
        }
        _ => false,
    }
}

fn is_tx_origin(expr: &Expr<'_>) -> bool {
    matches!(
        &expr.kind,
        ExprKind::Member(base, member)
            if member.as_str() == "origin"
            && matches!(&base.kind, ExprKind::Ident(ident) if ident.as_str() == "tx")
    )
}

fn is_require_or_assert_call(callee: &Expr<'_>) -> bool {
    matches!(
        &callee.kind,
        ExprKind::Ident(ident) if matches!(ident.as_str(), "require" | "assert")
    )
}
