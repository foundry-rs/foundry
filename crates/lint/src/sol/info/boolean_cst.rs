use super::BooleanCst;
use crate::{
    linter::{EarlyLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::ast::{BinOpKind, Expr, ExprKind, Lit, LitKind, Stmt, StmtKind, VariableDefinition};

declare_forge_lint!(BOOLEAN_CST, Severity::Med, "boolean-cst", "misuse of a boolean constant");

impl<'ast> EarlyLintPass<'ast> for BooleanCst {
    fn check_stmt(&mut self, ctx: &LintContext, stmt: &'ast Stmt<'ast>) {
        match &stmt.kind {
            StmtKind::If(cond, ..) | StmtKind::DoWhile(_, cond) | StmtKind::For { cond: Some(cond), .. } => {
                check_expr(ctx, cond, false);
            }
            // `while (true)` is the idiomatic infinite loop.
            StmtKind::While(cond, _) => check_expr(ctx, cond, bool_literal(cond) == Some(true)),
            StmtKind::DeclMulti(_, expr) | StmtKind::Expr(expr) | StmtKind::Return(Some(expr)) => {
                check_expr(ctx, expr, true);
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
            check_expr(ctx, initializer, true);
        }
    }
}

/// Reports boolean literals in `expr` that are not `allow_bare` at the top level: a literal
/// stored, returned or passed as an argument is fine, one combined into a larger expression or
/// used as a condition is a misuse.
fn check_expr(ctx: &LintContext, expr: &Expr<'_>, allow_bare: bool) {
    if bool_literal(expr).is_some() {
        if !allow_bare {
            ctx.emit(&BOOLEAN_CST, expr.span);
        }
        return;
    }
    match &expr.kind {
        ExprKind::Assign(_, _, rhs) => check_expr(ctx, rhs, true),
        // `x == true` is boolean-equal's business.
        ExprKind::Binary(left, op, right)
            if !(matches!(op.kind, BinOpKind::Eq | BinOpKind::Ne)
                && (bool_literal(left).is_some() || bool_literal(right).is_some())) =>
        {
            check_expr(ctx, left, false);
            check_expr(ctx, right, false);
        }
        ExprKind::Call(_, args) => args.exprs().for_each(|arg| check_expr(ctx, arg, true)),
        ExprKind::Delete(expr) | ExprKind::Unary(_, expr) => check_expr(ctx, expr, false),
        ExprKind::Ternary(cond, true_expr, false_expr) => {
            check_expr(ctx, cond, false);
            check_expr(ctx, true_expr, false);
            check_expr(ctx, false_expr, false);
        }
        ExprKind::Tuple(exprs) => exprs
            .iter()
            .filter_map(|expr| Option::from(expr.as_deref()))
            .for_each(|expr| check_expr(ctx, expr, false)),
        _ => {}
    }
}

fn bool_literal(expr: &Expr<'_>) -> Option<bool> {
    match &expr.peel_parens().kind {
        ExprKind::Lit(Lit { kind: LitKind::Bool(value), .. }, _) => Some(*value),
        _ => None,
    }
}
