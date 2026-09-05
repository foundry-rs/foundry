use super::TypeBasedTautology;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint, analysis::cast_type},
};
use alloy_primitives::U256;
use solar::{
    ast::{BinOpKind, LitKind, UnOpKind},
    sema::{
        Gcx,
        hir::{self, ElementaryType, Expr, ExprKind, TypeKind, VariableId},
    },
};
use std::cmp::Ordering;

declare_forge_lint!(
    TYPE_BASED_TAUTOLOGY,
    Severity::Med,
    "type-based-tautology",
    "condition is always true or false based on the variable's type"
);

impl<'gcx> LateLintPass<'gcx> for TypeBasedTautology {
    fn check_expr(&mut self, ctx: &LintContext, gcx: Gcx<'gcx>, expr: &'gcx Expr<'gcx>) {
        let ExprKind::Binary(left, op, right) = &expr.kind else { return };

        // A pair of comparisons can cover the complete type range even when neither is
        // tautological on its own, e.g. `x > 0 || x == 0` for `uint`.
        let is_tautology = if op.kind == BinOpKind::Or {
            matches!((comparison_of(&gcx.hir, left), comparison_of(&gcx.hir, right)),
                (Some(l), Some(r)) if is_boundary_composition(&l, &r))
        } else {
            split_comparison(expr).is_some_and(|(operand, val, op)| {
                elem_type_of(&gcx.hir, operand)
                    .and_then(integer_bounds)
                    .is_some_and(|range| is_tautology(range, val, op))
            })
        };
        if is_tautology {
            ctx.emit(&TYPE_BASED_TAUTOLOGY, expr.span);
        }
    }
}

/// A signed integer constant as `(is_negative, magnitude)`, matching how solar stores negated
/// literals (`-128` is `Unary(Neg, Lit(128))`). Zero is always `(false, 0)`.
type Const = (bool, U256);

fn cmp(a: Const, b: Const) -> Ordering {
    match (a.0, b.0) {
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        (false, false) => a.1.cmp(&b.1),
        (true, true) => b.1.cmp(&a.1),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Range {
    lo: Const,
    hi: Const,
}

fn integer_bounds(ty: ElementaryType) -> Option<Range> {
    match ty {
        ElementaryType::UInt(size) => {
            let bits = size.bits();
            let hi = if bits == 256 { U256::MAX } else { (U256::ONE << bits) - U256::ONE };
            Some(Range { lo: (false, U256::ZERO), hi: (false, hi) })
        }
        ElementaryType::Int(size) => {
            let half = U256::ONE << (size.bits() - 1);
            Some(Range { lo: (true, half), hi: (false, half - U256::ONE) })
        }
        _ => None,
    }
}

/// True if `x <op> val` has the same truth value for every `x` in `range`.
fn is_tautology(range: Range, val: Const, op: BinOpKind) -> bool {
    let (lo, hi) = (cmp(val, range.lo), cmp(val, range.hi));
    match op {
        BinOpKind::Gt | BinOpKind::Le => hi.is_ge() || lo.is_lt(),
        BinOpKind::Ge | BinOpKind::Lt => lo.is_le() || hi.is_gt(),
        BinOpKind::Eq | BinOpKind::Ne => hi.is_gt() || lo.is_lt(),
        _ => false,
    }
}

/// A relational/equality comparison between an operand and a constant, normalized to
/// `operand <op> const` (the operator is flipped when the constant is on the left).
fn split_comparison<'gcx>(expr: &'gcx Expr<'gcx>) -> Option<(&'gcx Expr<'gcx>, Const, BinOpKind)> {
    let ExprKind::Binary(left, op, right) = &expr.peel_parens().kind else { return None };
    let flipped = match op.kind {
        BinOpKind::Lt => BinOpKind::Gt,
        BinOpKind::Le => BinOpKind::Ge,
        BinOpKind::Gt => BinOpKind::Lt,
        BinOpKind::Ge => BinOpKind::Le,
        BinOpKind::Eq | BinOpKind::Ne => op.kind,
        _ => return None,
    };
    match lit_value_of(right) {
        Some(val) => Some((left, val, op.kind)),
        None => Some((right, lit_value_of(left)?, flipped)),
    }
}

struct Comparison {
    variable: VariableId,
    cast_path: Vec<ElementaryType>,
    range: Range,
    op: BinOpKind,
    val: Const,
}

/// A comparison of one resolved integer variable (possibly cast) against a constant.
fn comparison_of<'gcx>(hir: &hir::Hir<'gcx>, expr: &'gcx Expr<'gcx>) -> Option<Comparison> {
    let (operand, val, op) = split_comparison(expr)?;
    let (variable, cast_path, range) = comparison_operand_of(hir, operand)?;
    Some(Comparison { variable, cast_path, range, op, val })
}

/// True when two comparisons over the same operand together cover its whole range.
fn is_boundary_composition(l: &Comparison, r: &Comparison) -> bool {
    if l.variable != r.variable || l.cast_path != r.cast_path || l.range != r.range {
        return false;
    }
    let Range { lo, hi } = l.range;
    let is = |c: &Comparison, op, val| c.op == op && c.val == val;
    let at_lo = |c| is(c, BinOpKind::Eq, lo) || is(c, BinOpKind::Le, lo);
    let at_hi = |c| is(c, BinOpKind::Eq, hi) || is(c, BinOpKind::Ge, hi);
    [(l, r), (r, l)].into_iter().any(|(a, b)| {
        (is(a, BinOpKind::Gt, lo) && (at_lo(b) || is(b, BinOpKind::Lt, hi)))
            || (is(a, BinOpKind::Lt, hi) && at_hi(b))
    })
}

/// The variable an operand compares, the casts applied to it and the range its values span.
///
/// A same-signed widening cast preserves the value, so it neither changes the range nor
/// distinguishes the operand from the uncast expression; any other cast resets the range to the
/// target type's and becomes part of the operand's identity.
fn comparison_operand_of<'gcx>(
    hir: &hir::Hir<'gcx>,
    expr: &'gcx Expr<'gcx>,
) -> Option<(VariableId, Vec<ElementaryType>, Range)> {
    match &expr.peel_parens().kind {
        ExprKind::Ident(reses) => {
            let variable = reses.first()?.as_variable()?;
            let TypeKind::Elementary(ty) = hir.variable(variable).ty.kind else { return None };
            Some((variable, Vec::new(), integer_bounds(ty)?))
        }
        ExprKind::Call(callee, args, _) if args.len() == 1 => {
            let ty = cast_type(callee)?;
            let inner = args.exprs().next()?;
            let (variable, mut cast_path, range) = comparison_operand_of(hir, inner)?;
            let source = elem_type_of(hir, inner)?;
            let widening = match (source, ty) {
                (ElementaryType::UInt(from), ElementaryType::UInt(to))
                | (ElementaryType::Int(from), ElementaryType::Int(to)) => from.bits() <= to.bits(),
                _ => false,
            };
            if widening {
                return Some((variable, cast_path, range));
            }
            cast_path.push(ty);
            Some((variable, cast_path, integer_bounds(ty)?))
        }
        _ => None,
    }
}

/// The elementary type of a variable reference or of an explicit cast's target.
fn elem_type_of(hir: &hir::Hir<'_>, expr: &Expr<'_>) -> Option<ElementaryType> {
    match &expr.peel_parens().kind {
        ExprKind::Ident(reses) => match hir.variable(reses.first()?.as_variable()?).ty.kind {
            TypeKind::Elementary(ty) => Some(ty),
            _ => None,
        },
        ExprKind::Call(callee, ..) => cast_type(callee),
        _ => None,
    }
}

/// A numeric literal or negated numeric literal.
fn lit_value_of(expr: &Expr<'_>) -> Option<Const> {
    let (neg, lit) = match &expr.peel_parens().kind {
        ExprKind::Lit(lit) => (false, lit),
        ExprKind::Unary(op, inner) if op.kind == UnOpKind::Neg => match &inner.peel_parens().kind {
            ExprKind::Lit(lit) => (true, lit),
            _ => return None,
        },
        _ => return None,
    };
    let LitKind::Number(n) = lit.kind else { return None };
    Some((neg && !n.is_zero(), n))
}
