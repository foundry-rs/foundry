use super::TxOrigin;
use crate::{
    linter::{EarlyLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::{Expr, ExprKind, Stmt, StmtKind},
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
                emit_tx_origin_reads(ctx, cond);
            }
            StmtKind::While(cond, _) => {
                emit_tx_origin_reads(ctx, cond);
            }
            StmtKind::For { cond: Some(cond), .. } => {
                emit_tx_origin_reads(ctx, cond);
            }
            _ => {}
        }
    }

    fn check_expr(&mut self, ctx: &LintContext, expr: &'ast Expr<'ast>) {
        if let ExprKind::Call(callee, args) = &expr.kind
            && is_require_or_assert_call(callee)
            && let Some(cond) = args.exprs().next()
        {
            emit_tx_origin_reads(ctx, cond);
        }
    }
}

fn emit_tx_origin_reads(ctx: &LintContext, expr: &Expr<'_>) {
    if is_tx_origin(expr) {
        ctx.emit(&TX_ORIGIN, expr.span);
        return;
    }

    match &expr.kind {
        ExprKind::Array(elems) => {
            for elem in elems.iter() {
                emit_tx_origin_reads(ctx, elem);
            }
        }
        ExprKind::Assign(lhs, _, rhs) | ExprKind::Binary(lhs, _, rhs) => {
            emit_tx_origin_reads(ctx, lhs);
            emit_tx_origin_reads(ctx, rhs);
        }
        ExprKind::Call(callee, args) => {
            emit_tx_origin_reads(ctx, callee);
            for arg in args.exprs() {
                emit_tx_origin_reads(ctx, arg);
            }
        }
        ExprKind::CallOptions(callee, args) => {
            emit_tx_origin_reads(ctx, callee);
            for arg in args.iter() {
                emit_tx_origin_reads(ctx, &arg.value);
            }
        }
        ExprKind::Delete(inner) | ExprKind::Member(inner, _) | ExprKind::Unary(_, inner) => {
            emit_tx_origin_reads(ctx, inner);
        }
        ExprKind::Index(base, kind) => {
            emit_tx_origin_reads(ctx, base);
            match kind {
                solar::ast::IndexKind::Index(Some(index)) => emit_tx_origin_reads(ctx, index),
                solar::ast::IndexKind::Range(start, end) => {
                    if let Some(start) = start {
                        emit_tx_origin_reads(ctx, start);
                    }
                    if let Some(end) = end {
                        emit_tx_origin_reads(ctx, end);
                    }
                }
                _ => {}
            }
        }
        ExprKind::Payable(args) => {
            for arg in args.exprs() {
                emit_tx_origin_reads(ctx, arg);
            }
        }
        ExprKind::Ternary(cond, then_expr, else_expr) => {
            emit_tx_origin_reads(ctx, cond);
            emit_tx_origin_reads(ctx, then_expr);
            emit_tx_origin_reads(ctx, else_expr);
        }
        ExprKind::Tuple(elems) => {
            for elem in elems.iter() {
                if let SpannedOption::Some(elem) = elem.as_ref() {
                    emit_tx_origin_reads(ctx, elem);
                }
            }
        }
        _ => {}
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
