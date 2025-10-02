use super::UnsafeCheatcodes;
use crate::{
    linter::{EarlyLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::ast::{Expr, ExprKind};

declare_forge_lint!(
    UNSAFE_CHEATCODE_USAGE,
    Severity::Info,
    "unsafe-cheatcode",
    "usage of unsafe cheatcodes that can perform dangerous operations"
);

const UNSAFE_CHEATCODES: [&str; 9] = [
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
        if let ExprKind::Call(lhs, _args) = &expr.kind
            && let ExprKind::Member(_lhs, member) = &lhs.kind
            && UNSAFE_CHEATCODES.iter().any(|&c| c == member.as_str())
        {
            ctx.emit(&UNSAFE_CHEATCODE_USAGE, member.span);
        }
    }
}
