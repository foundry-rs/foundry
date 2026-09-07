use super::EncodedPackedCollision;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{
        Severity, SolLint,
        analysis::{expr_ty, is_builtin},
    },
};
use solar::{
    ast::LitKind,
    interface::sym,
    sema::{
        Gcx,
        hir::{Expr, ExprKind},
    },
};

declare_forge_lint!(
    ENCODE_PACKED_COLLISION,
    Severity::High,
    "encode-packed-collision",
    "`abi.encodePacked()` called with multiple dynamic type arguments; hash collisions possible"
);

impl<'gcx> LateLintPass<'gcx> for EncodedPackedCollision {
    fn check_expr(&mut self, ctx: &LintContext, gcx: Gcx<'gcx>, expr: &'gcx Expr<'gcx>) {
        let ExprKind::Call(callee, args, _) = &expr.kind else { return };
        let ExprKind::Member(base, member) = &callee.peel_parens().kind else { return };
        if member.name != sym::encodePacked || !is_builtin(base, sym::abi) {
            return;
        }
        // Only non-literal dynamic args count: a top-level string/hex/unicode literal is a
        // compile-time constant. With at most one non-literal dynamic arg the packed encoding
        // is still injective, so there is no collision risk.
        let dynamic_count =
            args.exprs().filter(|arg| !is_str_lit(arg) && is_dynamic_arg(gcx, arg)).count();
        if dynamic_count >= 2 {
            ctx.emit(&ENCODE_PACKED_COLLISION, expr.span);
        }
    }
}

fn is_str_lit(expr: &Expr<'_>) -> bool {
    matches!(expr.peel_parens().kind, ExprKind::Lit(lit) if matches!(lit.kind, LitKind::Str(..)))
}

fn is_dynamic_arg<'gcx>(gcx: Gcx<'gcx>, expr: &Expr<'gcx>) -> bool {
    match &expr.peel_parens().kind {
        // String literals (and multi-line/hex string sequences) are always dynamic.
        ExprKind::Lit(_) => is_str_lit(expr),
        // Ternary: dynamic when both branches are dynamic. Handled here so that literal branches
        // (which have no checked type) are correctly identified as dynamic.
        ExprKind::Ternary(_, then, else_) => {
            is_dynamic_arg(gcx, then) && is_dynamic_arg(gcx, else_)
        }
        _ => expr_ty(gcx, expr).is_some_and(|ty| ty.peel_refs().is_dynamically_sized()),
    }
}
