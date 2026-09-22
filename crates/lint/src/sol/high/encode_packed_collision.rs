use super::EncodedPackedCollision;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::LitKind,
    sema::{
        Gcx,
        builtins::Builtin,
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
        if gcx.resolved_builtin(callee) != Some(Builtin::AbiEncodePacked) {
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
    gcx.type_of_expr(expr.peel_parens().id).is_some_and(|ty| ty.peel_refs().is_dynamically_sized())
}
