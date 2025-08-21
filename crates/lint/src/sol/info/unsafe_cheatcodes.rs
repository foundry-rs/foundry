use super::UnsafeCheatcodes;
use crate::{
    linter::{EarlyLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar_ast::{Expr, ExprKind, ItemFunction, visit::Visit};
use std::ops::ControlFlow;

declare_forge_lint!(
    UNSAFE_CHEATCODE_USAGE,
    Severity::Info,
    "geiger-unsafe-cheatcode",
    "usage of unsafe cheatcodes that can perform dangerous operations"
);

impl<'ast> EarlyLintPass<'ast> for UnsafeCheatcodes {
    fn check_item_function(&mut self, ctx: &LintContext<'_>, func: &'ast ItemFunction<'ast>) {
        if let Some(body) = &func.body {
            let mut checker = UnsafeCheatcodeChecker {
                ctx,
                unsafe_cheatcodes: &[
                    "ffi",
                    "readFile",
                    "readLine",
                    "writeFile",
                    "writeLine",
                    "removeFile",
                    "closeFile",
                    "setEnv",
                    "deriveKey",
                ],
            };
            let _ = checker.visit_block(body);
        }
    }
}

struct UnsafeCheatcodeChecker<'a, 's> {
    ctx: &'a LintContext<'s>,
    unsafe_cheatcodes: &'a [&'a str],
}

impl<'ast> Visit<'ast> for UnsafeCheatcodeChecker<'_, '_> {
    type BreakValue = ();

    fn visit_expr(&mut self, expr: &'ast Expr<'ast>) -> ControlFlow<Self::BreakValue> {
        if let ExprKind::Call(lhs, _args) = &expr.kind {
            if let ExprKind::Member(_lhs, member) = &lhs.kind {
                if self.unsafe_cheatcodes.iter().any(|&c| c == member.as_str()) {
                    self.ctx.emit(&UNSAFE_CHEATCODE_USAGE, member.span);
                }
            }
        }
        self.walk_expr(expr)
    }
}
