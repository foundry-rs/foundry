use super::TautologicalCompare;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::{BinOpKind, Lit, LitKind},
    sema::{
        Gcx,
        hir::{self, Expr, ExprKind},
    },
};

declare_forge_lint!(
    TAUTOLOGICAL_COMPARE,
    Severity::Med,
    "tautological-compare",
    "comparing an expression with itself is always true or false"
);

impl<'hir> LateLintPass<'hir> for TautologicalCompare {
    fn check_expr(
        &mut self,
        ctx: &LintContext,
        _gcx: Gcx<'hir>,
        _hir: &'hir hir::Hir<'hir>,
        expr: &'hir hir::Expr<'hir>,
    ) {
        if let ExprKind::Binary(left, op, right) = &expr.kind
            && matches!(
                op.kind,
                BinOpKind::Lt
                    | BinOpKind::Le
                    | BinOpKind::Gt
                    | BinOpKind::Ge
                    | BinOpKind::Eq
                    | BinOpKind::Ne
            )
            && exprs_equal(left, right)
        {
            ctx.emit(&TAUTOLOGICAL_COMPARE, expr.span);
        }
    }
}

/// Structural equality for the side-effect-free expressions a self-comparison can involve:
/// identifiers, member access, and indexing (by an equal index). Anything else (notably calls,
/// which may return different values or have side effects, and inc/dec unary ops) is treated as
/// unequal, so the lint never fires on a comparison whose two sides could legitimately differ.
fn exprs_equal<'hir>(a: &Expr<'hir>, b: &Expr<'hir>) -> bool {
    match (&a.peel_parens().kind, &b.peel_parens().kind) {
        (ExprKind::Ident(ra), ExprKind::Ident(rb)) => ra == rb,
        (ExprKind::Lit(la), ExprKind::Lit(lb)) => lits_equal(la, lb),
        (ExprKind::Member(ba, na), ExprKind::Member(bb, nb)) => {
            na.name == nb.name && exprs_equal(ba, bb)
        }
        (ExprKind::Index(ba, ia), ExprKind::Index(bb, ib)) => {
            exprs_equal(ba, bb) && opt_exprs_equal(*ia, *ib)
        }
        _ => false,
    }
}

fn lits_equal(a: &Lit<'_>, b: &Lit<'_>) -> bool {
    match (&a.kind, &b.kind) {
        (LitKind::Str(ak, av, _), LitKind::Str(bk, bv, _)) => ak == bk && av == bv,
        (LitKind::Number(a), LitKind::Number(b)) => a == b,
        (LitKind::Rational(a), LitKind::Rational(b)) => a == b,
        (LitKind::Address(a), LitKind::Address(b)) => a == b,
        (LitKind::Bool(a), LitKind::Bool(b)) => a == b,
        _ => false,
    }
}

fn opt_exprs_equal<'hir>(a: Option<&Expr<'hir>>, b: Option<&Expr<'hir>>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => exprs_equal(a, b),
        (None, None) => true,
        _ => false,
    }
}
