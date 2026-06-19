use super::TautologicalCompare;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::{BinOpKind, Lit, LitKind},
    sema::{
        Gcx,
        hir::{self, ElementaryType, Expr, ExprKind, TypeKind},
        ty::TyKind,
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
        gcx: Gcx<'hir>,
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
            && !operand_is_udvt(gcx, left)
        {
            ctx.emit(&TAUTOLOGICAL_COMPARE, expr.span);
        }
    }
}

/// Returns `true` if `expr`'s type is a user-defined value type (UDVT).
///
/// A UDVT can only be compared through a user-defined operator (`using {f as ==} for T global`),
/// which dispatches to an arbitrary function instead of built-in equality, so `x == x` is not
/// guaranteed to be tautological. Built-in comparisons only apply to elementary types, so skipping
/// UDVT operands removes that false positive without missing any real self-comparison.
/// See <https://soliditylang.org/blog/2023/02/22/user-defined-operators/>.
fn operand_is_udvt<'hir>(gcx: Gcx<'hir>, expr: &Expr<'hir>) -> bool {
    gcx.type_of_expr(expr.peel_parens().id)
        .is_some_and(|ty| matches!(ty.peel_refs().kind, TyKind::Udvt(..)))
}

/// Structural equality for the side-effect-free expressions a self-comparison can involve:
/// identifiers, member access, indexing (by an equal index), and elementary-type casts. Anything
/// else (notably arbitrary calls, which may return different values or have side effects, and
/// inc/dec unary ops) is treated as unequal, so the lint never fires on a comparison whose two
/// sides could legitimately differ.
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
        // Casts to the *same* elementary type (`uint256(x)`, `address(this)`) are pure conversions,
        // so two such casts of structurally-equal operands are equal. The cast types must match:
        // `uint256(x) == uint8(x)` is not tautological because the narrower cast can truncate.
        // Restricted to elementary-type casts, never arbitrary calls (which may have side effects
        // or return different values).
        (ExprKind::Call(ca, args_a, _), ExprKind::Call(cb, args_b, _)) => {
            match (cast_elem_type(ca), cast_elem_type(cb)) {
                (Some(ea), Some(eb)) if ea == eb => {
                    args_a.len() == 1
                        && args_b.len() == 1
                        && match (args_a.exprs().next(), args_b.exprs().next()) {
                            (Some(ia), Some(ib)) => exprs_equal(ia, ib),
                            _ => false,
                        }
                }
                _ => false,
            }
        }
        _ => false,
    }
}

/// If `callee` is an elementary-type name used as a cast (`uint256`, `address`, `bytesN`, ...),
/// returns that type, else `None`.
fn cast_elem_type<'a>(callee: &'a Expr<'_>) -> Option<&'a ElementaryType> {
    match &callee.peel_parens().kind {
        ExprKind::Type(hir::Type { kind: TypeKind::Elementary(e), .. }) => Some(e),
        _ => None,
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
