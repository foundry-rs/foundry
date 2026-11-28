use super::*;
use std::cell::RefCell;

type LessThanFacts<'a> = HashSet<(&'a SymExpr, &'a SymExpr)>;
type LessOrEqualFacts<'a> = HashSet<(&'a SymExpr, &'a SymExpr)>;
type PositiveFacts<'a> = HashSet<&'a SymExpr>;
type ZeroFacts<'a> = HashSet<&'a SymExpr>;

#[derive(Default)]
struct OrderFacts<'a> {
    less_than: LessThanFacts<'a>,
    less_or_equal: LessOrEqualFacts<'a>,
    positive: PositiveFacts<'a>,
    zero: ZeroFacts<'a>,
    /// `expr_less_or_equal` results under these facts and the bounds of the same constraints.
    /// Shared subterms would otherwise make the search take exponential time.
    less_or_equal_results: RefCell<HashMap<(SymExpr, SymExpr), bool>>,
}

/// Returns whether normalized monotonic product facts make constraints unsatisfiable.
pub(crate) fn product_monotonic_unsat_normalized(constraints: &[SymBoolExpr]) -> bool {
    let facts = order_facts(constraints.iter());
    let bounds = ConstraintContext::new(constraints);

    constraints.iter().any(|constraint| {
        reversed_strict_comparison(constraint)
            .is_some_and(|(left, right)| chained_less_or_equal(right, left, &facts, &bounds))
            || wide_term_lower_bound(constraint, constraints, &bounds).is_some_and(
                |(large, lower)| wide_term_below(large, lower, constraints, &facts, &bounds),
            )
            || less_or_equal_comparison(constraint).is_some_and(|(left, right)| {
                successor_product_exceeds(left, right, &facts, &bounds)
                    || quotient_below(right, left, &facts, &bounds)
            })
            || disequality(constraint).is_some_and(|(left, right)| {
                expr_less_or_equal(left, right, &facts, &bounds)
                    && expr_less_or_equal(right, left, &facts, &bounds)
            })
            || product_less_than_negation(constraint).is_some_and(
                |(left_a, left_b, right_a, right_b)| {
                    product_less_than_known(
                        left_a,
                        left_b,
                        right_a,
                        right_b,
                        &facts.less_than,
                        &facts.positive,
                        &bounds,
                    )
                },
            )
    })
}

/// Removes hard-arithmetic comparisons implied by the remaining path constraints.
///
/// This keeps a sound monotonic success path from falling through to the heuristic witness
/// search, whose satisfiable models are useful for counterexamples but cannot establish a proof.
pub(in super::super) fn remove_implied_monotonic_constraints(
    mut constraints: Vec<SymBoolExpr>,
) -> Vec<SymBoolExpr> {
    // Remove constraints one at a time so two candidates cannot justify each other and then both
    // disappear from the final query.
    let mut index = 0;
    while index < constraints.len() {
        if !constraints[index].contains_hard_arith() {
            index += 1;
            continue;
        }
        let Some((left, right)) = less_or_equal_comparison(&constraints[index]) else {
            index += 1;
            continue;
        };
        let base = constraints
            .iter()
            .enumerate()
            .filter(|(candidate, _)| *candidate != index)
            .map(|(_, constraint)| constraint.clone())
            .collect::<Vec<_>>();
        let facts = order_facts(base.iter());
        let bounds = ConstraintContext::new(&base);
        if expr_less_or_equal(left, right, &facts, &bounds) {
            constraints.remove(index);
        } else {
            index += 1;
        }
    }
    constraints
}

fn order_facts<'a>(constraints: impl IntoIterator<Item = &'a SymBoolExpr>) -> OrderFacts<'a> {
    let mut facts = OrderFacts::default();
    for constraint in constraints {
        collect_order_facts(constraint, &mut facts);
    }
    facts
}

fn collect_order_facts<'a>(expr: &'a SymBoolExpr, facts: &mut OrderFacts<'a>) {
    match expr.kind() {
        SymBoolExprKind::And(values) => {
            for value in values.iter() {
                collect_order_facts(value, facts);
            }
        }
        SymBoolExprKind::Cmp(SymCmpOp::Ult, left, right) => {
            facts.less_than.insert((left, right));
            facts.less_or_equal.insert((left, right));
            if left.as_const().is_some_and(|value| value.is_zero()) {
                facts.positive.insert(right);
            }
        }
        SymBoolExprKind::Cmp(SymCmpOp::Ugt, left, right) => {
            facts.less_than.insert((right, left));
            facts.less_or_equal.insert((right, left));
            if right.as_const().is_some_and(|value| value.is_zero()) {
                facts.positive.insert(left);
            }
        }
        SymBoolExprKind::Cmp(SymCmpOp::Ule, left, right) => {
            facts.less_or_equal.insert((left, right));
        }
        SymBoolExprKind::Cmp(SymCmpOp::Uge, left, right) => {
            facts.less_or_equal.insert((right, left));
        }
        SymBoolExprKind::Cmp(SymCmpOp::Eq, left, right) => {
            facts.less_or_equal.insert((left, right));
            facts.less_or_equal.insert((right, left));
            if let Some(value) = expr.zero_check_operand() {
                facts.zero.insert(value);
            }
        }
        SymBoolExprKind::Not(value) => {
            if let Some(expr) = nonzero_expr(value) {
                facts.positive.insert(expr);
            }
            if let SymBoolExprKind::Cmp(op, left, right) = value.kind() {
                match op {
                    // !(a < b) => b <= a
                    SymCmpOp::Ult => {
                        facts.less_or_equal.insert((right, left));
                    }
                    // !(a > b) => a <= b
                    SymCmpOp::Ugt => {
                        facts.less_or_equal.insert((left, right));
                    }
                    // !(a <= b) => b < a
                    SymCmpOp::Ule => {
                        facts.less_than.insert((right, left));
                        facts.less_or_equal.insert((right, left));
                    }
                    // !(a >= b) => a < b
                    SymCmpOp::Uge => {
                        facts.less_than.insert((left, right));
                        facts.less_or_equal.insert((left, right));
                    }
                    SymCmpOp::Eq | SymCmpOp::Slt | SymCmpOp::Sgt => {}
                }
            }
        }
        SymBoolExprKind::Const(_) | SymBoolExprKind::Cmp(SymCmpOp::Slt | SymCmpOp::Sgt, _, _) => {}
    }
}

/// Returns a strict comparison that contradicts `right <= left`.
fn reversed_strict_comparison(expr: &SymBoolExpr) -> Option<(&SymExpr, &SymExpr)> {
    match expr.kind() {
        SymBoolExprKind::Cmp(SymCmpOp::Ult, left, right) => Some((left, right)),
        SymBoolExprKind::Cmp(SymCmpOp::Ugt, left, right) => Some((right, left)),
        SymBoolExprKind::Not(value) => match value.kind() {
            SymBoolExprKind::Cmp(SymCmpOp::Ule, left, right) => Some((right, left)),
            SymBoolExprKind::Cmp(SymCmpOp::Uge, left, right) => Some((left, right)),
            _ => None,
        },
        _ => None,
    }
}

/// Returns whether `left <= right` follows directly or through one asserted ordering.
fn chained_less_or_equal<'a>(
    left: &'a SymExpr,
    right: &'a SymExpr,
    facts: &OrderFacts<'a>,
    bounds: &ConstraintContext,
) -> bool {
    expr_less_or_equal(left, right, facts, bounds)
        || facts.less_or_equal.iter().any(|(small, large)| {
            (*small == left && *large != left && expr_less_or_equal(large, right, facts, bounds))
                || (*large == right
                    && *small != right
                    && expr_less_or_equal(left, small, facts, bounds))
        })
}

/// Returns a high word or quotient and the lower bound this constraint asserts for it.
///
/// A negated conjunction counts as its one conjunct that is not asserted elsewhere.
fn wide_term_lower_bound<'a>(
    constraint: &'a SymBoolExpr,
    constraints: &[SymBoolExpr],
    bounds: &ConstraintContext,
) -> Option<(&'a SymExpr, U256)> {
    if let Some((small, large)) = less_or_equal_comparison(constraint) {
        return is_wide_term(large).then(|| Some((large, bounds.word_range(small)?.0))).flatten();
    }
    if let SymBoolExprKind::Cmp(SymCmpOp::Eq, large, value) = constraint.kind()
        && is_wide_term(large)
    {
        return value.as_const().map(|value| (large, value));
    }
    let SymBoolExprKind::Not(value) = constraint.kind() else { return None };
    let value = match value.kind() {
        SymBoolExprKind::And(terms) => {
            let mut open = terms.iter().filter(|term| !constraints.contains(term));
            let term = open.next()?;
            open.next().is_none().then_some(term)?
        }
        _ => value,
    };
    let large = nonzero_expr(value)?;
    is_wide_term(large).then_some((large, U256::ONE))
}

/// Returns whether a high word or quotient `large` is below `lower` because it is at most a
/// term whose range ends below `lower`.
fn wide_term_below<'a>(
    large: &'a SymExpr,
    lower: U256,
    constraints: &'a [SymBoolExpr],
    facts: &OrderFacts<'a>,
    bounds: &ConstraintContext,
) -> bool {
    let below = |term: &SymExpr| bounds.word_range(term).is_some_and(|(_, upper)| upper < lower);
    below(large)
        || (lower == U256::MAX && successor_below_exact_quotient(large, facts, bounds))
        || constraints.iter().any(|constraint| {
            constraint
                .visit_exprs(&mut |term| {
                    if term != large
                        && is_wide_term(term)
                        && below(term)
                        && expr_less_or_equal(large, term, facts, bounds)
                    {
                        ControlFlow::Break(())
                    } else {
                        ControlFlow::Continue(())
                    }
                })
                .is_break()
        })
}

fn is_wide_term(expr: &SymExpr) -> bool {
    matches!(
        expr.kind(),
        SymExprKind::BinOp(SymBinOp::MulHigh, _, _) | SymExprKind::TernOp(SymTernOp::MulDiv, ..)
    )
}

/// Returns the operands of a disequality, which antisymmetry contradicts.
fn disequality(expr: &SymBoolExpr) -> Option<(&SymExpr, &SymExpr)> {
    let SymBoolExprKind::Not(value) = expr.kind() else { return None };
    let SymBoolExprKind::Cmp(SymCmpOp::Eq, left, right) = value.kind() else { return None };
    Some((left, right))
}

/// Returns the unsigned weak ordering asserted by this constraint.
fn less_or_equal_comparison(expr: &SymBoolExpr) -> Option<(&SymExpr, &SymExpr)> {
    match expr.kind() {
        SymBoolExprKind::Cmp(SymCmpOp::Ule, left, right) => Some((left, right)),
        SymBoolExprKind::Cmp(SymCmpOp::Uge, left, right) => Some((right, left)),
        SymBoolExprKind::Not(value) => match value.kind() {
            SymBoolExprKind::Cmp(SymCmpOp::Ult, left, right) => Some((right, left)),
            SymBoolExprKind::Cmp(SymCmpOp::Ugt, left, right) => Some((left, right)),
            _ => None,
        },
        _ => None,
    }
}

/// Returns whether the known unsigned order and no-overflow bounds imply `left <= right`.
fn expr_less_or_equal<'a>(
    left: &'a SymExpr,
    right: &'a SymExpr,
    facts: &OrderFacts<'a>,
    bounds: &ConstraintContext,
) -> bool {
    if left == right || facts.less_or_equal.contains(&(left, right)) {
        return true;
    }
    let key = (left.clone(), right.clone());
    if let Some(&result) = facts.less_or_equal_results.borrow().get(&key) {
        return result;
    }
    // A pair still being searched counts as unproven, which only declines.
    facts.less_or_equal_results.borrow_mut().insert(key.clone(), false);

    let result = match (left.kind(), right.kind()) {
        (
            SymExprKind::BinOp(SymBinOp::UDiv, left_num, left_den),
            SymExprKind::BinOp(SymBinOp::UDiv, right_num, right_den),
        ) if left_den == right_den => expr_less_or_equal(left_num, right_num, facts, bounds),
        // `n / d2 <= n / d1` for `0 < d1 <= d2`; a zero `d1` would make `n / d1` zero.
        (
            SymExprKind::BinOp(SymBinOp::UDiv, left_num, left_den),
            SymExprKind::BinOp(SymBinOp::UDiv, right_num, right_den),
        ) if left_num == right_num
            && (facts.positive.contains(right_den)
                || bounds.word_range(right_den).is_some_and(|(min, _)| !min.is_zero()))
            && expr_less_or_equal(right_den, left_den, facts, bounds) =>
        {
            true
        }
        // `left <= a` or `left <= b`, so `left <= a + b` when the sum does not wrap.
        (_, SymExprKind::BinOp(SymBinOp::Add, a, b))
            if bounds
                .word_range(a)
                .zip(bounds.word_range(b))
                .is_some_and(|((_, a), (_, b))| a.checked_add(b).is_some())
                && (expr_less_or_equal(left, a, facts, bounds)
                    || expr_less_or_equal(left, b, facts, bounds)) =>
        {
            true
        }
        // `c + x <= c + y` for `x <= y` when the larger sum does not wrap: its operands' ranges
        // fit, or one operand is at most the other's complement, as a checked addition asserts.
        (SymExprKind::BinOp(SymBinOp::Add, a, b), SymExprKind::BinOp(SymBinOp::Add, c, d))
            if let Some((x, y)) = [(a, b), (b, a)].into_iter().find_map(|(common, x)| {
                (common == c).then_some((x, d)).or((common == d).then_some((x, c)))
            }) && (facts.less_or_equal.iter().any(|(small, large)| {
                matches!(large.kind(), SymExprKind::Not(inner) if same_factors(small, inner, c, d))
            }) || bounds
                .word_range(c)
                .zip(bounds.word_range(d))
                .is_some_and(|((_, c), (_, d))| c.checked_add(d).is_some()))
                && expr_less_or_equal(x, y, facts, bounds) =>
        {
            true
        }
        // `x - p <= y - p` for `p <= x <= y`, so neither difference wraps.
        (SymExprKind::BinOp(SymBinOp::Sub, x, p), SymExprKind::BinOp(SymBinOp::Sub, y, q))
            if p == q
                && expr_less_or_equal(p, x, facts, bounds)
                && expr_less_or_equal(x, y, facts, bounds) =>
        {
            true
        }
        // `e - b <= e - a` for `a <= b <= e`, so neither difference wraps, as for decaying voting
        // power.
        (SymExprKind::BinOp(SymBinOp::Sub, e, b), SymExprKind::BinOp(SymBinOp::Sub, f, a))
            if e == f
                && expr_less_or_equal(b, e, facts, bounds)
                && expr_less_or_equal(a, b, facts, bounds) =>
        {
            true
        }
        // `~x <= ~y` for `y <= x`, as `~x` is `MAX - x`.
        (SymExprKind::Not(x), SymExprKind::Not(y)) => expr_less_or_equal(y, x, facts, bounds),
        // `x - e <= x <= right` when `e <= x`, so the difference does not wrap, as for an accrued
        // index delta.
        (SymExprKind::BinOp(SymBinOp::Sub, minuend, subtrahend), _)
            if expr_less_or_equal(subtrahend, minuend, facts, bounds)
                && expr_less_or_equal(minuend, right, facts, bounds) =>
        {
            true
        }
        // A failed bounded proof falls through: later arms can still order non-wrapping products.
        (
            SymExprKind::BinOp(SymBinOp::Mul, left_a, left_b),
            SymExprKind::BinOp(SymBinOp::Mul, right_a, right_b),
        ) if bounds.mul_cannot_overflow_256(left_a, left_b)
            && bounds.mul_cannot_overflow_256(right_a, right_b)
            && product_less_or_equal_known(left_a, left_b, right_a, right_b, facts, bounds) =>
        {
            true
        }
        // Equal high words order the full products by their low words.
        (
            SymExprKind::BinOp(SymBinOp::Mul, left_a, left_b),
            SymExprKind::BinOp(SymBinOp::Mul, right_a, right_b),
        ) if facts.less_or_equal.iter().any(|(small, large)| {
            is_mul_high_of(small, right_a, right_b) && is_mul_high_of(large, left_a, left_b)
        }) =>
        {
            wide_product_less_or_equal(left_a, left_b, right_a, right_b, facts, bounds)
        }
        // Equal full products have equal low words, such as `q * d` and `x * y` for the quotient
        // `q` of `x * y / d` with no remainder.
        (
            SymExprKind::BinOp(SymBinOp::Mul, left_a, left_b),
            SymExprKind::BinOp(SymBinOp::Mul, right_a, right_b),
        ) if is_quotient_product(left_a, left_b, right_a, right_b, bounds)
            || is_quotient_product(right_a, right_b, left_a, left_b, bounds) =>
        {
            wide_product_less_or_equal(left_a, left_b, right_a, right_b, facts, bounds)
                && wide_product_less_or_equal(right_a, right_b, left_a, left_b, facts, bounds)
        }
        (
            SymExprKind::BinOp(SymBinOp::MulHigh, left_a, left_b),
            SymExprKind::BinOp(SymBinOp::MulHigh, right_a, right_b),
        ) => wide_product_less_or_equal(left_a, left_b, right_a, right_b, facts, bounds),
        // `x <= x * a / d` for `0 < d <= a` when `x * a` does not wrap, as for a balance scaled by
        // a grown index.
        (_, SymExprKind::BinOp(SymBinOp::UDiv, product, divisor))
            if let Some((a, b)) = mul_operands(product)
                && let Some(scale) = (a == left).then_some(b).or((b == left).then_some(a))
                && (facts.positive.contains(divisor)
                    || bounds.word_range(divisor).is_some_and(|(min, _)| !min.is_zero()))
                && bounds.mul_cannot_overflow_256(a, b)
                && (expr_less_or_equal(divisor, scale, facts, bounds)
                    || bounds
                        .word_range(divisor)
                        .zip(bounds.word_range(scale))
                        .is_some_and(|((_, divisor), (scale, _))| divisor <= scale)) =>
        {
            true
        }
        // `(x * a + c) / d <= x` for `0 < d`, `a <= d`, and `c < d`, as for a debt scaled by a
        // close factor and rounded half up: `x * a + c < (x + 1) * d`, and a wrapped product or sum
        // only shrinks the dividend.
        (SymExprKind::BinOp(SymBinOp::UDiv, dividend, divisor), _)
            if scaled_quotient_at_most(dividend, divisor, right, facts, bounds) =>
        {
            true
        }
        // `left <= C` when the interval of `left` is at most `C`.
        (_, SymExprKind::Const(bound))
            if bounds.word_range(left).is_some_and(|(_, max)| max <= *bound) =>
        {
            true
        }
        // `left <= round(a * b / d) <= round'(a' * b' / d) <= right`.
        _ if let Some(upper) = quotient_upper_bound(left, facts, bounds)
            && let Some(lower) = quotient_lower_bound(right, facts, bounds) =>
        {
            rounded_quotients_ordered(upper, lower, facts, bounds)
        }
        _ => false,
    };
    facts.less_or_equal_results.borrow_mut().insert(key, result);
    result
}

/// Rounding of `x * y / d` over the integers, where every rounding of `x * y / 0` is zero as in
/// the EVM. `Floor <= Ceil`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Rounding {
    Floor,
    Ceil,
}

/// `(x, y, d, rounding)` for the rounded quotient `rounding(x * y / d)`.
type RoundedQuotient<'a> = (&'a SymExpr, &'a SymExpr, &'a SymExpr, Rounding);

/// Returns a rounded quotient that `expr` is at most.
///
/// These bounds follow from the Euclidean relation `x * y == q * d + r` with
/// `q = floor(x * y / d)` and `r = mulmod(x, y, d) < d`, so `ceil(x * y / d)` is `q + (r != 0)`.
fn quotient_upper_bound<'a>(
    expr: &'a SymExpr,
    facts: &OrderFacts<'a>,
    bounds: &ConstraintContext,
) -> Option<RoundedQuotient<'a>> {
    // A truncated quotient is still at most `floor(x * y / d)`.
    if let Some((x, y, denominator)) = quotient_operands(expr, bounds) {
        return Some((x, y, denominator, Rounding::Floor));
    }
    // Wrapping only lowers `q + 1`.
    successor_upper_bound(increment_operand(expr)?, facts, bounds)
}

/// Returns `ceil(x * y / d)` when the quotient `q` of `x * y / d` has a nonzero remainder, so that
/// `q + 1 <= floor(x * y / d) + 1 == ceil(x * y / d)` over the integers.
fn successor_upper_bound<'a>(
    quotient: &'a SymExpr,
    facts: &OrderFacts<'a>,
    bounds: &ConstraintContext,
) -> Option<RoundedQuotient<'a>> {
    let (x, y, denominator) = quotient_operands(quotient, bounds)?;
    if has_remainder(x, y, denominator, facts) != Some(true) {
        return None;
    }
    Some((x, y, denominator, Rounding::Ceil))
}

/// Returns whether `round(a * b / d) <= round'(a' * b' / d)`, which holds for the same divisor
/// once `round <= round'` and `a * b <= a' * b'`.
fn rounded_quotients_ordered<'a>(
    (left_a, left_b, left_den, left_rounding): RoundedQuotient<'a>,
    (right_a, right_b, right_den, right_rounding): RoundedQuotient<'a>,
    facts: &OrderFacts<'a>,
    bounds: &ConstraintContext,
) -> bool {
    left_den == right_den
        && left_rounding <= right_rounding
        && wide_product_less_or_equal(left_a, left_b, right_a, right_b, facts, bounds)
}

/// Returns whether `quotient + 1 <= MAX`, because it is at most `ceil(x * y / d)` for a product
/// that `d` divides with a quotient that fits a word.
///
/// This refutes the overflow check of a rounded-up quotient once the path has computed an exact
/// quotient of a product at least as large, whose own term is not part of the query, as in
/// `mulDivRoundingUp(x1, y, d) <= mulDivRoundingUp(x2, y, d)` when only `x1 * y` has a remainder.
fn successor_below_exact_quotient<'a>(
    quotient: &'a SymExpr,
    facts: &OrderFacts<'a>,
    bounds: &ConstraintContext,
) -> bool {
    successor_upper_bound(quotient, facts, bounds).is_some_and(|upper| {
        facts.zero.iter().any(|remainder| {
            matches!(
                remainder.kind(),
                SymExprKind::TernOp(SymTernOp::MulMod, x, y, denominator)
                    if (product_quotient_fits(x, y, denominator, facts)
                        || bounds.mul_cannot_overflow_256(x, y))
                        && rounded_quotients_ordered(
                            upper,
                            (x, y, denominator, Rounding::Ceil),
                            facts,
                            bounds,
                        )
            )
        })
    })
}

/// Returns a rounded quotient that `expr` is at least.
fn quotient_lower_bound<'a>(
    expr: &'a SymExpr,
    facts: &OrderFacts<'a>,
    bounds: &ConstraintContext,
) -> Option<RoundedQuotient<'a>> {
    // A quotient that fits is `floor(x * y / d)`, which is also the ceiling when `r == 0`.
    if let Some((x, y, denominator)) = quotient_operands(expr, bounds) {
        if !quotient_fits(expr, facts) {
            return None;
        }
        let exact = has_remainder(x, y, denominator, facts) == Some(false);
        return Some((x, y, denominator, if exact { Rounding::Ceil } else { Rounding::Floor }));
    }
    // `q + 1 == floor(x * y / d) + 1 >= ceil(x * y / d)` when the increment does not wrap.
    let quotient = increment_operand(expr)?;
    let (x, y, denominator) = quotient_operands(quotient, bounds)?;
    let below_max = bounds.word_range(quotient).is_some_and(|(_, max)| max < U256::MAX)
        // A word division `x * y / d` reaches `MAX` only for `d == 1`, which leaves no remainder.
        || (matches!(quotient.kind(), SymExprKind::BinOp(SymBinOp::UDiv, ..))
            && has_remainder(x, y, denominator, facts) == Some(true));
    (quotient_fits(quotient, facts) && below_max).then_some((x, y, denominator, Rounding::Ceil))
}

/// Returns whether the facts assert `mulmod(x, y, d) != 0`, or `false` when they assert it is zero.
fn has_remainder(
    x: &SymExpr,
    y: &SymExpr,
    denominator: &SymExpr,
    facts: &OrderFacts<'_>,
) -> Option<bool> {
    let is_remainder = |expr: &&SymExpr| {
        matches!(
            expr.kind(),
            SymExprKind::TernOp(SymTernOp::MulMod, a, b, modulus)
                if modulus == denominator && same_factors(a, b, x, y)
        )
    };
    if facts.positive.iter().any(is_remainder) {
        Some(true)
    } else if facts.zero.iter().any(is_remainder) {
        Some(false)
    } else {
        None
    }
}

/// Returns `q` for `q + 1`.
fn increment_operand(expr: &SymExpr) -> Option<&SymExpr> {
    match expr.kind() {
        SymExprKind::BinOp(SymBinOp::Add, value, one) if one.as_const() == Some(U256::ONE) => {
            Some(value)
        }
        _ => None,
    }
}

/// Returns whether `left_a * left_b <= right_a * right_b` over the unbounded integers.
fn wide_product_less_or_equal<'a>(
    left_a: &'a SymExpr,
    left_b: &'a SymExpr,
    right_a: &'a SymExpr,
    right_b: &'a SymExpr,
    facts: &OrderFacts<'a>,
    bounds: &ConstraintContext,
) -> bool {
    product_less_or_equal_known(left_a, left_b, right_a, right_b, facts, bounds)
        // `floor(x * y / d) * d <= x * y`.
        || is_quotient_product(left_a, left_b, right_a, right_b, bounds)
        || [(right_a, right_b), (right_b, right_a)].into_iter().any(|(quotient, divisor)| {
            // `x * y <= ceil(x * y / d) * d` for `d != 0`.
            quotient_lower_bound(quotient, facts, bounds).is_some_and(
                |(x, y, denominator, rounding)| {
                    rounding == Rounding::Ceil
                        && denominator == divisor
                        && same_factors(x, y, left_a, left_b)
                        && bounds.word_range(denominator).is_some_and(|(min, _)| !min.is_zero())
                },
            )
        })
}

/// Returns `(x, y, d)` when `expr` is `floor(x * y / d)` truncated to a word, or zero for `d == 0`.
fn quotient_operands<'a>(
    expr: &'a SymExpr,
    bounds: &ConstraintContext,
) -> Option<(&'a SymExpr, &'a SymExpr, &'a SymExpr)> {
    match expr.kind() {
        SymExprKind::TernOp(SymTernOp::MulDiv, left, right, denominator) => {
            Some((left, right, denominator))
        }
        SymExprKind::BinOp(SymBinOp::UDiv, product, denominator) => {
            let (left, right) = mul_operands(product)?;
            bounds.mul_cannot_overflow_256(left, right).then_some((left, right, denominator))
        }
        _ => None,
    }
}

/// Returns whether a quotient is known not to be truncated to a word.
fn quotient_fits(expr: &SymExpr, facts: &OrderFacts<'_>) -> bool {
    match expr.kind() {
        SymExprKind::TernOp(SymTernOp::MulDiv, left, right, denominator) => {
            product_quotient_fits(left, right, denominator, facts)
        }
        _ => true,
    }
}

/// Returns whether the facts assert `mul_high(x, y) < d`, so `x * y / d` fits a word.
fn product_quotient_fits(
    x: &SymExpr,
    y: &SymExpr,
    denominator: &SymExpr,
    facts: &OrderFacts<'_>,
) -> bool {
    facts.less_than.iter().any(|(high, bound)| *bound == denominator && is_mul_high_of(high, x, y))
}

/// Returns whether `a * b` is `q * d` for a quotient `q` of `x * y / d`, where `(x, y)` are the
/// factors of the other product.
fn is_quotient_product(
    a: &SymExpr,
    b: &SymExpr,
    x: &SymExpr,
    y: &SymExpr,
    bounds: &ConstraintContext,
) -> bool {
    [(a, b), (b, a)].into_iter().any(|(quotient, divisor)| {
        quotient_operands(quotient, bounds).is_some_and(|(left, right, denominator)| {
            denominator == divisor && same_factors(left, right, x, y)
        })
    })
}

fn is_mul_high_of(expr: &SymExpr, left: &SymExpr, right: &SymExpr) -> bool {
    matches!(
        expr.kind(),
        SymExprKind::BinOp(SymBinOp::MulHigh, a, b) if same_factors(a, b, left, right)
    )
}

/// Returns whether `{a, b}` and `{x, y}` are the same factors in either order.
fn same_factors(a: &SymExpr, b: &SymExpr, x: &SymExpr, y: &SymExpr) -> bool {
    (a == x && b == y) || (a == y && b == x)
}

fn product_less_or_equal_known<'a>(
    left_a: &'a SymExpr,
    left_b: &'a SymExpr,
    right_a: &'a SymExpr,
    right_b: &'a SymExpr,
    facts: &OrderFacts<'a>,
    bounds: &ConstraintContext,
) -> bool {
    product_less_or_equal_known_ordered(left_a, left_b, right_a, right_b, facts, bounds)
        || product_less_or_equal_known_ordered(left_b, left_a, right_a, right_b, facts, bounds)
        || product_less_or_equal_known_ordered(left_a, left_b, right_b, right_a, facts, bounds)
        || product_less_or_equal_known_ordered(left_b, left_a, right_b, right_a, facts, bounds)
}

fn product_less_or_equal_known_ordered<'a>(
    left_a: &'a SymExpr,
    left_b: &'a SymExpr,
    right_a: &'a SymExpr,
    right_b: &'a SymExpr,
    facts: &OrderFacts<'a>,
    bounds: &ConstraintContext,
) -> bool {
    expr_less_or_equal(left_a, right_a, facts, bounds)
        && expr_less_or_equal(left_b, right_b, facts, bounds)
}

fn nonzero_expr(expr: &SymBoolExpr) -> Option<&SymExpr> {
    let SymBoolExprKind::Cmp(SymCmpOp::Eq, left, right) = expr.kind() else { return None };
    if left.as_const().is_some_and(|value| value.is_zero()) {
        Some(right)
    } else if right.as_const().is_some_and(|value| value.is_zero()) {
        Some(left)
    } else {
        None
    }
}

fn product_less_than_negation(
    expr: &SymBoolExpr,
) -> Option<(&SymExpr, &SymExpr, &SymExpr, &SymExpr)> {
    let SymBoolExprKind::Not(value) = expr.kind() else { return None };
    let SymBoolExprKind::Cmp(SymCmpOp::Ult, left, right) = value.kind() else {
        return None;
    };
    let (left_a, left_b) = mul_operands(left)?;
    let (right_a, right_b) = mul_operands(right)?;
    Some((left_a, left_b, right_a, right_b))
}

fn product_less_than_known<'a>(
    left_a: &'a SymExpr,
    left_b: &'a SymExpr,
    right_a: &'a SymExpr,
    right_b: &'a SymExpr,
    less_than: &LessThanFacts<'a>,
    positive: &PositiveFacts<'a>,
    bounds: &ConstraintContext,
) -> bool {
    product_less_than_known_ordered(left_a, left_b, right_a, right_b, less_than, positive, bounds)
        || product_less_than_known_ordered(
            left_b, left_a, right_a, right_b, less_than, positive, bounds,
        )
        || product_less_than_known_ordered(
            left_a, left_b, right_b, right_a, less_than, positive, bounds,
        )
        || product_less_than_known_ordered(
            left_b, left_a, right_b, right_a, less_than, positive, bounds,
        )
}

fn product_less_than_known_ordered<'a>(
    left_a: &'a SymExpr,
    left_b: &'a SymExpr,
    right_a: &'a SymExpr,
    right_b: &'a SymExpr,
    less_than: &LessThanFacts<'a>,
    positive: &PositiveFacts<'a>,
    bounds: &ConstraintContext,
) -> bool {
    positive.contains(left_a)
        && positive.contains(left_b)
        && less_than.contains(&(left_a, right_a))
        && less_than.contains(&(left_b, right_b))
        && bounds.mul_cannot_overflow_256(left_a, left_b)
        && bounds.mul_cannot_overflow_256(right_a, right_b)
}

fn mul_operands(expr: &SymExpr) -> Option<(&SymExpr, &SymExpr)> {
    match expr.kind() {
        SymExprKind::BinOp(SymBinOp::Mul, left, right) => Some((left, right)),
        _ => None,
    }
}

/// Returns whether `x * y < (u + 1) * c`, refuting `(u + 1) * c <= x * y`, which is how a strict
/// `u < x * y / c` normalizes. `x * y <= u * c < u * c + c` once `c` is nonzero and neither
/// `u + 1` nor the larger product wraps, so `x * y` cannot wrap either.
fn successor_product_exceeds(
    successor_product: &SymExpr,
    product: &SymExpr,
    facts: &OrderFacts<'_>,
    bounds: &ConstraintContext,
) -> bool {
    let (Some((successor, c)), Some((x, y))) =
        (mul_operands(successor_product), mul_operands(product))
    else {
        return false;
    };
    c.as_const().is_some_and(|c| !c.is_zero())
        && increment_operand(successor).is_some_and(|u| {
            bounds.word_range(u).is_some_and(|(_, max)| max < U256::MAX)
                && bounds.mul_cannot_overflow_256(successor, c)
                && product_less_or_equal_known(x, y, u, c, facts, bounds)
        })
}

/// Returns whether `quotient = x * scale / d` is below `bound` because `x < d` and
/// `0 < scale <= bound`, refuting `bound <= quotient` as in a collateral ratio of at least 100%
/// with collateral below debt. The wrapped product is at most `x * scale < d * scale`, and `d > x`
/// makes `d` nonzero.
fn quotient_below(
    quotient: &SymExpr,
    bound: &SymExpr,
    facts: &OrderFacts<'_>,
    bounds: &ConstraintContext,
) -> bool {
    let SymExprKind::BinOp(SymBinOp::UDiv, product, divisor) = quotient.kind() else {
        return false;
    };
    let Some((a, b)) = mul_operands(product) else { return false };
    [(a, b), (b, a)].into_iter().any(|(scale, x)| {
        (facts.positive.contains(scale)
            || bounds.word_range(scale).is_some_and(|(min, _)| !min.is_zero()))
            && (expr_less_or_equal(scale, bound, facts, bounds)
                || bounds
                    .word_range(scale)
                    .zip(bounds.word_range(bound))
                    .is_some_and(|((_, scale), (bound, _))| scale <= bound))
            && facts.less_than.contains(&(x, divisor))
    })
}

/// Returns whether `(x * a + c) / d <= x`, with an optional addend `c`, for a nonzero `d`,
/// `a <= d`, and `c < d`. `division_order` in `normalize/rounding.rs` decides a direct comparison
/// with this bound once `x * a` cannot wrap.
fn scaled_quotient_at_most<'a>(
    dividend: &'a SymExpr,
    divisor: &SymExpr,
    x: &SymExpr,
    facts: &OrderFacts<'_>,
    bounds: &ConstraintContext,
) -> bool {
    let scale = |product: &'a SymExpr| {
        let (a, b) = mul_operands(product)?;
        (a == x).then_some(b).or((b == x).then_some(a))
    };
    let (scale, addend) = match dividend.kind() {
        SymExprKind::BinOp(SymBinOp::Add, a, b) => match (scale(a), scale(b)) {
            (Some(scale), _) => (scale, Some(b)),
            (_, Some(scale)) => (scale, Some(a)),
            _ => return false,
        },
        _ => {
            let Some(scale) = scale(dividend) else { return false };
            (scale, None)
        }
    };
    // A bare `UDiv` by zero is all ones in SMT; only the EVM's guarded `DIV` yields zero.
    let divisor_min = match bounds.word_range(divisor) {
        Some((min, _)) if !min.is_zero() => min,
        _ if facts.positive.contains(divisor) => U256::ONE,
        _ => return false,
    };
    addend.is_none_or(|addend| bounds.word_range(addend).is_some_and(|(_, max)| max < divisor_min))
        && (expr_less_or_equal(scale, divisor, facts, bounds)
            || bounds.word_range(scale).is_some_and(|(_, max)| max <= divisor_min))
}
