use super::CustomErrors;
use crate::{
    linter::{EarlyLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::ast::{CallArgsKind, Expr, ExprKind};

declare_forge_lint!(
    CUSTOM_ERRORS,
    Severity::Gas,
    "custom-errors",
    "prefer using custom errors on revert and require calls"
);

impl<'ast> EarlyLintPass<'ast> for CustomErrors {
    fn check_expr(&mut self, ctx: &LintContext, expr: &'ast Expr<'ast>) {
        if let ExprKind::Call(callee, args) = &expr.kind
            && ((is_require_call(callee) && should_lint_require(args))
                || (is_revert_call(callee) && should_lint_revert(args)))
        {
            ctx.emit(&CUSTOM_ERRORS, expr.span);
        }
    }
}

/// Checks if an expression is a call to the `require` builtin function.
fn is_require_call(callee: &Expr<'_>) -> bool {
    matches!(&callee.kind, ExprKind::Ident(ident) if ident.as_str() == "require")
}

/// Checks if an expression is a call to the `revert` builtin function.
fn is_revert_call(callee: &Expr<'_>) -> bool {
    matches!(&callee.kind, ExprKind::Ident(ident) if ident.as_str() == "revert")
}

/// Checks if a revert call should be linted: `revert()` or `revert("message")`.
fn should_lint_revert(args: &solar::ast::CallArgs<'_>) -> bool {
    matches!(&args.kind, CallArgsKind::Unnamed(arg_exprs) if {
        arg_exprs.is_empty() || arg_exprs.first().is_some_and(|e| is_string_literal(e))
    })
}

/// Checks if a require call should be linted: has string literal as second argument.
fn should_lint_require(args: &solar::ast::CallArgs<'_>) -> bool {
    matches!(&args.kind, CallArgsKind::Unnamed(arg_exprs) if {
        arg_exprs.get(1).is_some_and(|e| is_string_literal(e))
    })
}

/// Checks if an expression is a string literal.
fn is_string_literal(expr: &Expr<'_>) -> bool {
    matches!(&expr.kind, ExprKind::Lit(lit, _) if matches!(lit.kind, solar::ast::LitKind::Str(..)))
}
