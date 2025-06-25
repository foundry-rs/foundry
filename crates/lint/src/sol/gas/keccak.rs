use super::AsmKeccak256;
use crate::{
    linter::EarlyLintPass,
    sol::{Severity, SolLint},
};
use solar_ast::{CallArgsKind, Expr, ExprKind};
use solar_interface::kw;

declare_forge_lint!(
    ASM_KECCAK256,
    Severity::Gas,
    "asm-keccak256",
    "inefficient hashing mechanism",
    diff: {
        bad: "bytes32 hash = keccak256(abi.encodePacked(a, b));",
        good: "bytes32 hash;\nassembly {\n    mstore(0x00, a)\n    mstore(0x20, b)\n    hash := keccak256(0x00, 0x40)\n}",
        desc: "consider using inline assembly to reduce gas usage, like shown in this example:"
    }
);

impl<'ast> EarlyLintPass<'ast> for AsmKeccak256 {
    fn check_expr(&mut self, ctx: &crate::linter::LintContext<'_>, expr: &'ast Expr<'ast>) {
        if let ExprKind::Call(expr, args) = &expr.kind {
            if let ExprKind::Ident(ident) = &expr.kind {
                if ident.name == kw::Keccak256 {
                    // Do not flag when hashing a single literal, as the compiler should optimize it
                    if let CallArgsKind::Unnamed(ref exprs) = args.kind {
                        if exprs.len() == 1 {
                            if let ExprKind::Lit(_, _) = exprs[0].kind {
                                return;
                            }
                        }
                    }
                    ctx.emit(&ASM_KECCAK256, expr.span);
                }
            }
        }
    }
}
