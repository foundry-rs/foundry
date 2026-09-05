use super::IncorrectStrictEquality;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{
        Severity, SolLint,
        analysis::{is_address_like, referenced_item},
    },
};
use solar::{
    ast::BinOpKind,
    interface::kw,
    sema::{
        Gcx,
        hir::{Expr, ExprKind, ItemId},
    },
};
use std::ops::ControlFlow;

declare_forge_lint!(
    INCORRECT_STRICT_EQUALITY,
    Severity::Med,
    "incorrect-strict-equality",
    "dangerous strict equality check on an externally-influenced value"
);

impl<'gcx> LateLintPass<'gcx> for IncorrectStrictEquality {
    fn check_expr(&mut self, ctx: &LintContext, gcx: Gcx<'gcx>, expr: &'gcx Expr<'gcx>) {
        if let ExprKind::Binary(lhs, op, rhs) = &expr.kind
            && matches!(op.kind, BinOpKind::Eq | BinOpKind::Ne)
            && [lhs, rhs].into_iter().any(|side| {
                side.visit(&mut |e| {
                    if is_externally_influenced(gcx, e) {
                        ControlFlow::Break(())
                    } else {
                        ControlFlow::Continue(())
                    }
                })
                .is_break()
            })
        {
            ctx.emit(&INCORRECT_STRICT_EQUALITY, expr.span);
        }
    }
}

/// `<address>.balance` or `<non-library>.balanceOf(...)`.
///
/// `.balance` is only flagged when the receiver is provably an address, so that struct fields named
/// `balance` do not trigger it. `balanceOf` is matched by name (it is overwhelmingly an ERC-20
/// method), skipping static library calls to avoid internal helpers of the same name.
fn is_externally_influenced<'gcx>(gcx: Gcx<'gcx>, expr: &Expr<'gcx>) -> bool {
    match &expr.peel_parens().kind {
        ExprKind::Member(base, member) => member.name == kw::Balance && is_address_like(gcx, base),
        ExprKind::Call(callee, ..) => {
            matches!(&callee.peel_parens().kind, ExprKind::Member(base, m)
                if m.as_str() == "balanceOf"
                    && !matches!(referenced_item(base), Some(ItemId::Contract(cid))
                        if gcx.hir.contract(cid).kind.is_library()))
        }
        _ => false,
    }
}
