use super::TypeBasedTautology;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use alloy_primitives::U256;
use solar::{
    ast::{BinOpKind, LitKind, UnOpKind},
    sema::{
        Gcx,
        hir::{self, ElementaryType, ExprKind, ItemId, Res, TypeKind, VariableId},
    },
};

declare_forge_lint!(
    TYPE_BASED_TAUTOLOGY,
    Severity::Med,
    "type-based-tautology",
    "condition is always true or false based on the variable's type"
);

impl<'hir> LateLintPass<'hir> for TypeBasedTautology {
    fn check_expr(
        &mut self,
        ctx: &LintContext,
        _gcx: Gcx<'hir>,
        hir: &'hir hir::Hir<'hir>,
        expr: &'hir hir::Expr<'hir>,
    ) {
        let ExprKind::Binary(left, op, right) = &expr.kind else { return };

        // A pair of comparisons can cover the complete type range even when neither
        // comparison is tautological on its own, e.g. `x > 0 || x == 0` for `uint`.
        if op.kind == BinOpKind::Or
            && let (Some(left), Some(right)) = (comparison_of(hir, left), comparison_of(hir, right))
            && is_boundary_composition(&left, &right)
        {
            ctx.emit(&TYPE_BASED_TAUTOLOGY, expr.span);
            return;
        }

        // Only relational/equality comparisons can produce tautologies via type bounds.
        if !matches!(
            op.kind,
            BinOpKind::Lt
                | BinOpKind::Le
                | BinOpKind::Gt
                | BinOpKind::Ge
                | BinOpKind::Eq
                | BinOpKind::Ne
        ) {
            return;
        }

        // var op const
        if let Some(elem_ty) = elem_type_of(hir, left)
            && let Some((val_neg, val_mag)) = lit_value_of(right)
            && is_tautology(elem_ty, val_neg, val_mag, op.kind)
        {
            ctx.emit(&TYPE_BASED_TAUTOLOGY, expr.span);
            return;
        }

        // const op var: swap operands and flip the operator
        if let Some((val_neg, val_mag)) = lit_value_of(left)
            && let Some(elem_ty) = elem_type_of(hir, right)
            && is_tautology(elem_ty, val_neg, val_mag, flip(op.kind))
        {
            ctx.emit(&TYPE_BASED_TAUTOLOGY, expr.span);
        }
    }
}

#[derive(Clone)]
struct Comparison {
    variable: VariableId,
    cast_path: Vec<ElementaryType>,
    range: IntegerRange,
    op: BinOpKind,
    val_neg: bool,
    val_mag: U256,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct IntegerRange {
    lower: (bool, U256),
    upper: (bool, U256),
}

/// Extracts a comparison over one resolved integer variable, normalizing constants on the left.
fn comparison_of<'hir>(
    hir: &'hir hir::Hir<'hir>,
    expr: &'hir hir::Expr<'hir>,
) -> Option<Comparison> {
    let ExprKind::Binary(left, op, right) = expr.peel_parens().kind else { return None };
    if !matches!(
        op.kind,
        BinOpKind::Lt
            | BinOpKind::Le
            | BinOpKind::Gt
            | BinOpKind::Ge
            | BinOpKind::Eq
            | BinOpKind::Ne
    ) {
        return None;
    }

    if let (Some((variable, cast_path, range)), Some((val_neg, val_mag))) =
        (comparison_operand_of(hir, left), lit_value_of(right))
    {
        return Some(Comparison { variable, cast_path, range, op: op.kind, val_neg, val_mag });
    }

    if let (Some((val_neg, val_mag)), Some((variable, cast_path, range))) =
        (lit_value_of(left), comparison_operand_of(hir, right))
    {
        return Some(Comparison {
            variable,
            cast_path,
            range,
            op: flip(op.kind),
            val_neg,
            val_mag,
        });
    }

    None
}

/// Returns true for boundary comparisons whose union covers the complete integer type range.
fn is_boundary_composition(left: &Comparison, right: &Comparison) -> bool {
    if left.variable != right.variable
        || left.cast_path != right.cast_path
        || left.range != right.range
    {
        return false;
    }

    let lower = left.range.lower;
    let upper = left.range.upper;

    // Values greater than the minimum plus the minimum itself cover the whole range.
    (matches_comparison(left, BinOpKind::Gt, lower) && is_lower_point(right, lower))
        || (matches_comparison(right, BinOpKind::Gt, lower) && is_lower_point(left, lower))
        // Values below the maximum plus the maximum itself cover the whole range.
        || (matches_comparison(left, BinOpKind::Lt, upper) && is_upper_point(right, upper))
        || (matches_comparison(right, BinOpKind::Lt, upper) && is_upper_point(left, upper))
        // Strict comparisons against opposite boundaries cover the whole range.
        || (matches_comparison(left, BinOpKind::Gt, lower)
            && matches_comparison(right, BinOpKind::Lt, upper))
        || (matches_comparison(right, BinOpKind::Gt, lower)
            && matches_comparison(left, BinOpKind::Lt, upper))
}

fn matches_comparison(comparison: &Comparison, op: BinOpKind, value: (bool, U256)) -> bool {
    comparison.op == op && comparison.val_neg == value.0 && comparison.val_mag == value.1
}

fn is_lower_point(comparison: &Comparison, lower: (bool, U256)) -> bool {
    (comparison.op == BinOpKind::Eq || comparison.op == BinOpKind::Le)
        && comparison.val_neg == lower.0
        && comparison.val_mag == lower.1
}

fn is_upper_point(comparison: &Comparison, upper: (bool, U256)) -> bool {
    (comparison.op == BinOpKind::Eq || comparison.op == BinOpKind::Ge)
        && comparison.val_neg == upper.0
        && comparison.val_mag == upper.1
}

fn integer_bounds(ty: ElementaryType) -> Option<IntegerRange> {
    match ty {
        ElementaryType::UInt(size) => {
            let bits = size.bits();
            let upper =
                if bits == 256 { U256::MAX } else { (U256::from(1u8) << bits) - U256::from(1u8) };
            Some(IntegerRange { lower: (false, U256::ZERO), upper: (false, upper) })
        }
        ElementaryType::Int(size) => {
            let half = U256::from(1u8) << (size.bits() - 1);
            Some(IntegerRange { lower: (true, half), upper: (false, half - U256::from(1u8)) })
        }
        _ => None,
    }
}

/// Returns the equivalent operator after swapping left and right operands.
/// e.g. `const < var` rewritten as `var > const` needs `Gt`.
const fn flip(op: BinOpKind) -> BinOpKind {
    match op {
        BinOpKind::Lt => BinOpKind::Gt,
        BinOpKind::Le => BinOpKind::Ge,
        BinOpKind::Gt => BinOpKind::Lt,
        BinOpKind::Ge => BinOpKind::Le,
        BinOpKind::Eq | BinOpKind::Ne => op, // symmetric
        _ => unreachable!(),
    }
}

/// Returns true if `var <op> val` is always true or always false for every value in the
/// type's range.
///
/// The constant is represented as a sign bit (`val_neg`) and a magnitude (`val_mag`), matching
/// how solar stores negated literals (e.g. `-128` -> `Unary(Neg, Lit(128))`).
fn is_tautology(ty: ElementaryType, val_neg: bool, val_mag: U256, op: BinOpKind) -> bool {
    match ty {
        ElementaryType::UInt(size) => {
            // lo = 0, hi = 2^bits - 1
            let bits = size.bits();
            let hi =
                if bits == 256 { U256::MAX } else { (U256::from(1u8) << bits) - U256::from(1u8) };
            let val_lt_lo = val_neg && val_mag != U256::ZERO; // val < 0
            let val_le_lo = val_neg || val_mag == U256::ZERO; // val <= 0
            let hi_lt_val = !val_neg && val_mag > hi; // val > hi
            let hi_le_val = !val_neg && val_mag >= hi; // val >= hi
            match op {
                BinOpKind::Gt | BinOpKind::Le => hi_le_val || val_lt_lo,
                BinOpKind::Ge | BinOpKind::Lt => val_le_lo || hi_lt_val,
                BinOpKind::Eq | BinOpKind::Ne => hi_lt_val || val_lt_lo,
                _ => false,
            }
        }
        ElementaryType::Int(size) => {
            // lo = -(2^(bits-1)), hi = 2^(bits-1) - 1
            let bits = size.bits();
            let half = U256::from(1u8) << (bits - 1); // 2^(bits-1)
            let hi = half - U256::from(1u8); // 2^(bits-1) - 1
            let val_lt_lo = val_neg && val_mag > half; // val < -half
            let val_le_lo = val_neg && val_mag >= half; // val <= -half
            let hi_lt_val = !val_neg && val_mag > hi; // val > hi
            let hi_le_val = !val_neg && val_mag >= hi; // val >= hi
            match op {
                BinOpKind::Gt | BinOpKind::Le => hi_le_val || val_lt_lo,
                BinOpKind::Ge | BinOpKind::Lt => val_le_lo || hi_lt_val,
                BinOpKind::Eq | BinOpKind::Ne => hi_lt_val || val_lt_lo,
                _ => false,
            }
        }
        _ => false,
    }
}

/// Extracts the elementary integer type from a variable reference or explicit cast.
fn elem_type_of<'hir>(
    hir: &'hir hir::Hir<'hir>,
    expr: &'hir hir::Expr<'hir>,
) -> Option<ElementaryType> {
    match &expr.peel_parens().kind {
        ExprKind::Ident(resolutions) => {
            if let Some(Res::Item(ItemId::Variable(var_id))) = resolutions.first()
                && let TypeKind::Elementary(ty) = hir.variable(*var_id).ty.kind
            {
                return Some(ty);
            }
            None
        }
        // Explicit cast: `uint8(x)`, the cast type determines the effective range.
        ExprKind::Call(call_expr, _, _) => {
            if let ExprKind::Type(hir::Type { kind: TypeKind::Elementary(ty), .. }) =
                &call_expr.kind
            {
                return Some(*ty);
            }
            None
        }
        _ => None,
    }
}

/// Extracts a stable variable identity and the reachable integer range of an operand.
///
/// Explicit casts can change the range used for the comparison, but not necessarily the
/// underlying value being compared. Keeping the cast path as part of the identity lets boundary
/// compositions recognize identical nested casts without treating different conversions as
/// identical.
fn comparison_operand_of<'hir>(
    hir: &'hir hir::Hir<'hir>,
    expr: &'hir hir::Expr<'hir>,
) -> Option<(VariableId, Vec<ElementaryType>, IntegerRange)> {
    match &expr.peel_parens().kind {
        ExprKind::Ident(resolutions) => {
            if let Some(Res::Item(ItemId::Variable(variable))) = resolutions.first()
                && let TypeKind::Elementary(ty) = hir.variable(*variable).ty.kind
            {
                return integer_bounds(ty).map(|range| (*variable, Vec::new(), range));
            }
        }
        ExprKind::Call(call_expr, args, _) => {
            let ExprKind::Type(hir::Type { kind: TypeKind::Elementary(ty), .. }) = &call_expr.kind
            else {
                return None;
            };
            if !matches!(ty, ElementaryType::Int(_) | ElementaryType::UInt(_)) {
                return None;
            }

            let mut exprs = args.exprs();
            let inner = exprs.next()?;
            if exprs.next().is_some() {
                return None;
            }
            let (variable, mut cast_path, source_range) = comparison_operand_of(hir, inner)?;
            let source_type = expression_type(hir, inner)?;
            let range = effective_range_for_cast(source_type, source_range, *ty)?;
            cast_path.push(*ty);
            return Some((variable, cast_path, range));
        }
        _ => {}
    }
    None
}

fn effective_range_for_cast(
    source_type: ElementaryType,
    source_range: IntegerRange,
    target_type: ElementaryType,
) -> Option<IntegerRange> {
    if is_value_preserving_widening(source_type, target_type) {
        Some(source_range)
    } else {
        integer_bounds(target_type)
    }
}

const fn is_value_preserving_widening(
    source_type: ElementaryType,
    target_type: ElementaryType,
) -> bool {
    match (source_type, target_type) {
        (ElementaryType::UInt(source), ElementaryType::UInt(target))
        | (ElementaryType::Int(source), ElementaryType::Int(target)) => {
            source.bits() <= target.bits()
        }
        _ => false,
    }
}

fn expression_type<'hir>(
    hir: &'hir hir::Hir<'hir>,
    expr: &'hir hir::Expr<'hir>,
) -> Option<ElementaryType> {
    match &expr.peel_parens().kind {
        ExprKind::Ident(resolutions) => {
            if let Some(Res::Item(ItemId::Variable(variable))) = resolutions.first()
                && let TypeKind::Elementary(ty) = hir.variable(*variable).ty.kind
            {
                return Some(ty);
            }
        }
        ExprKind::Call(call_expr, _, _) => {
            if let ExprKind::Type(hir::Type { kind: TypeKind::Elementary(ty), .. }) =
                &call_expr.kind
            {
                return Some(*ty);
            }
        }
        _ => {}
    }
    None
}

/// Extracts a signed constant from a numeric literal or negated numeric literal,
/// returning `(is_negative, magnitude)`.
fn lit_value_of(expr: &hir::Expr<'_>) -> Option<(bool, U256)> {
    match &expr.peel_parens().kind {
        ExprKind::Lit(lit) => {
            if let LitKind::Number(n) = lit.kind {
                return Some(normalize_zero(false, n));
            }
            None
        }
        ExprKind::Unary(op, inner) if op.kind == UnOpKind::Neg => {
            if let ExprKind::Lit(lit) = &inner.peel_parens().kind
                && let LitKind::Number(n) = lit.kind
            {
                return Some(normalize_zero(true, n));
            }
            None
        }
        _ => None,
    }
}

fn normalize_zero(is_negative: bool, magnitude: U256) -> (bool, U256) {
    if magnitude.is_zero() { (false, U256::ZERO) } else { (is_negative, magnitude) }
}
