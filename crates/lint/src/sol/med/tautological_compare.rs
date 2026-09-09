use super::TautologicalCompare;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint, analysis::cast_type},
};
use solar::{
    ast::{BinOpKind, Lit, LitKind},
    sema::{
        Gcx,
        hir::{self, Expr, ExprKind},
        ty::TyKind,
    },
};

declare_forge_lint!(
    TAUTOLOGICAL_COMPARE,
    Severity::Med,
    "tautological-compare",
    "comparing an expression with itself is always true or false"
);

impl<'gcx> LateLintPass<'gcx> for TautologicalCompare {
    fn check_expr(&mut self, ctx: &LintContext, gcx: Gcx<'gcx>, expr: &'gcx hir::Expr<'gcx>) {
        // A UDVT can only be compared through a user-defined operator (`using {f as ==} for T`),
        // which dispatches to an arbitrary function, so `x == x` is not tautological for it.
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
            && !gcx
                .type_of_expr(left.peel_parens().id)
                .is_some_and(|ty| matches!(ty.peel_refs().kind, TyKind::Udvt(..)))
        {
            ctx.emit(&TAUTOLOGICAL_COMPARE, expr.span);
        }
    }
}

/// Structural equality for side-effect-free expressions. Anything else (notably arbitrary calls,
/// which may return different values, and the mutating `++`/`--`) is treated as unequal.
fn exprs_equal<'gcx>(a: &Expr<'gcx>, b: &Expr<'gcx>) -> bool {
    match (&a.peel_parens().kind, &b.peel_parens().kind) {
        (ExprKind::Ident(ra), ExprKind::Ident(rb)) => ra == rb,
        (ExprKind::Lit(la), ExprKind::Lit(lb)) => literals_equal(la, lb),
        (ExprKind::Member(ba, na), ExprKind::Member(bb, nb)) => {
            na.name == nb.name && exprs_equal(ba, bb)
        }
        (ExprKind::Index(ba, ia), ExprKind::Index(bb, ib)) => {
            exprs_equal(ba, bb)
                && match (ia, ib) {
                    (Some(ia), Some(ib)) => exprs_equal(ia, ib),
                    (None, None) => true,
                    _ => false,
                }
        }
        (ExprKind::Binary(la, opa, ra), ExprKind::Binary(lb, opb, rb)) => {
            opa.kind == opb.kind && exprs_equal(la, lb) && exprs_equal(ra, rb)
        }
        // Only casts to the *same* elementary type are pure conversions: `uint256(x) == uint8(x)`
        // is not tautological because the narrower cast can truncate.
        (ExprKind::Call(ca, args_a, _), ExprKind::Call(cb, args_b, _)) => {
            matches!((cast_type(ca), cast_type(cb)), (Some(ea), Some(eb)) if ea == eb)
                && args_a.len() == 1
                && args_b.len() == 1
                && args_a.exprs().zip(args_b.exprs()).all(|(ia, ib)| exprs_equal(ia, ib))
        }
        (ExprKind::Payable(a), ExprKind::Payable(b)) => exprs_equal(a, b),
        (ExprKind::Unary(opa, a), ExprKind::Unary(opb, b)) => {
            opa.kind == opb.kind && !opa.kind.has_side_effects() && exprs_equal(a, b)
        }
        (ExprKind::Ternary(ca, ta, fa), ExprKind::Ternary(cb, tb, fb)) => {
            exprs_equal(ca, cb) && exprs_equal(ta, tb) && exprs_equal(fa, fb)
        }
        _ => false,
    }
}

fn literals_equal(a: &Lit<'_>, b: &Lit<'_>) -> bool {
    match (&a.kind, &b.kind) {
        (LitKind::Str(ak, av, _), LitKind::Str(bk, bv, _)) => ak == bk && av == bv,
        (LitKind::Number(a), LitKind::Number(b)) => a == b,
        (LitKind::Rational(a), LitKind::Rational(b)) => a == b,
        (LitKind::Address(a), LitKind::Address(b)) => a == b,
        (LitKind::Bool(a), LitKind::Bool(b)) => a == b,
        _ => false,
    }
}
