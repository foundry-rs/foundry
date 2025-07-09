use super::UnwrappedModifierLogic;
use crate::{
    linter::{EarlyLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar_ast::{ExprKind, ItemFunction, Stmt, StmtKind};

declare_forge_lint!(
    UNWRAPPED_MODIFIER_LOGIC,
    Severity::Gas,
    "unwrapped-modifier-logic",
    "modifier logic should be wrapped to avoid code duplication and reduce codesize"
);

impl<'ast> EarlyLintPass<'ast> for UnwrappedModifierLogic {
    fn check_item_function(&mut self, ctx: &LintContext<'_>, func: &'ast ItemFunction<'ast>) {
        // If not a modifier, skip.
        if !func.kind.is_modifier() {
            return;
        }

        // If modifier has no contents, skip.
        let body = match &func.body {
            Some(body) => body,
            _ => return,
        };

        // If body contains unwrapped logic, emit.
        if body.iter().any(|stmt| !is_valid_stmt(stmt))
            && let Some(name) = func.header.name
        {
            ctx.emit(&UNWRAPPED_MODIFIER_LOGIC, name.span);
        }
    }
}

fn is_valid_stmt(stmt: &Stmt<'_>) -> bool {
    match &stmt.kind {
        // If the statement is an expression, emit if not valid.
        StmtKind::Expr(expr) => is_valid_expr(expr),

        // If the statement is a placeholder, skip.
        StmtKind::Placeholder => true,

        // Disallow all other statements.
        _ => false,
    }
}

fn is_valid_expr(expr: &solar_ast::Expr<'_>) -> bool {
    match &expr.kind {
        // If the expression is a function call...
        ExprKind::Call(func_expr, _) => match &func_expr.kind {
            // If the expression is a built-in control flow function, emit.
            ExprKind::Ident(ident) => !matches!(ident.name.as_str(), "require" | "assert"),

            // If the expression is a member call, emit.
            ExprKind::Member(_, _) => false, // TODO: enable library calls

            // Disallow all other expressions.
            _ => false,
        },

        // Disallow all other expressions.
        _ => false,
    }
}
