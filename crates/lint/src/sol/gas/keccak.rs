use super::AsmKeccak256;
use crate::{
    linter::{EarlyLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar_ast::{CallArgsKind, Expr, ExprKind};
use solar_interface::kw;

declare_forge_lint!(
    ASM_KECCAK256,
    Severity::Gas,
    "asm-keccak256",
    "hash using inline assembly to save gas"
);

impl<'ast> EarlyLintPass<'ast> for AsmKeccak256 {
    fn check_expr(&mut self, ctx: &LintContext<'_>, expr: &'ast Expr<'ast>) {
        if let ExprKind::Call(expr, args) = &expr.kind
            && let ExprKind::Ident(ident) = &expr.kind
            && ident.name == kw::Keccak256
        {
            // Do not flag when hashing a single literal, as the compiler should optimize it
            if let CallArgsKind::Unnamed(ref exprs) = args.kind
                && exprs.len() == 1
                && let ExprKind::Lit(_, _) = exprs[0].kind
            {
                return;
            }
            ctx.emit(&ASM_KECCAK256, expr.span);
        }
    }
}
