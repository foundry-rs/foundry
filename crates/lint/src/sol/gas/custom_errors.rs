use super::CustomErrors;
use crate::{
    linter::{EarlyLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::ast::{CallArgsKind, Expr, ExprKind, ItemFunction, Stmt, StmtKind, visit::Visit};
use std::ops::ControlFlow;

declare_forge_lint!(
    CUSTOM_ERRORS,
    Severity::Gas,
    "custom-errors",
    "prefer using custom errors on revert and require calls"
);

impl<'ast> EarlyLintPass<'ast> for CustomErrors {
    fn check_item_function(&mut self, ctx: &LintContext, func: &'ast ItemFunction<'ast>) {
        if let Some(body) = &func.body {
            let mut checker = CustomErrorsChecker { ctx };
            let _ = checker.visit_block(body);
        }
    }
}

/// Visitor that detects require/revert statements with string messages.
struct CustomErrorsChecker<'a, 's> {
    ctx: &'a LintContext<'s, 'a>,
}

impl<'ast> Visit<'ast> for CustomErrorsChecker<'_, '_> {
    type BreakValue = ();

    fn visit_stmt(&mut self, stmt: &'ast Stmt<'ast>) -> ControlFlow<Self::BreakValue> {
        if let StmtKind::Expr(expr) = &stmt.kind
            && let ExprKind::Call(callee, args) = &expr.kind
            && ((is_require_call(callee) && should_lint_require(args))
                || (is_revert_call(callee) && should_lint_revert(args)))
        {
            self.ctx.emit(&CUSTOM_ERRORS, expr.span);
        }
        self.walk_stmt(stmt)
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
