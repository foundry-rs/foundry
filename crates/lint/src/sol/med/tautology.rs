use super::TypeBasedTautology;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use alloy_primitives::U256;
use solar::{
    ast::{BinOpKind, LitKind, UnOpKind},
    sema::hir::{self, ElementaryType, ExprKind, ItemId, Res, TypeKind},
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
        hir: &'hir hir::Hir<'hir>,
        expr: &'hir hir::Expr<'hir>,
    ) {
        let ExprKind::Binary(left, op, right) = &expr.kind else { return };

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

/// Extracts a signed constant from a numeric literal or negated numeric literal,
/// returning `(is_negative, magnitude)`.
fn lit_value_of(expr: &hir::Expr<'_>) -> Option<(bool, U256)> {
    match &expr.peel_parens().kind {
        ExprKind::Lit(lit) => {
            if let LitKind::Number(n) = lit.kind {
                return Some((false, n));
            }
            None
        }
        ExprKind::Unary(op, inner) if op.kind == UnOpKind::Neg => {
            if let ExprKind::Lit(lit) = &inner.peel_parens().kind
                && let LitKind::Number(n) = lit.kind
            {
                return Some((true, n));
            }
            None
        }
        _ => None,
    }
}
