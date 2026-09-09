use super::UnsafeCheatcodes;
use crate::{
    linter::{EarlyLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::ast::{Expr, ExprKind};

declare_forge_lint!(UNSAFE_CHEATCODE_USAGE, Severity::Info, "unsafe-cheatcode");

const UNSAFE_CHEATCODES: &[&str] = &[
    "ffi",
    "readFile",
    "readLine",
    "writeFile",
    "writeLine",
    "removeFile",
    "closeFile",
    "setEnv",
    "deriveKey",
];

impl<'ast> EarlyLintPass<'ast> for UnsafeCheatcodes {
    fn check_expr(&mut self, ctx: &LintContext, expr: &'ast Expr<'ast>) {
        if let ExprKind::Call(callee, _) = &expr.kind
            && let ExprKind::Member(_, member) = &callee.kind
            && UNSAFE_CHEATCODES.contains(&member.as_str())
        {
            ctx.span_lint(&UNSAFE_CHEATCODE_USAGE, member.span, |diag| {
                diag.primary_message(
                    "usage of unsafe cheatcodes that can perform dangerous operations",
                );
            });
        }
    }
}
