//! Rounding relations between a rounded multiple and its original dividend.
//!
//! These bounds describe mathematical differences, not wrapping EVM subtractions.
//! They are consumed only where the sign or a non-wrapping product is established.

use super::{
    ConstraintContext, HashMap, MAX_LOCAL_ANALYSIS_NODES, SymBinOp, SymBoolExpr, SymBoolExprKind,
    SymCmpOp, SymExpr, SymExprKind, U256, WordInterval,
};

/// `anchor - below <= rounded <= anchor + above` over mathematical integers.
struct RoundingBounds<'a> {
    anchor: &'a SymExpr,
    below: U256,
    above: U256,
}

impl ConstraintContext {
    /// Matches `(dividend / d) * d` for a positive constant `d`.
    pub(super) fn rounded_product_operands(expr: &SymExpr) -> Option<(&SymExpr, U256)> {
        let (quotient, factor) = Self::constant_mul_operands(expr)?;
        let (dividend, divisor) = quotient.udiv_operands()?;
        (divisor.as_const() == Some(factor) && !factor.is_zero()).then_some((dividend, factor))
    }

    // Requesting an anchor preserves the raw dividend relation even when a safe
    // offset also exposes a relation to the pre-offset value. With no requested
    // anchor, prefer the shifted relation for quotient cancellation.
    fn rounding_bounds<'a>(
        &self,
        expr: &'a SymExpr,
        anchor: Option<&SymExpr>,
    ) -> Option<RoundingBounds<'a>> {
        let mut remaining = MAX_LOCAL_ANALYSIS_NODES;
        self.rounding_bounds_cached(expr, anchor, &mut HashMap::default(), &mut remaining)
    }

    fn rounding_bounds_cached<'a>(
        &self,
        expr: &'a SymExpr,
        anchor: Option<&SymExpr>,
        intervals: &mut HashMap<SymExpr, Option<WordInterval>>,
        remaining: &mut usize,
    ) -> Option<RoundingBounds<'a>> {
        let (dividend, divisor) = Self::rounded_product_operands(expr)?;
        // Euclidean division: q*d = dividend - remainder, 0 <= remainder < d.
        // In particular, q*d <= dividend <= MAX, so the rescaling cannot wrap.
        let width = divisor - U256::ONE;
        let raw = RoundingBounds { anchor: dividend, below: width, above: U256::ZERO };
        if anchor == Some(dividend) {
            return Some(raw);
        }
        let offset = match dividend.kind() {
            SymExprKind::BinOp(SymBinOp::Add, anchor, bias)
                if let Some(bias) = bias.as_const()
                    && bias <= width =>
            {
                Some((anchor, bias, bias))
            }
            SymExprKind::BinOp(SymBinOp::Sub, sum, one)
                if one.as_const() == Some(U256::ONE)
                    && let SymExprKind::BinOp(SymBinOp::Add, anchor, bias) = sum.kind()
                    && bias.as_const() == Some(divisor) =>
            {
                // Preserve the intermediate addition in `(anchor + d) - 1`.
                Some((anchor, width, divisor))
            }
            _ => None,
        };
        if let Some((anchor, bias, addition)) = offset
            && self
                .interval_cached(anchor, intervals, remaining)
                .is_some_and(|range| range.max.checked_add(addition).is_some())
        {
            // For dividend = anchor + bias, the error is bias - remainder.
            return Some(RoundingBounds { anchor, below: width - bias, above: bias });
        }
        // An unproved offset remains opaque. A bound on a wrapped sum cannot
        // justify removing that sum from the relation.
        Some(raw)
    }

    /// A nonnegative rounding error smaller than a divisor preserves its quotient.
    pub(super) fn quotient_of_rounded_product<'a>(&self, expr: &'a SymExpr) -> Option<&'a SymExpr> {
        let (numerator, divisor) = expr.udiv_operands()?;
        let bounds = self.rounding_bounds(numerator, None)?;
        let minimum =
            divisor.as_const().or_else(|| self.unsigned_lower_bounds.get(divisor).copied())?;
        if !bounds.below.is_zero() || bounds.above >= minimum {
            return None;
        }
        let SymExprKind::BinOp(SymBinOp::Mul, left, right) = bounds.anchor.kind() else {
            return None;
        };
        let value = if left == divisor {
            right
        } else if right == divisor {
            left
        } else {
            return None;
        };
        // The anchor is an EVM word. Cancelling a factor requires independent
        // evidence that its mathematical product did not wrap.
        self.mul_cannot_overflow_256(value, divisor).then_some(value)
    }

    pub(super) fn rounding_comparison_value(&self, expr: &SymBoolExpr) -> Option<bool> {
        if let SymBoolExprKind::Not(inner) = expr.kind() {
            return self.rounding_comparison_value(inner).map(|value| !value);
        }
        let SymBoolExprKind::Cmp(op, left, right) = expr.kind() else { return None };
        for (rounded, anchor, op) in [
            (left, right, *op),
            (
                right,
                left,
                match op {
                    SymCmpOp::Ult => SymCmpOp::Ugt,
                    SymCmpOp::Ule => SymCmpOp::Uge,
                    SymCmpOp::Ugt => SymCmpOp::Ult,
                    SymCmpOp::Uge => SymCmpOp::Ule,
                    other => *other,
                },
            ),
        ] {
            if let Some(bounds) = self.rounding_bounds(rounded, Some(anchor))
                && bounds.anchor == anchor
            {
                match op {
                    SymCmpOp::Uge if bounds.below.is_zero() => return Some(true),
                    SymCmpOp::Ult if bounds.below.is_zero() => return Some(false),
                    SymCmpOp::Ule if bounds.above.is_zero() => return Some(true),
                    SymCmpOp::Ugt if bounds.above.is_zero() => return Some(false),
                    _ => {}
                }
            }
        }
        None
    }

    /// Bounds a subtraction only when the relation proves it cannot underflow.
    pub(super) fn rounding_error_interval(
        &self,
        left: &SymExpr,
        right: &SymExpr,
        intervals: &mut HashMap<SymExpr, Option<WordInterval>>,
        remaining: &mut usize,
    ) -> Option<WordInterval> {
        if let Some(bounds) = self.rounding_bounds_cached(left, Some(right), intervals, remaining)
            && bounds.anchor == right
            && bounds.below.is_zero()
        {
            return Some(WordInterval { min: U256::ZERO, max: bounds.above });
        }
        if let Some(bounds) = self.rounding_bounds_cached(right, Some(left), intervals, remaining)
            && bounds.anchor == left
            && bounds.above.is_zero()
        {
            return Some(WordInterval { min: U256::ZERO, max: bounds.below });
        }
        None
    }
}
