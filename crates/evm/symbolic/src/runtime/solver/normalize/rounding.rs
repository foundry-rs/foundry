//! Rounding relations between a rounded multiple and its original dividend.
//!
//! These bounds describe mathematical differences, not wrapping EVM subtractions.
//! They are consumed only where the sign or a non-wrapping product is established.

use super::{
    ConstraintContext, HashMap, MAX_LOCAL_ANALYSIS_NODES, SymBinOp, SymBoolExpr, SymBoolExprKind,
    SymCmpOp, SymExpr, SymExprKind, U256, WordInterval, euclidean_sum_dividend,
    rounded_product_operands,
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

    /// Decides a comparison between a division result and one of its operands.
    ///
    /// With a nonzero divisor, `a % d < d` and `a / d <= a`. For every divisor, `a % d <= a` and
    /// `(a / d) * k <= a` once `k <= d`, and `x * y / k <= x` once `y <= k` and `x * y` cannot
    /// wrap. With a nonzero `y + z` that cannot wrap, `x * y / (y + z) <= x`, strictly for nonzero
    /// `x` and `z`. The Euclidean sum `(a / d) * d + a % d` is `a`, so each summand is at most the
    /// other's complement. With a nonzero divisor, `a / d <= (a - 1) / d + 1`, also when selected
    /// by `a != 0`, and `(x * y + c) / d <= x` once `y <= d`, `x * y` cannot wrap, and `c < d`. A
    /// weighted average `(p * u + q * v) / k` with `u + v <= k` is at most a bound on `p` and `q`.
    pub(super) fn division_comparison_value(&self, expr: &SymBoolExpr) -> Option<bool> {
        if let SymBoolExprKind::Not(inner) = expr.kind() {
            return self.division_comparison_value(inner).map(|value| !value);
        }
        let SymBoolExprKind::Cmp(op, left, right) = expr.kind() else { return None };
        let mut intervals = HashMap::default();
        let mut remaining = MAX_LOCAL_ANALYSIS_NODES;
        if let Some(strict) = self.division_order(left, right, &mut intervals, &mut remaining) {
            return match op {
                SymCmpOp::Ule => Some(true),
                SymCmpOp::Ugt => Some(false),
                SymCmpOp::Ult if strict => Some(true),
                SymCmpOp::Uge if strict => Some(false),
                _ => None,
            };
        }
        if let Some(strict) = self.division_order(right, left, &mut intervals, &mut remaining) {
            return match op {
                SymCmpOp::Uge => Some(true),
                SymCmpOp::Ult => Some(false),
                SymCmpOp::Ugt if strict => Some(true),
                SymCmpOp::Ule if strict => Some(false),
                _ => None,
            };
        }
        None
    }

    /// Returns whether a division shape proves `small <= large`, and whether the order is strict.
    ///
    /// Intervals share the caller's cache and node budget, so a chain of nested subtractions is
    /// analyzed once instead of once per level.
    fn division_order(
        &self,
        small: &SymExpr,
        large: &SymExpr,
        intervals: &mut HashMap<SymExpr, Option<WordInterval>>,
        remaining: &mut usize,
    ) -> Option<bool> {
        // The overflow rewrite turns a checked `a + b` into `~a < b` before `d != 0` resolves the
        // division guards, so a symbolic divisor's Euclidean sum is not folded to `x`. That sum is
        // `x` and cannot wrap, so `b <= ~a`.
        if let SymExprKind::Not(base) = large.kind()
            && euclidean_sum_dividend(base, small).is_some()
        {
            return Some(false);
        }
        if let Some((dividend, divisor)) = small.udiv_operands()
            && self.weighted_within(dividend, divisor, large, intervals, remaining)
        {
            return Some(false);
        }
        // Division comparisons are rewritten to `(b + 1) * k <= sum` before facts apply.
        if let SymExprKind::BinOp(SymBinOp::Mul, x, y) = large.kind() {
            for (multiplier, divisor) in [(x, y), (y, x)] {
                if self.weighted_within(small, divisor, multiplier, intervals, remaining) {
                    return Some(false);
                }
                // Strict once `b + 1` and `(b + 1) * k` do not wrap.
                if let SymExprKind::BinOp(SymBinOp::Add, bound, one) = multiplier.kind()
                    && one.as_const() == Some(U256::ONE)
                    && self.weighted_within(small, divisor, bound, intervals, remaining)
                    && self
                        .interval_cached(bound, intervals, remaining)
                        .is_some_and(|bound| bound.max < U256::MAX)
                    && self.mul_cannot_overflow_256(multiplier, divisor)
                {
                    return Some(true);
                }
            }
        }
        let mut interval = |expr: &SymExpr| self.interval_cached(expr, intervals, remaining);
        let nonzero = |range: Option<WordInterval>| range.is_some_and(|range| !range.min.is_zero());
        match small.kind() {
            SymExprKind::BinOp(SymBinOp::URem, dividend, divisor) => {
                if dividend == large {
                    return Some(false);
                }
                (divisor == large && nonzero(interval(divisor))).then_some(true)
            }
            SymExprKind::BinOp(SymBinOp::UDiv, dividend, divisor) => {
                if dividend == large {
                    return nonzero(interval(divisor)).then_some(false);
                }
                // `x / d <= (x - 1) / d + 1` for `d >= 1`, as in OpenZeppelin's `ceilDiv`: for
                // `x >= 1` the quotients differ by at most one and the sum cannot wrap, and
                // `0 / d` is zero. SMT's `x / 0` is all ones, so `d` must be nonzero.
                let ceiling = match large.kind() {
                    // OpenZeppelin v5 selects zero for `x == 0`, where `x / d` is zero too.
                    SymExprKind::Ite(condition, value, zero)
                        if zero.as_const() == Some(U256::ZERO)
                            && let SymBoolExprKind::Not(inner) = condition.kind()
                            && inner.zero_check_operand() == Some(dividend) =>
                    {
                        value
                    }
                    _ => large,
                };
                if let SymExprKind::BinOp(SymBinOp::Add, quotient, increment) = ceiling.kind()
                    && increment.as_const() == Some(U256::ONE)
                    && let Some((predecessor, other)) = quotient.udiv_operands()
                    && other == divisor
                    && let SymExprKind::BinOp(SymBinOp::Sub, value, decrement) = predecessor.kind()
                    && decrement.as_const() == Some(U256::ONE)
                    && value == dividend
                {
                    return nonzero(interval(divisor)).then_some(false);
                }
                // When `y + z` is nonzero and does not wrap, `x * y / (y + z) <= x`, strictly once
                // `x` and `z` are nonzero, since `x * y < x * (y + z)`. A wrapped product only
                // shrinks the dividend. `UDiv` by zero is all ones, so the divisor must be nonzero.
                if let SymExprKind::BinOp(SymBinOp::Mul, left, right) = dividend.kind()
                    && let SymExprKind::BinOp(SymBinOp::Add, a, b) = divisor.kind()
                    && let Some(other) = [(left, right), (right, left)]
                        .into_iter()
                        .find_map(|(x, y)| (x == large).then_some(y))
                    && let Some(rest) =
                        [(a, b), (b, a)].into_iter().find_map(|(y, z)| (y == other).then_some(z))
                    && let Some(other) = interval(other)
                    && let Some(rest) = interval(rest)
                    && other.max.checked_add(rest.max).is_some()
                    && nonzero(interval(divisor))
                {
                    return Some(nonzero(interval(large)) && !rest.min.is_zero());
                }
                // Rounding up adds at most `d - 1`, so `(x * y + c) / d <= x` still holds once
                // `c <= d - 1`; a wrapped sum only shrinks the dividend.
                let mut bounded = |dividend: &SymExpr, addend: Option<&SymExpr>| {
                    let SymExprKind::BinOp(SymBinOp::Mul, left, right) = dividend.kind() else {
                        return false;
                    };
                    let other = if left == large {
                        right
                    } else if right == large {
                        left
                    } else {
                        return false;
                    };
                    let Some(range) = interval(divisor).filter(|range| !range.min.is_zero()) else {
                        return false;
                    };
                    addend.is_none_or(|addend| {
                        matches!(addend.kind(), SymExprKind::BinOp(SymBinOp::Sub, value, one)
                            if value == divisor && one.as_const() == Some(U256::ONE))
                            || interval(addend).is_some_and(|addend| addend.max < range.min)
                    }) && (other == divisor
                        || interval(other).is_some_and(|other| other.max <= range.min)
                        || self.asserts_less_or_equal(other, divisor))
                        && self.mul_cannot_overflow_256(left, right)
                };
                // `round_down(n, t) <= n` (zero when `t == 0`), so dividing the rounded product
                // is bounded by `n / d`.
                let peeled = match dividend.kind() {
                    SymExprKind::BinOp(SymBinOp::Mul, left, right) => {
                        rounded_product_operands(left, right).map(|(n, _)| n)
                    }
                    _ => None,
                };
                let rounded_up = match dividend.kind() {
                    SymExprKind::BinOp(SymBinOp::Add, a, b) => [(a, b), (b, a)]
                        .into_iter()
                        .any(|(product, addend)| bounded(product, Some(addend))),
                    // `(x * y + d) - 1`. A wrapped sum is smaller, and a sum that wraps to zero
                    // gives `MAX == x * y + d - 1` exactly.
                    SymExprKind::BinOp(SymBinOp::Sub, sum, one)
                        if one.as_const() == Some(U256::ONE)
                            && let SymExprKind::BinOp(SymBinOp::Add, a, b) = sum.kind()
                            && let Some(product) =
                                (a == divisor).then_some(b).or((b == divisor).then_some(a)) =>
                    {
                        bounded(product, None)
                    }
                    _ => false,
                };
                (rounded_up
                    || peeled.is_some_and(|peeled| bounded(peeled, None))
                    || bounded(dividend, None))
                .then_some(false)
            }
            // `(x / d) * k <= (x / d) * d <= x` once `k <= d`, so the product cannot wrap; a zero
            // `d` forces a zero product.
            SymExprKind::BinOp(SymBinOp::Mul, left, right) => [(left, right), (right, left)]
                .into_iter()
                .any(|(quotient, factor)| {
                    quotient.udiv_operands().is_some_and(|(dividend, divisor)| {
                        dividend == large
                            && (factor == divisor
                                || interval(factor)
                                    .zip(interval(divisor))
                                    .is_some_and(|(factor, divisor)| factor.max <= divisor.min))
                    })
                })
                .then_some(false),
            _ => None,
        }
    }

    /// Returns whether `p * u + q * v <= b * k`, so `(p * u + q * v) / k <= b`, as in a weighted
    /// average or an EMA: both multipliers are at most `b`, the weights satisfy `u + v <= k`, `k`
    /// is nonzero, and `b * k` does not wrap. Each product is then at most `b * k`, so neither the
    /// products nor the sum wrap. With `v = k - u`, possibly masked, `u <= k` keeps the complement
    /// exact; either weight may be the complement.
    fn weighted_within(
        &self,
        dividend: &SymExpr,
        divisor: &SymExpr,
        bound: &SymExpr,
        intervals: &mut HashMap<SymExpr, Option<WordInterval>>,
        remaining: &mut usize,
    ) -> bool {
        let SymExprKind::BinOp(SymBinOp::Add, first, second) = dividend.kind() else {
            return false;
        };
        let (SymExprKind::BinOp(SymBinOp::Mul, p1, u1), SymExprKind::BinOp(SymBinOp::Mul, q1, v1)) =
            (first.kind(), second.kind())
        else {
            return false;
        };
        let mut interval = |expr: &SymExpr| self.interval_cached(expr, intervals, remaining);
        let (Some(k), Some(b)) = (interval(divisor), interval(bound)) else {
            return false;
        };
        if k.min.is_zero() || !self.mul_cannot_overflow_256(bound, divisor) {
            return false;
        }
        let complement = |u: &SymExpr, v: &SymExpr| {
            let unmasked = match v.kind() {
                SymExprKind::BinOp(SymBinOp::And, value, mask) if mask.as_const().is_some() => {
                    value
                }
                _ => v,
            };
            matches!(unmasked.kind(), SymExprKind::BinOp(SymBinOp::Sub, total, part)
                if total == divisor && part == u)
        };
        [(p1, u1, q1, v1), (u1, p1, q1, v1), (p1, u1, v1, q1), (u1, p1, v1, q1)].into_iter().any(
            |(p, u, q, v)| {
                [p, q].into_iter().all(|multiplier| {
                    multiplier == bound
                        || self.asserts_less_or_equal(multiplier, bound)
                        || interval(multiplier).is_some_and(|m| m.max <= b.min)
                }) && ([(u, v), (v, u)].into_iter().any(|(u, v)| {
                    complement(u, v)
                        && (self.asserts_less_or_equal(u, divisor)
                            || interval(u).is_some_and(|u| u.max <= k.min))
                }) || interval(u)
                    .zip(interval(v))
                    .and_then(|(u, v)| u.max.checked_add(v.max))
                    .is_some_and(|sum| sum <= k.min))
            },
        )
    }

    /// Returns whether another conjunct asserts `small <= large`, including strict `<`.
    fn asserts_less_or_equal(&self, small: &SymExpr, large: &SymExpr) -> bool {
        self.facts.iter().any(|fact| match fact.kind() {
            SymBoolExprKind::Cmp(SymCmpOp::Ult | SymCmpOp::Ule, left, right) => {
                left == small && right == large
            }
            SymBoolExprKind::Not(inner) => matches!(
                inner.kind(),
                SymBoolExprKind::Cmp(SymCmpOp::Ult, left, right) if left == large && right == small
            ),
            _ => false,
        })
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
        // A division shape such as `d - x % d` proves `right <= left`.
        if self.division_order(right, left, intervals, remaining).is_some()
            && let Some(left) = self.interval_cached(left, intervals, remaining)
        {
            return Some(WordInterval { min: U256::ZERO, max: left.max });
        }
        None
    }
}
