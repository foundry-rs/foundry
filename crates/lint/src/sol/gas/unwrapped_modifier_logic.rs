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
        let Some(body) = &func.body else { return };

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

// TODO: Support library member calls like `Lib.foo` (throws false positives).
fn is_valid_expr(expr: &solar_ast::Expr<'_>) -> bool {
    // If the expression is a call, continue.
    if let ExprKind::Call(func_expr, _) = &expr.kind
        && let ExprKind::Ident(ident) = &func_expr.kind
    {
        // If the call is a built-in control flow function, emit.
        return !matches!(ident.name.as_str(), "require" | "assert");
    }

    // Disallow all other expressions.
    false
}
