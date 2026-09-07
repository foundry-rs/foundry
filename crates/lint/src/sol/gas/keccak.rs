use super::AsmKeccak256;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint, analysis::is_builtin},
};
use solar::{
    interface::kw,
    sema::{
        Gcx,
        hir::{self, ExprKind, StmtKind},
    },
};

declare_forge_lint!(
    ASM_KECCAK256,
    Severity::Gas,
    "asm-keccak256",
    "use of inefficient hashing mechanism; consider using inline assembly"
);

impl<'gcx> LateLintPass<'gcx> for AsmKeccak256 {
    fn check_stmt(&mut self, ctx: &LintContext, gcx: Gcx<'gcx>, stmt: &'gcx hir::Stmt<'gcx>) {
        let expr = match stmt.kind {
            StmtKind::DeclSingle(var_id) => gcx.hir.variable(var_id).initializer,
            StmtKind::Expr(expr)
            | StmtKind::Emit(expr)
            | StmtKind::Revert(expr)
            | StmtKind::DeclMulti(_, expr)
            | StmtKind::If(expr, ..)
            | StmtKind::Return(Some(expr)) => Some(expr),
            _ => None,
        };
        if let Some(expr) = expr
            && let ExprKind::Call(callee, args, _) = &expr.kind
            && args.len() == 1
            && is_builtin(callee, kw::Keccak256)
        {
            ctx.emit(&ASM_KECCAK256, expr.span);
        }
    }
}
