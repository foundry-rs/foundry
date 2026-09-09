use super::TxOrigin;
use crate::{
    linter::{EarlyLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::{Expr, ExprKind, Stmt, StmtKind, visit::Visit},
    interface::{Span, kw, sym},
};
use std::ops::ControlFlow;

declare_forge_lint!(TX_ORIGIN, Severity::Med, "tx-origin");

impl<'ast> EarlyLintPass<'ast> for TxOrigin {
    fn check_stmt(&mut self, ctx: &LintContext, stmt: &'ast Stmt<'ast>) {
        if let StmtKind::If(cond, ..)
        | StmtKind::DoWhile(_, cond)
        | StmtKind::While(cond, _)
        | StmtKind::For { cond: Some(cond), .. } = &stmt.kind
        {
            emit_if_contains_tx_origin(ctx, cond);
        }
    }

    fn check_expr(&mut self, ctx: &LintContext, expr: &'ast Expr<'ast>) {
        if let ExprKind::Call(callee, args) = &expr.kind
            && matches!(&callee.kind, ExprKind::Ident(id) if matches!(id.name, sym::require | sym::assert))
            && let Some(cond) = args.exprs().next()
        {
            emit_if_contains_tx_origin(ctx, cond);
        }
    }
}

fn emit_if_contains_tx_origin<'ast>(ctx: &LintContext, expr: &'ast Expr<'ast>) {
    if let ControlFlow::Break(span) = TxOriginFinder.visit_expr(expr) {
        ctx.span_lint(&TX_ORIGIN, span, |diag| {
            diag.primary_message("`tx.origin` is used for authorization");
        });
    }
}

struct TxOriginFinder;

impl<'ast> Visit<'ast> for TxOriginFinder {
    type BreakValue = Span;

    fn visit_expr(&mut self, expr: &'ast Expr<'ast>) -> ControlFlow<Self::BreakValue> {
        if let ExprKind::Member(base, member) = &expr.kind
            && member.name == kw::Origin
            && matches!(&base.kind, ExprKind::Ident(id) if id.name == sym::tx)
        {
            return ControlFlow::Break(expr.span);
        }
        self.walk_expr(expr)
    }
}
