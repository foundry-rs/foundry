//! Rounding relations between a rounded multiple and its original dividend.
//!
//! These bounds describe mathematical differences, not wrapping EVM subtractions.
//! They are consumed only where the sign or a non-wrapping product is established.

use super::{
    ConstraintContext, HashMap, MAX_LOCAL_ANALYSIS_NODES, SymBinOp, SymBoolExpr, SymBoolExprKind,
    SymCmpOp, SymExpr, SymExprKind, U256, WordInterval, euclidean_sum_dividend,
    word_division_operands,
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
    /// by `a != 0`, and `(x * y + c) / d <= x` once `y <= d`, `x * y` cannot wrap, and `c < d`.
    /// Another conjunct's cross-multiplied `x * k <= y * l` also decides `x <= y` once `k` is
    /// positive, `l <= k`, and `x * k` cannot wrap.
    /// The shares `w * a / k + w * b / k <= w` once `a + b <= k`, and `n <= ceil(n / d) * d` for a
    /// nonzero `d` once the multiple cannot wrap.
    /// A weighted average `(p * u + q * v) / k` with `u + v <= k` is at most a bound on `p` and
    /// `q`.
    pub(super) fn division_comparison_value(&self, expr: &SymBoolExpr) -> Option<bool> {
        if let SymBoolExprKind::Not(inner) = expr.kind() {
            return self.division_comparison_value(inner).map(|value| !value);
        }
        let SymBoolExprKind::Cmp(op, left, right) = expr.kind() else { return None };
        let mut intervals = HashMap::default();
        let mut remaining = MAX_LOCAL_ANALYSIS_NODES;
        let mut order = |small, large| {
            self.division_order(small, large, &mut intervals, &mut remaining).or_else(|| {
                self.asserts_scaled_order(small, large, &mut intervals, &mut remaining)
                    .then_some(false)
            })
        };
        if let Some(strict) = order(left, right) {
            return match op {
                SymCmpOp::Ule => Some(true),
                SymCmpOp::Ugt => Some(false),
                SymCmpOp::Ult if strict => Some(true),
                SymCmpOp::Uge if strict => Some(false),
                _ => None,
            };
        }
        if let Some(strict) = order(right, left) {
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
        // `x` and cannot wrap, so `b <= ~a`. Two shares of one whole cannot wrap either.
        if let SymExprKind::Not(base) = large.kind()
            && (euclidean_sum_dividend(base, small).is_some()
                || self.shares_whole(base, small, None, intervals, remaining).is_some())
        {
            return Some(false);
        }
        // Two shares sum to at most their whole.
        if let SymExprKind::BinOp(SymBinOp::Add, first, second) = small.kind()
            && self.shares_whole(first, second, Some(large), intervals, remaining).is_some()
        {
            return Some(false);
        }
        if let Some((dividend, divisor)) = small.udiv_operands()
            && self.weighted_within(dividend, divisor, large, intervals, remaining)
        {
            return Some(false);
        }
        // The same sum compared with `(b + 1) * k`, as written in the source or as
        // `normalize_udiv_comparison` cross-multiplies the quotient form, is at most `b * k`, so
        // strictly below it once `b + 1` and that product do not wrap.
        if let SymExprKind::BinOp(SymBinOp::Mul, x, y) = large.kind()
            && [(x, y), (y, x)].into_iter().any(|(multiplier, divisor)| {
                matches!(multiplier.kind(), SymExprKind::BinOp(SymBinOp::Add, bound, one)
                    if one.as_const() == Some(U256::ONE)
                        && self
                            .interval_cached(bound, intervals, remaining)
                            .is_some_and(|bound| bound.max < U256::MAX)
                        && self.mul_cannot_overflow_256_cached(
                            multiplier, divisor, intervals, remaining,
                        )
                        && self.weighted_within(small, divisor, bound, intervals, remaining))
            })
        {
            return Some(true);
        }
        // `n <= ceil(n / d) * d` for a nonzero `d`, rounding up as `(n + d - 1) / d` or as
        // OpenZeppelin's `ceilDiv`. Over the integers the multiple lies in `[n, n + d - 1]`, so it
        // cannot wrap once `n + d` does not, from value ranges or a checked addition's `d <= ~n`
        // guard. A checked product also covers `ceilDiv`, whose zero `n` makes the bound trivial.
        if let SymExprKind::BinOp(SymBinOp::Mul, left, right) = large.kind()
            && let Some(divisor) =
                [(left, right), (right, left)].into_iter().find_map(|(ceiling, divisor)| {
                    let fits = if ceil_div_operands(ceiling) == Some((small, divisor)) {
                        self.mul_cannot_overflow_256_cached(ceiling, divisor, intervals, remaining)
                            || self.sum_cannot_wrap(small, divisor, intervals, remaining)
                    } else {
                        ceil_sum_operands(ceiling) == Some((small, divisor))
                            && self.sum_cannot_wrap(small, divisor, intervals, remaining)
                    };
                    fits.then_some(divisor)
                })
            && self
                .interval_cached(divisor, intervals, remaining)
                .is_some_and(|range| !range.min.is_zero())
        {
            return Some(false);
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
                if ceil_div_operands(large) == Some((dividend, divisor)) {
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
                // is bounded by `n / d`. The EVM `DIV` may still carry its `t == 0` guard.
                let peeled = match dividend.kind() {
                    SymExprKind::BinOp(SymBinOp::Mul, left, right) => {
                        [(left, right), (right, left)].into_iter().find_map(|(quotient, factor)| {
                            word_division_operands(quotient)
                                .and_then(|(n, t)| (t == factor).then_some(n))
                        })
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

    /// Returns whether another conjunct asserts `small * k <= large * l` with `l <= k` and a
    /// positive `k`, as in a cross-multiplied collateral check. When `small * k` does not wrap,
    /// `small > large` would give `small * k > large * k >= large * l`, using `k > 0` and
    /// `l <= k`, and a wrapped `large * l` is only smaller. The reverse, deciding a scaled fact
    /// from `small <= large`, must never remove a conjunct, or the two could justify each other.
    fn asserts_scaled_order(
        &self,
        small: &SymExpr,
        large: &SymExpr,
        intervals: &mut HashMap<SymExpr, Option<WordInterval>>,
        remaining: &mut usize,
    ) -> bool {
        let factor = |product: &'_ SymExpr, operand: &SymExpr| match product.kind() {
            SymExprKind::BinOp(SymBinOp::Mul, left, right) if left == operand => {
                Some(right.clone())
            }
            SymExprKind::BinOp(SymBinOp::Mul, left, right) if right == operand => {
                Some(left.clone())
            }
            _ => None,
        };
        self.facts.iter().any(|fact| {
            let (scaled_small, scaled_large) = match fact.kind() {
                SymBoolExprKind::Cmp(SymCmpOp::Ult | SymCmpOp::Ule, left, right) => (left, right),
                SymBoolExprKind::Not(inner) => match inner.kind() {
                    SymBoolExprKind::Cmp(SymCmpOp::Ult, left, right) => (right, left),
                    _ => return false,
                },
                _ => return false,
            };
            let (Some(k), Some(l)) = (factor(scaled_small, small), factor(scaled_large, large))
            else {
                return false;
            };
            self.interval_cached(&k, intervals, remaining)
                .zip(self.interval_cached(&l, intervals, remaining))
                .is_some_and(|(k, l)| !k.min.is_zero() && l.max <= k.min)
                && self.mul_cannot_overflow_256(small, &k)
        })
    }

    /// Returns `w` for the shares `w * a / k` and `w * b / k` of a nonzero `k` once `a + b <= k`,
    /// only matching `whole` when given. Their floors sum to at most `w * (a + b) / k <= w`, so the
    /// sum cannot wrap, and a wrapped product only shrinks its share.
    fn shares_whole<'a>(
        &self,
        first: &'a SymExpr,
        second: &'a SymExpr,
        whole: Option<&SymExpr>,
        intervals: &mut HashMap<SymExpr, Option<WordInterval>>,
        remaining: &mut usize,
    ) -> Option<&'a SymExpr> {
        let (first, divisor) = first.udiv_operands()?;
        let (second, other_divisor) = second.udiv_operands()?;
        if divisor != other_divisor {
            return None;
        }
        let (SymExprKind::BinOp(SymBinOp::Mul, w1, a1), SymExprKind::BinOp(SymBinOp::Mul, w2, b2)) =
            (first.kind(), second.kind())
        else {
            return None;
        };
        let (found, a, b) =
            [(w1, a1, w2, b2), (w1, a1, b2, w2), (a1, w1, w2, b2), (a1, w1, b2, w2)]
                .into_iter()
                .find_map(|(found, a, other, b)| {
                    (found == other && whole.is_none_or(|whole| whole == found))
                        .then_some((found, a, b))
                })?;
        let min = self.interval_cached(divisor, intervals, remaining)?.min;
        if min.is_zero() {
            return None;
        }
        self.weights_fit(a, b, divisor, min, intervals, remaining).then_some(found)
    }

    /// Returns whether `a + b <= k` over the integers for a divisor `k` at least `min`: from `b`
    /// being `k - a`, possibly masked, with `a <= k`, either way round, from value ranges, or from
    /// `k` being `a + b` or an asserted `a + b <= k` once the sum cannot wrap.
    fn weights_fit(
        &self,
        a: &SymExpr,
        b: &SymExpr,
        divisor: &SymExpr,
        min: U256,
        intervals: &mut HashMap<SymExpr, Option<WordInterval>>,
        remaining: &mut usize,
    ) -> bool {
        // `a <= k` keeps `k - a` exact, and a mask only lowers it.
        let difference = |u: &SymExpr, v: &SymExpr| {
            let unmasked = match v.kind() {
                SymExprKind::BinOp(SymBinOp::And, value, mask) if mask.as_const().is_some() => {
                    value
                }
                _ => v,
            };
            matches!(unmasked.kind(), SymExprKind::BinOp(SymBinOp::Sub, total, part)
                if total == divisor && part == u)
        };
        if [(a, b), (b, a)].into_iter().any(|(u, v)| {
            difference(u, v) && self.factor_within_divisor(u, divisor, intervals, remaining)
        }) {
            return true;
        }
        // Whether another conjunct asserts `x <= y`, including strict `<`.
        let asserted = |accept: &dyn Fn(&SymExpr, &SymExpr) -> bool| {
            self.facts.iter().any(|fact| match fact.kind() {
                SymBoolExprKind::Cmp(SymCmpOp::Ult | SymCmpOp::Ule, left, right) => {
                    accept(left, right)
                }
                SymBoolExprKind::Not(inner) => matches!(
                    inner.kind(),
                    SymBoolExprKind::Cmp(SymCmpOp::Ult, left, right) if accept(right, left)
                ),
                _ => false,
            })
        };
        // `a + b` must not wrap for `a + b <= k` to hold over the integers: from value ranges, or
        // from a checked addition's `b <= ~a` guard, which older compilers write as `b <= MAX - a`.
        let sum_max = self
            .interval_cached(a, intervals, remaining)
            .and_then(|a| a.max.checked_add(self.interval_cached(b, intervals, remaining)?.max));
        let complement = |x: &SymExpr, y: &SymExpr| match y.kind() {
            SymExprKind::Not(inner) => inner == x,
            SymExprKind::BinOp(SymBinOp::Sub, max, inner) => {
                max.as_const() == Some(U256::MAX) && inner == x
            }
            _ => false,
        };
        if sum_max.is_none()
            && !asserted(&|x, y| (x == b && complement(a, y)) || (x == a && complement(b, y)))
        {
            return false;
        }
        let is_sum = |expr: &SymExpr| {
            matches!(expr.kind(), SymExprKind::BinOp(SymBinOp::Add, x, y)
                if (x == a && y == b) || (x == b && y == a))
        };
        sum_max.is_some_and(|max| max <= min)
            || is_sum(divisor)
            || asserted(&|x, y| is_sum(x) && y == divisor)
    }

    /// Returns whether `(p * u + q * v) / k <= b`, as in a weighted average or an EMA: both
    /// multipliers are at most `b` and the weights fit in a nonzero `k`. Over the integers the
    /// dividend is then at most `b * k`, and a wrapped product or sum only lowers it. Once `b * k`
    /// wraps, every word divided by `k` is at most `b`, so callers that compare the dividend with a
    /// multiple of `k` must rule out that wrap themselves.
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
        let Some(k_min) = self.interval_cached(divisor, intervals, remaining).map(|k| k.min) else {
            return false;
        };
        if k_min.is_zero() {
            return false;
        }
        [(p1, u1, q1, v1), (u1, p1, q1, v1), (p1, u1, v1, q1), (u1, p1, v1, q1)].into_iter().any(
            |(p, u, q, v)| {
                self.factor_within_divisor(p, bound, intervals, remaining)
                    && self.factor_within_divisor(q, bound, intervals, remaining)
                    && self.weights_fit(u, v, divisor, k_min, intervals, remaining)
            },
        )
    }

    /// Returns whether `a + b` cannot wrap, from value ranges or a checked addition's `b <= ~a`
    /// guard.
    fn sum_cannot_wrap(
        &self,
        a: &SymExpr,
        b: &SymExpr,
        intervals: &mut HashMap<SymExpr, Option<WordInterval>>,
        remaining: &mut usize,
    ) -> bool {
        let complement = |x: &SymExpr, y: &SymExpr| {
            self.asserted_upper_bounds(x)
                .iter()
                .any(|bound| matches!(bound.kind(), SymExprKind::Not(inner) if inner == y))
        };
        self.interval_cached(a, intervals, remaining)
            .zip(self.interval_cached(b, intervals, remaining))
            .is_some_and(|(a, b)| a.max.checked_add(b.max).is_some())
            || complement(a, b)
            || complement(b, a)
    }

    /// Returns whether other conjuncts assert `small <= large`, including strict `<`. An elapsed
    /// `t - s` is at most `d` once `s <= t` and `t <= d + s`. A wrapped `d + s` would be below
    /// `s <= t`, so the two facts cannot both hold.
    pub(super) fn asserts_less_or_equal(&self, small: &SymExpr, large: &SymExpr) -> bool {
        if self.asserted_upper_bounds(small).contains(large) {
            return true;
        }
        let SymExprKind::BinOp(SymBinOp::Sub, end, start) = small.kind() else { return false };
        self.asserted_upper_bounds(start).contains(end)
            && self.asserted_upper_bounds(end).iter().any(|sum| {
                matches!(sum.kind(), SymExprKind::BinOp(SymBinOp::Add, a, b)
                    if (a == large && b == start) || (a == start && b == large))
            })
    }

    /// Returns whether `factor <= divisor`, by equality, another conjunct, or value ranges.
    fn factor_within_divisor(
        &self,
        factor: &SymExpr,
        divisor: &SymExpr,
        intervals: &mut HashMap<SymExpr, Option<WordInterval>>,
        remaining: &mut usize,
    ) -> bool {
        factor == divisor
            || self.asserts_less_or_equal(factor, divisor)
            || self
                .interval_cached(factor, intervals, remaining)
                .zip(self.interval_cached(divisor, intervals, remaining))
                .is_some_and(|(factor, divisor)| factor.max <= divisor.min)
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

/// Returns `(n, d)` for OpenZeppelin's `ceilDiv`, `(n - 1) / d + 1`. v5 selects zero for `n == 0`.
fn ceil_div_operands(ceiling: &SymExpr) -> Option<(&SymExpr, &SymExpr)> {
    let (ceiling, selector) = match ceiling.kind() {
        SymExprKind::Ite(condition, value, zero)
            if zero.as_const() == Some(U256::ZERO)
                && let SymBoolExprKind::Not(inner) = condition.kind()
                && let Some(selector) = inner.zero_check_operand() =>
        {
            (value, Some(selector))
        }
        _ => (ceiling, None),
    };
    let SymExprKind::BinOp(SymBinOp::Add, quotient, increment) = ceiling.kind() else {
        return None;
    };
    let (predecessor, divisor) = quotient.udiv_operands()?;
    let SymExprKind::BinOp(SymBinOp::Sub, value, decrement) = predecessor.kind() else {
        return None;
    };
    (increment.as_const() == Some(U256::ONE)
        && decrement.as_const() == Some(U256::ONE)
        && selector.is_none_or(|selector| selector == value))
    .then_some((value, divisor))
}

/// Returns `(n, d)` for `n` rounded up as `(n + d - 1) / d`.
fn ceil_sum_operands(ceiling: &SymExpr) -> Option<(&SymExpr, &SymExpr)> {
    let (numerator, divisor) = ceiling.udiv_operands()?;
    let SymExprKind::BinOp(SymBinOp::Sub, sum, decrement) = numerator.kind() else { return None };
    let SymExprKind::BinOp(SymBinOp::Add, a, b) = sum.kind() else { return None };
    let n = if b == divisor {
        a
    } else if a == divisor {
        b
    } else {
        return None;
    };
    (decrement.as_const() == Some(U256::ONE)).then_some((n, divisor))
}
