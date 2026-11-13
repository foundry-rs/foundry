//! Recognition of the 512-bit product high word and of the full-precision `mulDiv` quotient.

use super::*;

/// Newton steps that lift the `(3 * d) ^ 2` seed, an inverse modulo `2**4`, to `2**256`.
const MIN_NEWTON_STEPS: usize = 6;

/// Returns `(a, b)` when `expr` computes the high word of `a * b` from `mulmod(a, b, MAX)`.
pub(super) fn mul_high_operands(expr: &SymExpr) -> Option<(&SymExpr, &SymExpr)> {
    let SymExprKind::BinOp(SymBinOp::Sub, minuend, subtrahend) = expr.kind() else {
        return None;
    };
    // `(mm - lo) - lt(mm, lo)`.
    if let SymExprKind::BinOp(SymBinOp::Sub, mm, lo) = minuend.kind()
        && is_less_than_word(subtrahend, mm, lo)
    {
        return wide_product_operands(mm, lo);
    }
    // `mm - (lo + lt(mm, lo))`, whose sum is built as `ite(mm < lo, lo + 1, lo)`.
    match subtrahend.kind() {
        SymExprKind::Ite(condition, incremented, lo)
            if is_less_than(condition, minuend, lo) && is_increment_of(incremented, lo) =>
        {
            wide_product_operands(minuend, lo)
        }
        SymExprKind::BinOp(SymBinOp::Add, left, right) => [(left, right), (right, left)]
            .into_iter()
            .find(|(lo, borrow)| is_less_than_word(borrow, minuend, lo))
            .and_then(|(lo, _)| wide_product_operands(minuend, lo)),
        _ => None,
    }
}

/// Returns `(a, b, d)` when `expr` is the 512-by-256-bit `mulDiv(a, b, d)` quotient.
///
/// The quotient equals `floor(a * b / d)` whenever `mul_high(a, b) < d`.
pub(super) fn full_mul_div_operands(expr: &SymExpr) -> Option<(&SymExpr, &SymExpr, &SymExpr)> {
    let SymExprKind::BinOp(SymBinOp::Mul, left, right) = expr.kind() else {
        return None;
    };
    [(left, right), (right, left)].into_iter().find_map(|(shifted, inverse)| {
        let odd = newton_inverse_operand(inverse)?;
        let (denominator, twos) = word_division_operands(odd)?;
        if !is_lowest_set_bit(twos, denominator) {
            return None;
        }
        exact_shifted_product_operands(shifted, denominator, twos)
    })
}

impl ConstraintContext {
    /// Replaces a recognized `mulDiv` quotient by its exact value once the context proves
    /// `mul_high(a, b) < d`.
    pub(super) fn full_mul_div(&self, cx: &mut SymCx, expr: &SymExpr) -> Option<SymExpr> {
        let (left, right, denominator) = full_mul_div_operands(expr)?;
        let high = SymExpr::binop(cx, SymBinOp::MulHigh, left.clone(), right.clone());
        let below = SymBoolExpr::cmp(cx, SymCmpOp::Ult, high.clone(), denominator.clone());
        let not_above = SymBoolExpr::cmp(cx, SymCmpOp::Ule, denominator.clone(), high).not(cx);
        // `mul_high(a, d) < d` holds exactly when `d != 0`.
        let factor_below = (denominator == left || denominator == right)
            && self.interval(denominator).is_some_and(|interval| !interval.min.is_zero());
        (self.facts.contains(&below) || self.facts.contains(&not_above) || factor_below).then(
            || {
                SymExpr::ternop(
                    cx,
                    SymTernOp::MulDiv,
                    left.clone(),
                    right.clone(),
                    denominator.clone(),
                )
            },
        )
    }

    /// Records `mul_high(a, b) == 0` as a non-wrapping product.
    pub(super) fn record_zero_mul_high(&mut self, constraint: &SymBoolExpr) {
        if let Some(word) = constraint.zero_check_operand()
            && let SymExprKind::BinOp(SymBinOp::MulHigh, left, right) = word.kind()
        {
            self.non_wrapping_products.insert((left.clone(), right.clone()));
        }
    }
}

/// Decides comparisons between `mul_high(a, b)` and one of its factors.
pub(super) fn normalize_mul_high_factor_cmp(
    cx: &mut SymCx,
    op: SymCmpOp,
    left: &SymExpr,
    right: &SymExpr,
) -> Option<SymBoolExpr> {
    let (small, large, strict) = match op {
        SymCmpOp::Ult => (left, right, true),
        SymCmpOp::Ugt => (right, left, true),
        SymCmpOp::Ule => (left, right, false),
        SymCmpOp::Uge => (right, left, false),
        SymCmpOp::Eq | SymCmpOp::Slt | SymCmpOp::Sgt => return None,
    };
    // `a * b < 2**256 * b`, so `mul_high(a, b) < b` exactly when `b != 0`.
    if is_mul_high_factor(small, large) {
        return Some(if strict {
            SymBoolExpr::eq_word_const(cx, large, U256::ZERO).not(cx)
        } else {
            SymBoolExpr::constant(cx, true)
        });
    }
    if is_mul_high_factor(large, small) {
        return Some(if strict {
            SymBoolExpr::constant(cx, false)
        } else {
            SymBoolExpr::eq_word_const(cx, small, U256::ZERO)
        });
    }
    None
}

/// Rewrites `q + c == k` and `!q == k` for a word quotient `q` to `q == k'`, where a division
/// reaches `MAX` only as `MAX / 1`.
pub(super) fn normalize_quotient_offset_eq(
    cx: &mut SymCx,
    left: &SymExpr,
    right: &SymExpr,
) -> Option<SymBoolExpr> {
    let mut target = right.as_const()?;
    let mut quotient = left;
    loop {
        match quotient.kind() {
            SymExprKind::BinOp(SymBinOp::Add, value, offset)
                if let Some(offset) = offset.as_const() =>
            {
                target = target.wrapping_sub(offset);
                quotient = value;
            }
            SymExprKind::Not(value) => {
                target = !target;
                quotient = value;
            }
            _ => break,
        }
    }
    if let Some((numerator, denominator)) = word_division_operands(quotient)
        && target == U256::MAX
    {
        let unit = SymBoolExpr::eq_word_const(cx, denominator, U256::ONE);
        let max = SymBoolExpr::eq_word_const(cx, numerator, U256::MAX);
        return Some(SymBoolExpr::and(cx, vec![unit, max]));
    }
    let is_quotient = word_division_operands(quotient).is_some()
        || matches!(quotient.kind(), SymExprKind::TernOp(SymTernOp::MulDiv, ..));
    (is_quotient && quotient != left).then(|| SymBoolExpr::eq_word_const(cx, quotient, target))
}

/// Returns `(x, y)` when `word` is the overflow check `x == 0 || (x * y) / x == y`, which holds
/// exactly when `mul_high(x, y) == 0`.
pub(super) fn checked_product_guard_operands(word: &SymExpr) -> Option<(&SymExpr, &SymExpr)> {
    let SymExprKind::BinOp(SymBinOp::Or, left, right) = word.kind() else { return None };
    [(left, right), (right, left)].into_iter().find_map(|(zero_word, quotient_word)| {
        let zero_check = bool_word_condition(zero_word)?;
        let quotient_check = bool_word_condition(quotient_word)?;
        let factor = zero_check.zero_check_operand()?;
        let SymBoolExprKind::Cmp(SymCmpOp::Eq, a, b) = quotient_check.kind() else {
            return None;
        };
        [(a, b), (b, a)].into_iter().find_map(|(quotient, other)| {
            let (product, divisor) = word_division_operands(quotient)?;
            (divisor == factor && is_product_of(product, factor, other)).then_some((factor, other))
        })
    })
}

fn bool_word_condition(word: &SymExpr) -> Option<&SymBoolExpr> {
    match word.kind() {
        SymExprKind::Ite(condition, one, zero)
            if one.as_const() == Some(U256::ONE) && zero.as_const() == Some(U256::ZERO) =>
        {
            Some(condition)
        }
        _ => None,
    }
}

fn is_mul_high_factor(high: &SymExpr, factor: &SymExpr) -> bool {
    matches!(
        high.kind(),
        SymExprKind::BinOp(SymBinOp::MulHigh, left, right) if left == factor || right == factor
    )
}

fn wide_product_operands<'a>(mm: &'a SymExpr, lo: &SymExpr) -> Option<(&'a SymExpr, &'a SymExpr)> {
    let SymExprKind::TernOp(SymTernOp::MulMod, left, right, modulus) = mm.kind() else {
        return None;
    };
    (modulus.as_const() == Some(U256::MAX) && is_product_of(lo, left, right))
        .then_some((left, right))
}

fn is_product_of(expr: &SymExpr, left: &SymExpr, right: &SymExpr) -> bool {
    matches!(
        expr.kind(),
        SymExprKind::BinOp(SymBinOp::Mul, a, b)
            if (a == left && b == right) || (a == right && b == left)
    )
}

fn is_less_than_word(word: &SymExpr, left: &SymExpr, right: &SymExpr) -> bool {
    matches!(
        word.kind(),
        SymExprKind::Ite(condition, one, zero)
            if one.as_const() == Some(U256::ONE)
                && zero.as_const() == Some(U256::ZERO)
                && is_less_than(condition, left, right)
    )
}

fn is_less_than(condition: &SymBoolExpr, left: &SymExpr, right: &SymExpr) -> bool {
    match condition.kind() {
        SymBoolExprKind::Cmp(SymCmpOp::Ult, a, b) => a == left && b == right,
        SymBoolExprKind::Cmp(SymCmpOp::Ugt, a, b) => a == right && b == left,
        _ => false,
    }
}

fn is_increment_of(expr: &SymExpr, value: &SymExpr) -> bool {
    matches!(
        expr.kind(),
        SymExprKind::BinOp(SymBinOp::Add, a, one) if a == value && one.as_const() == Some(U256::ONE)
    )
}

/// Returns `d` when `expr` is at least [`MIN_NEWTON_STEPS`] of `inv * (2 - d * inv)` over the
/// `(3 * d) ^ 2` seed, the inverse of an odd `d` modulo `2**256`.
fn newton_inverse_operand(mut expr: &SymExpr) -> Option<&SymExpr> {
    let mut odd = None;
    let mut steps = 0;
    loop {
        match expr.kind() {
            SymExprKind::BinOp(SymBinOp::Xor, seed, two)
                if two.as_const() == Some(U256::from(2)) =>
            {
                let SymExprKind::BinOp(SymBinOp::Mul, value, three) = seed.kind() else {
                    return None;
                };
                let matches = three.as_const() == Some(U256::from(3))
                    && odd.is_none_or(|odd| odd == value)
                    && steps >= MIN_NEWTON_STEPS;
                return matches.then_some(value);
            }
            SymExprKind::BinOp(SymBinOp::Mul, left, right) => {
                let (previous, value) = [(left, right), (right, left)].into_iter().find_map(
                    |(previous, correction)| {
                        let SymExprKind::BinOp(SymBinOp::Sub, two, product) = correction.kind()
                        else {
                            return None;
                        };
                        let SymExprKind::BinOp(SymBinOp::Mul, a, b) = product.kind() else {
                            return None;
                        };
                        if two.as_const() != Some(U256::from(2)) {
                            return None;
                        }
                        if a == previous {
                            Some((previous, b))
                        } else if b == previous {
                            Some((previous, a))
                        } else {
                            None
                        }
                    },
                )?;
                if odd.is_some_and(|odd| odd != value) {
                    return None;
                }
                odd = Some(value);
                steps += 1;
                expr = previous;
            }
            _ => return None,
        }
    }
}

/// Returns whether `twos` is `(0 - d) & d`, the lowest set bit of `d`.
fn is_lowest_set_bit(twos: &SymExpr, denominator: &SymExpr) -> bool {
    let SymExprKind::BinOp(SymBinOp::And, left, right) = twos.kind() else {
        return false;
    };
    (left == denominator && is_negation_of(right, denominator))
        || (right == denominator && is_negation_of(left, denominator))
}

fn is_negation_of(expr: &SymExpr, value: &SymExpr) -> bool {
    matches!(
        expr.kind(),
        SymExprKind::BinOp(SymBinOp::Sub, zero, negated)
            if zero.as_const() == Some(U256::ZERO) && negated == value
    )
}

/// Returns `(a, b, d)` when `expr` is the low word of `(a * b - a * b % d) / twos`: the exact
/// low quotient `(lo - rem) / twos` joined with `(high - borrow) * (2**256 / twos)`.
fn exact_shifted_product_operands<'a>(
    expr: &'a SymExpr,
    denominator: &'a SymExpr,
    twos: &SymExpr,
) -> Option<(&'a SymExpr, &'a SymExpr, &'a SymExpr)> {
    let SymExprKind::BinOp(SymBinOp::Or, left, right) = expr.kind() else {
        return None;
    };
    [(left, right), (right, left)].into_iter().find_map(|(low, shifted_high)| {
        let (difference, low_twos) = word_division_operands(low)?;
        let SymExprKind::BinOp(SymBinOp::Sub, lo, remainder) = difference.kind() else {
            return None;
        };
        let SymExprKind::TernOp(SymTernOp::MulMod, a, b, modulus) = remainder.kind() else {
            return None;
        };
        let SymExprKind::BinOp(SymBinOp::Mul, high_part, flip) = shifted_high.kind() else {
            return None;
        };
        let matches = low_twos == twos
            && modulus == denominator
            && is_product_of(lo, a, b)
            && ([(high_part, flip), (flip, high_part)].into_iter().any(|(high_part, flip)| {
                is_borrowed_high(high_part, a, b, lo, remainder) && is_inverse_shift(flip, twos)
            }));
        matches.then_some((a, b, denominator))
    })
}

/// Returns whether `expr` is `mul_high(a, b) - lt(lo, remainder)`.
fn is_borrowed_high(
    expr: &SymExpr,
    a: &SymExpr,
    b: &SymExpr,
    lo: &SymExpr,
    remainder: &SymExpr,
) -> bool {
    matches!(
        expr.kind(),
        SymExprKind::BinOp(SymBinOp::Sub, high, borrow)
            if matches!(
                high.kind(),
                SymExprKind::BinOp(SymBinOp::MulHigh, left, right)
                    if (left == a && right == b) || (left == b && right == a)
            ) && is_less_than_word(borrow, lo, remainder)
    )
}

/// Returns whether `expr` is `(0 - twos) / twos + 1`, which is `2**256 / twos` modulo `2**256`.
fn is_inverse_shift(expr: &SymExpr, twos: &SymExpr) -> bool {
    let SymExprKind::BinOp(SymBinOp::Add, quotient, one) = expr.kind() else {
        return false;
    };
    one.as_const() == Some(U256::ONE)
        && word_division_operands(quotient)
            .is_some_and(|(negated, divisor)| divisor == twos && is_negation_of(negated, twos))
}

/// Returns `(n, d)` for `n / d`, also in the `ite(d == 0, 0, n / d)` form of the EVM `DIV`.
fn word_division_operands(expr: &SymExpr) -> Option<(&SymExpr, &SymExpr)> {
    let division = match expr.kind() {
        SymExprKind::Ite(condition, zero, division) if zero.as_const() == Some(U256::ZERO) => {
            let (_, denominator) = division.udiv_operands()?;
            (condition.zero_check_operand() == Some(denominator)).then_some(division)?
        }
        _ => expr,
    };
    division.udiv_operands()
}
