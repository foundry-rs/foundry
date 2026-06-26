use super::BooleanCst;
use crate::{
    linter::{EarlyLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::{BinOp, BinOpKind, Expr, ExprKind, LitKind, Stmt, StmtKind, VariableDefinition},
    interface::SpannedOption,
};

declare_forge_lint!(BOOLEAN_CST, Severity::Med, "boolean-cst", "misuse of a boolean constant");

impl<'ast> EarlyLintPass<'ast> for BooleanCst {
    fn check_stmt(&mut self, ctx: &LintContext, stmt: &'ast Stmt<'ast>) {
        match &stmt.kind {
            StmtKind::If(cond, ..) | StmtKind::DoWhile(_, cond) => {
                check_expr(ctx, cond, ExprContext::Condition { allow_bare_true: false });
            }
            StmtKind::While(cond, _) => {
                check_expr(ctx, cond, ExprContext::Condition { allow_bare_true: true });
            }
            StmtKind::For { cond: Some(cond), .. } => {
                check_expr(ctx, cond, ExprContext::Condition { allow_bare_true: false });
            }
            StmtKind::DeclMulti(_, expr) => check_allowed_bare_expr(ctx, expr),
            StmtKind::Expr(expr) | StmtKind::Return(Some(expr)) => {
                check_allowed_bare_expr(ctx, expr);
            }
            _ => {}
        }
    }

    fn check_variable_definition(
        &mut self,
        ctx: &LintContext,
        var: &'ast VariableDefinition<'ast>,
    ) {
        if let Some(initializer) = &var.initializer {
            check_allowed_bare_expr(ctx, initializer);
        }
    }
}

#[derive(Clone, Copy)]
enum ExprContext {
    Condition { allow_bare_true: bool },
    General,
    AllowedBare,
}

fn check_allowed_bare_expr(ctx: &LintContext, expr: &Expr<'_>) {
    let context =
        if bool_literal(expr).is_some() { ExprContext::AllowedBare } else { ExprContext::General };
    check_expr(ctx, expr, context);
}

fn check_expr(ctx: &LintContext, expr: &Expr<'_>, context: ExprContext) {
    if let Some(value) = bool_literal(expr) {
        match context {
            ExprContext::AllowedBare => {}
            ExprContext::Condition { allow_bare_true: true } if value => {}
            ExprContext::Condition { .. } | ExprContext::General => {
                ctx.emit(&BOOLEAN_CST, expr.span);
            }
        }
        return;
    }

    match &expr.kind {
        ExprKind::Assign(_, _, rhs) => check_allowed_bare_expr(ctx, rhs),
        ExprKind::Binary(left, op, right) => check_binary_expr(ctx, left, *op, right),
        ExprKind::Call(_, args) => {
            for arg in args.exprs() {
                check_allowed_bare_expr(ctx, arg);
            }
        }
        ExprKind::Delete(expr) | ExprKind::Unary(_, expr) => {
            check_expr(ctx, expr, ExprContext::General);
        }
        ExprKind::Ternary(cond, true_expr, false_expr) => {
            check_expr(ctx, cond, ExprContext::Condition { allow_bare_true: false });
            check_expr(ctx, true_expr, ExprContext::General);
            check_expr(ctx, false_expr, ExprContext::General);
        }
        ExprKind::Tuple(exprs) => {
            for opt_expr in exprs.iter() {
                if let SpannedOption::Some(expr) = opt_expr.as_ref() {
                    check_expr(ctx, expr, ExprContext::General);
                }
            }
        }
        _ => {}
    }
}

fn check_binary_expr(ctx: &LintContext, left: &Expr<'_>, op: BinOp, right: &Expr<'_>) {
    if matches!(op.kind, BinOpKind::Eq | BinOpKind::Ne)
        && (bool_literal(left).is_some() || bool_literal(right).is_some())
    {
        return;
    }

    check_expr(ctx, left, ExprContext::General);
    check_expr(ctx, right, ExprContext::General);
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
