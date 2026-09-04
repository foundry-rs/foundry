use super::CustomErrors;
use crate::{
    linter::{EarlyLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::{CallArgs, CallArgsKind, Expr, ExprKind, LitKind},
    interface::{kw, sym},
};

declare_forge_lint!(
    CUSTOM_ERRORS,
    Severity::Gas,
    "custom-errors",
    "prefer using custom errors on revert and require calls"
);

impl<'ast> EarlyLintPass<'ast> for CustomErrors {
    fn check_expr(&mut self, ctx: &LintContext, expr: &'ast Expr<'ast>) {
        let ExprKind::Call(callee, CallArgs { kind: CallArgsKind::Unnamed(args), .. }) =
            &expr.kind
        else {
            return;
        };
        let ExprKind::Ident(ident) = &callee.kind else { return };
        // `require(cond)` / `require(cond, "reason")` and `revert()` / `revert("reason")`.
        let lint = match ident.name {
            sym::require => args.len() == 1 || args.get(1).is_some_and(|e| is_string_literal(e)),
            kw::Revert => args.first().is_none_or(|e| is_string_literal(e)),
            _ => false,
        };
        if lint {
            ctx.emit(&CUSTOM_ERRORS, expr.span);
        }
    }
}

const fn is_string_literal(expr: &Expr<'_>) -> bool {
    matches!(&expr.kind, ExprKind::Lit(lit, _) if matches!(lit.kind, LitKind::Str(..)))
}
