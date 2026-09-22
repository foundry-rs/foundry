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

#[cfg(test)]
mod tests {
    use super::{
        super::{
            SymCx, SymbolicModel, normalize_constraints_for_solver,
            normalize_constraints_for_solver_cached,
        },
        *,
    };

    fn rounded(cx: &mut SymCx, value: &SymExpr, divisor: U256, bias: U256, split: bool) -> SymExpr {
        let divisor_expr = SymExpr::constant(cx, divisor);
        let bias_expr = SymExpr::constant(cx, if split { bias + U256::ONE } else { bias });
        let dividend = SymExpr::binop(cx, SymBinOp::Add, value.clone(), bias_expr);
        let dividend = if split {
            let one = SymExpr::one(cx);
            SymExpr::binop(cx, SymBinOp::Sub, dividend, one)
        } else {
            dividend
        };
        let quotient = SymExpr::binop(cx, SymBinOp::UDiv, dividend, divisor_expr.clone());
        SymExpr::binop(cx, SymBinOp::Mul, quotient, divisor_expr)
    }

    #[test]
    fn ceiling_rounding_proves_order_and_error_for_both_addition_forms() {
        let mut cx = SymCx::new();
        let value = SymExpr::var(&mut cx, "value");
        let bound =
            SymBoolExpr::cmp_word_const(&mut cx, SymCmpOp::Ule, &value, U256::MAX - U256::from(37));
        let complement = SymExpr::not(&mut cx, value.clone());
        let complement_bound =
            SymBoolExpr::cmp_word_const(&mut cx, SymCmpOp::Uge, &complement, U256::from(37));
        for bound in [bound, complement_bound] {
            for split in [false, true] {
                let rounded = rounded(&mut cx, &value, U256::from(37), U256::from(36), split);
                let error = SymExpr::binop(&mut cx, SymBinOp::Sub, rounded.clone(), value.clone());
                let order = SymBoolExpr::cmp(&mut cx, SymCmpOp::Uge, rounded, value.clone());
                let error_bound =
                    SymBoolExpr::cmp_word_const(&mut cx, SymCmpOp::Ult, &error, U256::from(37));
                for property in [order, error_bound] {
                    let failure = property.not(&mut cx);
                    let normalized =
                        normalize_constraints_for_solver(&mut cx, &[bound.clone(), failure]);
                    assert_eq!(normalized, vec![SymBoolExpr::constant(&mut cx, false)]);
                }
            }
        }
    }

    #[test]
    fn safe_offsets_preserve_dividend_order_and_remainder() {
        let mut cx = SymCx::new();
        let value = SymExpr::var(&mut cx, "value");
        let safe =
            SymBoolExpr::cmp_word_const(&mut cx, SymCmpOp::Ule, &value, U256::MAX - U256::from(37));
        let mut cache = HashMap::default();
        for bias in [U256::ZERO, U256::from(18), U256::from(36)] {
            for split in [false, true] {
                let rounded = rounded(&mut cx, &value, U256::from(37), bias, split);
                let (dividend, _) = ConstraintContext::rounded_product_operands(&rounded).unwrap();
                let error =
                    SymExpr::binop(&mut cx, SymBinOp::Sub, dividend.clone(), rounded.clone());
                let order =
                    SymBoolExpr::cmp(&mut cx, SymCmpOp::Ule, rounded.clone(), dividend.clone());
                let remainder =
                    SymBoolExpr::cmp_word_const(&mut cx, SymCmpOp::Ult, &error, U256::from(37));
                for property in [order, remainder] {
                    for bounded in [true, false, true] {
                        let mut constraints = vec![property.clone().not(&mut cx)];
                        if bounded {
                            constraints.push(safe.clone());
                        }
                        for _ in 0..2 {
                            let normalized = normalize_constraints_for_solver_cached(
                                &mut cx,
                                &constraints,
                                &mut cache,
                            );
                            assert_eq!(
                                normalized,
                                vec![SymBoolExpr::constant(&mut cx, false)],
                                "bias={bias}, split={split}, bounded={bounded}"
                            );
                            constraints.reverse();
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn rounding_relations_bound_biased_dividends_at_word_boundaries() {
        let mut cx = SymCx::new();
        let value = SymExpr::var(&mut cx, "value");
        for divisor in [U256::from(3), U256::from(37), (U256::ONE << 255) - U256::ONE, U256::MAX] {
            for bias in [U256::ZERO, divisor / U256::from(2), divisor - U256::ONE] {
                let bound =
                    SymBoolExpr::cmp_word_const(&mut cx, SymCmpOp::Ule, &value, U256::MAX - bias);
                let context = ConstraintContext::new(&[bound]);
                let rounded = rounded(&mut cx, &value, divisor, bias, false);
                let relation = context.rounding_bounds(&rounded, None).unwrap_or_else(|| {
                    panic!(
                        "missing relation: divisor={divisor}, bias={bias}, expression={rounded:?}"
                    )
                });
                assert_eq!(relation.anchor, &value);
                let samples = (0..=255).map(U256::from).chain([
                    U256::ONE << 255,
                    U256::MAX - bias,
                    U256::MAX,
                ]);
                for input in samples.filter(|input| *input <= U256::MAX - bias) {
                    let mut model = SymbolicModel::default();
                    assert!(value.assign_model_value(&mut model, input));
                    let output = rounded.eval_model(&model).unwrap();
                    if output >= input {
                        assert!(output - input <= relation.above);
                    } else {
                        assert!(input - output <= relation.below);
                    }
                }
            }
        }
    }

    #[test]
    fn rounding_normalization_preserves_wrapping_and_signed_comparisons() {
        let mut cx = SymCx::new();
        let value = SymExpr::var(&mut cx, "value");
        let mut cache = HashMap::default();
        let safe =
            SymBoolExpr::cmp_word_const(&mut cx, SymCmpOp::Ule, &value, U256::MAX - U256::from(37));
        for bias in [U256::ZERO, U256::from(36)] {
            for split in [false, true] {
                let rounded = rounded(&mut cx, &value, U256::from(37), bias, split);
                let error = SymExpr::binop(&mut cx, SymBinOp::Sub, rounded.clone(), value.clone());
                for op in [
                    SymCmpOp::Eq,
                    SymCmpOp::Ult,
                    SymCmpOp::Ule,
                    SymCmpOp::Ugt,
                    SymCmpOp::Uge,
                    SymCmpOp::Slt,
                    SymCmpOp::Sgt,
                ] {
                    let comparison = SymBoolExpr::cmp(&mut cx, op, rounded.clone(), value.clone());
                    let error_bound =
                        SymBoolExpr::cmp_word_const(&mut cx, op, &error, U256::from(37));
                    for predicate in [comparison, error_bound] {
                        for predicate in [predicate.clone(), predicate.not(&mut cx)] {
                            for bounded in [true, false, true] {
                                let mut constraints = vec![predicate.clone()];
                                if bounded {
                                    constraints.push(safe.clone());
                                }
                                for _ in 0..2 {
                                    let normalized = normalize_constraints_for_solver_cached(
                                        &mut cx,
                                        &constraints,
                                        &mut cache,
                                    );
                                    for input in [
                                        U256::ZERO,
                                        U256::ONE,
                                        U256::from(36),
                                        U256::from(37),
                                        (U256::ONE << 255) - U256::ONE,
                                        U256::ONE << 255,
                                        U256::MAX - U256::from(37),
                                        U256::MAX - U256::ONE,
                                        U256::MAX,
                                    ] {
                                        let mut model = SymbolicModel::default();
                                        assert!(value.assign_model_value(&mut model, input));
                                        assert_eq!(
                                            constraints
                                                .iter()
                                                .all(|c| c.eval_model(&model).unwrap()),
                                            normalized
                                                .iter()
                                                .all(|c| c.eval_model(&model).unwrap()),
                                            "bias={bias}, split={split}, op={op:?}, bounded={bounded}, input={input}"
                                        );
                                    }
                                    constraints.reverse();
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn wrapped_dividend_bound_does_not_establish_rounding_safety() {
        let mut cx = SymCx::new();
        let value = SymExpr::var(&mut cx, "value");
        let rounded = rounded(&mut cx, &value, U256::from(37), U256::from(36), false);
        let (dividend, _) = ConstraintContext::rounded_product_operands(&rounded).unwrap();
        let bound = SymBoolExpr::cmp_word_const(&mut cx, SymCmpOp::Ule, dividend, U256::from(36));
        let order = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ult, rounded, value.clone());
        let constraints = vec![bound, order];
        let normalized = normalize_constraints_for_solver(&mut cx, &constraints);
        let mut model = SymbolicModel::default();
        assert!(value.assign_model_value(&mut model, U256::MAX));
        assert!(constraints.iter().all(|c| c.eval_model(&model).unwrap()));
        assert!(normalized.iter().all(|c| c.eval_model(&model).unwrap()));
    }
}
