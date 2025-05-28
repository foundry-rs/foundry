use super::AsmKeccak256;
use crate::{
    declare_forge_lint,
    linter::EarlyLintPass,
    sol::{Severity, SolLint},
};
use solar_ast::{Expr, ExprKind};
use solar_interface::kw;

declare_forge_lint!(
    ASM_KECCAK256,
    Severity::Gas,
    "asm-keccak256",
    "hash using inline assembly to save gas"
);

impl<'ast> EarlyLintPass<'ast> for AsmKeccak256 {
    fn check_expr(&mut self, ctx: &crate::linter::LintContext<'_>, expr: &'ast Expr<'ast>) {
        if let ExprKind::Call(expr, _) = &expr.kind {
            if let ExprKind::Ident(ident) = &expr.kind {
                if ident.name == kw::Keccak256 {
                    ctx.emit(&ASM_KECCAK256, expr.span);
                }
            }
        }
    }
}
