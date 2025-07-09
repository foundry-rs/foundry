use super::ModifierLogic;
use crate::{
    linter::{EarlyLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar_ast::{ExprKind, FunctionKind, ItemFunction, Stmt, StmtKind};

declare_forge_lint!(
    UNWRAPPED_MODIFIER_LOGIC,
    Severity::Gas,
    "unwrapped-modifier-logic",
    "modifier logic should be wrapped to avoid code duplication and reduce codesize"
);

impl<'ast> EarlyLintPass<'ast> for ModifierLogic {
    fn check_item_function(&mut self, ctx: &LintContext<'_>, func: &'ast ItemFunction<'ast>) {
        // Only check modifiers
        if func.kind != FunctionKind::Modifier {
            return;
        }

        // Skip if modifier has no body or the body is empty
        let body = match &func.body {
            Some(body) if !body.is_empty() => body,
            _ => return,
        };

        // Emit lint if the modifier contains unwrapped logic
        if contains_unwrapped_logic(body)
            && let Some(name) = func.header.name
        {
            ctx.emit(&UNWRAPPED_MODIFIER_LOGIC, name.span);
        }
    }
}

/// Returns true if the modifier body contains any logic other than:
/// - The placeholder `_;`
/// - Calls to internal/private/public functions via direct identifier
fn contains_unwrapped_logic(stmts: &[Stmt<'_>]) -> bool {
    stmts.iter().any(|stmt| !is_permitted_statement(stmt))
}

/// Returns true if the statement is allowed in a modifier without triggering the lint
fn is_permitted_statement(stmt: &Stmt<'_>) -> bool {
    match &stmt.kind {
        StmtKind::Placeholder => true,
        StmtKind::Expr(expr) => is_permitted_expression(expr),
        _ => false,
    }
}

/// Returns true if the expression is a call to a function via direct identifier
/// (i.e., not a built-in like require/assert/revert, and not a member/external call)
fn is_permitted_expression(expr: &solar_ast::Expr<'_>) -> bool {
    match &expr.kind {
        // Only allow function calls.
        ExprKind::Call(func_expr, _) => match &func_expr.kind {
            // Allow direct calls to user-defined functions (by identifier)
            ExprKind::Ident(ident) => {
                // Disallow calls to built-in control flow functions like require/assert/revert
                !matches!(ident.name.as_str(), "require" | "assert" | "revert")
            }

            // Disallow member calls (e.g., object.method()), which could be external or library
            // calls
            // TODO: enable library calls
            ExprKind::Member(_, _) => false,

            // Disallow all other forms of function expressions (e.g., function pointers, etc.)
            _ => false,
        },

        // Disallow all other expression types (not a function call)
        _ => false,
    }
}
