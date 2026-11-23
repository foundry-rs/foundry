use super::*;
use std::cell::RefCell;

type LessThanFacts<'a> = HashSet<(&'a SymExpr, &'a SymExpr)>;
type LessOrEqualFacts<'a> = HashSet<(&'a SymExpr, &'a SymExpr)>;
type PositiveFacts<'a> = HashSet<&'a SymExpr>;
type ZeroFacts<'a> = HashSet<&'a SymExpr>;

/// The power of two `1 << n` for each shift amount `n` of a query, keyed by the amount.
type ShiftPowers = HashMap<SymExpr, SymExpr>;

#[derive(Default)]
struct OrderFacts<'a> {
    less_than: LessThanFacts<'a>,
    less_or_equal: LessOrEqualFacts<'a>,
    positive: PositiveFacts<'a>,
    zero: ZeroFacts<'a>,
    /// The conjuncts of each negated conjunction.
    denied: Vec<&'a [SymBoolExpr]>,
    powers: Option<&'a ShiftPowers>,
    /// `expr_less_or_equal` results under these facts and the bounds of the same constraints.
    /// Shared subterms would otherwise make the search take exponential time.
    less_or_equal_results: RefCell<HashMap<(SymExpr, SymExpr), bool>>,
}

impl<'a> OrderFacts<'a> {
    /// Returns `1 << n` for the shift amount `n`.
    fn power(&self, shift: &SymExpr) -> Option<&'a SymExpr> {
        self.powers?.get(shift)
    }
}

/// Returns whether normalized monotonic product facts make constraints unsatisfiable.
pub(crate) fn product_monotonic_unsat_normalized(
    cx: &mut SymCx,
    constraints: &[SymBoolExpr],
) -> bool {
    let powers = shift_powers(cx, constraints);
    let bounds = if divides_a_product(constraints) {
        ConstraintContext::with_non_wrapping_products(cx, constraints)
    } else {
        ConstraintContext::new(constraints)
    };
    let facts = order_facts(constraints, &powers, &bounds);

    remainder_spellings_disagree(&facts, &bounds)
        || constraints.iter().any(|constraint| {
            reversed_strict_comparison(constraint)
                .is_some_and(|(left, right)| chained_less_or_equal(right, left, &facts, &bounds))
                || wide_term_lower_bound(constraint, constraints, &bounds).is_some_and(
                    |(large, lower)| wide_term_below(large, lower, constraints, &facts, &bounds),
                )
                || less_or_equal_comparison(constraint).is_some_and(|(left, right)| {
                    successor_product_exceeds(left, right, &facts, &bounds)
                })
                || disequality(constraint).is_some_and(|(left, right)| {
                    expr_less_or_equal(left, right, &facts, &bounds)
                        && expr_less_or_equal(right, left, &facts, &bounds)
                })
                || product_less_than_negation(constraint, &facts).is_some_and(
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
    cx: &mut SymCx,
    mut constraints: Vec<SymBoolExpr>,
) -> Vec<SymBoolExpr> {
    if !constraints.iter().any(SymBoolExpr::contains_hard_arith) {
        return constraints;
    }
    let powers = shift_powers(cx, &constraints);
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
        let bounds = ConstraintContext::new(&base);
        let facts = order_facts(&base, &powers, &bounds);
        if expr_less_or_equal(left, right, &facts, &bounds) {
            constraints.remove(index);
        } else {
            index += 1;
        }
    }
    constraints
}

fn order_facts<'a>(
    constraints: &'a [SymBoolExpr],
    powers: &'a ShiftPowers,
    bounds: &ConstraintContext,
) -> OrderFacts<'a> {
    let mut facts = OrderFacts { powers: Some(powers), ..Default::default() };
    for constraint in constraints {
        collect_order_facts(constraint, &mut facts);
        // A negated conjunction denies its one conjunct that is neither asserted nor range-implied.
        if let SymBoolExprKind::Not(value) = constraint.kind()
            && let SymBoolExprKind::And(terms) = value.kind()
        {
            facts.denied.push(terms);
            if let Some(open) = open_conjunct(value, constraints, bounds)
                && let SymBoolExprKind::Not(denied) = open.kind()
            {
                collect_order_facts(denied, &mut facts);
            }
        }
    }
    facts
}

/// Returns the one conjunct of `value` that is neither a constraint nor implied by the ranges, or
/// `value` when it is not a conjunction.
fn open_conjunct<'a>(
    value: &'a SymBoolExpr,
    constraints: &[SymBoolExpr],
    bounds: &ConstraintContext,
) -> Option<&'a SymBoolExpr> {
    let SymBoolExprKind::And(terms) = value.kind() else { return Some(value) };
    let mut open = terms.iter().filter(|term| {
        !constraints.contains(term) && bounds.bounded_bool_value(term) != Some(true)
    });
    let term = open.next()?;
    open.next().is_none().then_some(term)
}

/// Returns whether `expr` is the high word of a product that fits a word.
fn is_zero_high<'a>(expr: &'a SymExpr, facts: &OrderFacts<'a>, bounds: &ConstraintContext) -> bool {
    matches!(
        expr.kind(),
        SymExprKind::BinOp(SymBinOp::MulHigh, a, b) if product_fits(a, b, facts, bounds)
    )
}

/// Returns whether `a * b` fits a word: by the context, or as `q * d` for a quotient `q` of
/// `x * y / d` whose product `x * y` fits.
fn product_fits<'a>(
    a: &'a SymExpr,
    b: &'a SymExpr,
    facts: &OrderFacts<'a>,
    bounds: &ConstraintContext,
) -> bool {
    bounds.mul_cannot_overflow_256(a, b)
        || [(a, b), (b, a)].into_iter().any(|(quotient, divisor)| {
            quotient_operands(quotient, facts, bounds).is_some_and(|(x, y, denominator)| {
                denominator == divisor && bounds.mul_cannot_overflow_256(x, y)
            })
        })
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

/// Returns `1 << n` for each shift amount `n` of the constraints.
fn shift_powers(cx: &mut SymCx, constraints: &[SymBoolExpr]) -> ShiftPowers {
    let mut shifts = HashSet::<SymExpr>::default();
    for constraint in constraints {
        let _ = constraint.visit_exprs::<()>(&mut |expr| {
            if let SymExprKind::BinOp(SymBinOp::Shl | SymBinOp::Shr, _, shift) = expr.kind() {
                shifts.insert(shift.clone());
            }
            ControlFlow::Continue(())
        });
    }
    shifts
        .into_iter()
        .map(|shift| {
            let power = shift_power(cx, &shift);
            (shift, power)
        })
        .collect()
}

/// Returns whether a constraint divides or shifts down a product by other than its own factor.
fn divides_a_product(constraints: &[SymBoolExpr]) -> bool {
    constraints.iter().any(|constraint| {
        constraint.visit_bool(|expr| {
            matches!(
                expr.kind(),
                SymExprKind::BinOp(SymBinOp::UDiv | SymBinOp::Shr, value, operand)
                    if match value.kind() {
                        SymExprKind::BinOp(SymBinOp::Mul, a, b) => a != operand && b != operand,
                        SymExprKind::BinOp(SymBinOp::Shl, a, _) => a != operand,
                        _ => false,
                    }
            )
        })
    })
}

/// Returns whether the facts assert a zero and a nonzero remainder of one division: `mulmod(x, y,
/// d)` and `(x * y) % d` agree for a product that fits a word and `d != 0`.
fn remainder_spellings_disagree(facts: &OrderFacts<'_>, bounds: &ConstraintContext) -> bool {
    [(&facts.zero, &facts.positive), (&facts.positive, &facts.zero)].into_iter().any(
        |(wide, word)| {
            wide.iter().any(|remainder| {
                let SymExprKind::TernOp(SymTernOp::MulMod, x, y, denominator) = remainder.kind()
                else {
                    return false;
                };
                bounds.mul_cannot_overflow_256(x, y)
                    && bounds.word_range(denominator).is_some_and(|(min, _)| !min.is_zero())
                    && word.iter().any(|other| {
                        matches!(
                            other.kind(),
                            SymExprKind::BinOp(SymBinOp::URem, product, modulus)
                                if modulus == denominator && is_product_of(product, x, y)
                        )
                    })
            })
        },
    )
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
    if let Some(large) = at_max_operand(constraint).filter(|large| {
        is_wide_term(large) || matches!(large.kind(), SymExprKind::BinOp(SymBinOp::UDiv, ..))
    }) {
        return Some((large, U256::MAX));
    }
    if let Some((small, large)) = less_or_equal_comparison(constraint) {
        return is_wide_term(large).then(|| Some((large, bounds.word_range(small)?.0))).flatten();
    }
    if let Some((small, large)) = reversed_strict_comparison(constraint) {
        return is_wide_term(large)
            .then(|| Some((large, bounds.word_range(small)?.0.checked_add(U256::ONE)?)))
            .flatten();
    }
    if let SymBoolExprKind::Cmp(SymCmpOp::Eq, large, value) = constraint.kind()
        && is_wide_term(large)
    {
        return value.as_const().map(|value| (large, value));
    }
    let SymBoolExprKind::Not(value) = constraint.kind() else { return None };
    let value = open_conjunct(value, constraints, bounds)?;
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
        || (!lower.is_zero() && is_zero_high(large, facts, bounds))
        || (lower == U256::MAX
            && (successor_below_exact_quotient(large, facts, bounds)
                || quotient_operands(large, facts, bounds).is_some_and(|(x, y, denominator)| {
                    has_remainder(x, y, denominator, facts) == Some(true)
                        && remainder_excludes_max(large, x, y, denominator, facts, bounds)
                })))
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
    let result = expr_less_or_equal_uncached(left, right, facts, bounds);
    facts.less_or_equal_results.borrow_mut().insert(key, result);
    result
}

fn expr_less_or_equal_uncached<'a>(
    left: &'a SymExpr,
    right: &'a SymExpr,
    facts: &OrderFacts<'a>,
    bounds: &ConstraintContext,
) -> bool {
    if is_zero_high(left, facts, bounds) {
        return true;
    }

    if let Some((left_a, left_b)) = product_operands(left, facts)
        && let Some((right_a, right_b)) = product_operands(right, facts)
    {
        // Low words that are the full products order as the products do.
        if product_fits(left_a, left_b, facts, bounds)
            && product_fits(right_a, right_b, facts, bounds)
        {
            return wide_product_less_or_equal(left_a, left_b, right_a, right_b, facts, bounds);
        }
        // Equal high words order the full products by their low words.
        if facts.less_or_equal.iter().any(|(small, large)| {
            is_mul_high_of(small, right_a, right_b) && is_mul_high_of(large, left_a, left_b)
        }) {
            return wide_product_less_or_equal(left_a, left_b, right_a, right_b, facts, bounds);
        }
        // Equal full products have equal low words, such as `q * d` and `x * y` for the quotient
        // `q` of `x * y / d` with no remainder.
        if is_quotient_product(left_a, left_b, right_a, right_b, facts, bounds)
            || is_quotient_product(right_a, right_b, left_a, left_b, facts, bounds)
        {
            return wide_product_less_or_equal(left_a, left_b, right_a, right_b, facts, bounds)
                && wide_product_less_or_equal(right_a, right_b, left_a, left_b, facts, bounds);
        }
    }

    match (left.kind(), right.kind()) {
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
        // `~x <= ~y` for `y <= x`, as `~x` is `MAX - x`.
        (SymExprKind::Not(x), SymExprKind::Not(y)) => expr_less_or_equal(y, x, facts, bounds),
        (
            SymExprKind::BinOp(SymBinOp::MulHigh, left_a, left_b),
            SymExprKind::BinOp(SymBinOp::MulHigh, right_a, right_b),
        ) => {
            wide_product_less_or_equal(left_a, left_b, right_a, right_b, facts, bounds)
                // `q * 2**n` is `x * y` less its low `n` bits, which leaves the high word.
                || quotient_product_parts(right_a, right_b, left_a, left_b, facts, bounds)
                    .is_some_and(|(quotient, divisor)| {
                        quotient_fits(quotient, facts, bounds) && is_power_of_two(divisor, bounds)
                    })
        }
        // `x <= x * a / d` for `0 < d <= a` when `x * a` does not wrap, as for a balance scaled by
        // a grown index.
        (_, SymExprKind::BinOp(SymBinOp::UDiv, product, divisor))
            if let Some((a, b)) = product_operands(product, facts)
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
        // `left <= round(a * b / d) <= round'(a' * b' / d) <= right`.
        _ if let Some(upper) = quotient_upper_bound(left, facts, bounds)
            && let Some(lower) = quotient_lower_bound(right, facts, bounds) =>
        {
            rounded_quotients_ordered(upper, lower, facts, bounds)
        }
        _ => false,
    }
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
    if let Some((x, y, denominator)) = quotient_operands(expr, facts, bounds) {
        return Some((x, y, denominator, Rounding::Floor));
    }
    // Wrapping only lowers the increment of a rounded-up quotient.
    if let Some((_, x, y, denominator)) = rounded_up_quotient(expr, facts, bounds) {
        return Some((x, y, denominator, Rounding::Ceil));
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
    let (x, y, denominator) = quotient_operands(quotient, facts, bounds)?;
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
        let denominator = upper.2;
        facts.zero.iter().any(|remainder| {
            let factors = match remainder.kind() {
                SymExprKind::TernOp(SymTernOp::MulMod, x, y, _) => Some((x, y)),
                SymExprKind::BinOp(SymBinOp::And, product, _) => product_operands(product, facts),
                _ => None,
            };
            factors.is_some_and(|(x, y)| {
                is_remainder_term(remainder, None, x, y, denominator)
                    && product_quotient_fits(x, y, denominator, facts, bounds)
                    && rounded_quotients_ordered(
                        upper,
                        (x, y, denominator, Rounding::Ceil),
                        facts,
                        bounds,
                    )
            })
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
    if let Some((x, y, denominator)) = quotient_operands(expr, facts, bounds) {
        if !quotient_fits(expr, facts, bounds) {
            return None;
        }
        let exact = has_remainder(x, y, denominator, facts) == Some(false);
        return Some((x, y, denominator, if exact { Rounding::Ceil } else { Rounding::Floor }));
    }
    // A rounded-up quotient that fits is `ceil(x * y / d)` when its increment does not wrap.
    if let Some((quotient, x, y, denominator)) = rounded_up_quotient(expr, facts, bounds) {
        let below_max = bounds.word_range(quotient).is_some_and(|(_, max)| max < U256::MAX)
            || facts.less_or_equal.contains(&(quotient, expr))
            || remainder_excludes_max(quotient, x, y, denominator, facts, bounds);
        return (quotient_fits(quotient, facts, bounds) && below_max).then_some((
            x,
            y,
            denominator,
            Rounding::Ceil,
        ));
    }
    // `q + 1 == floor(x * y / d) + 1 >= ceil(x * y / d)` when the increment does not wrap.
    let quotient = increment_operand(expr)?;
    let (x, y, denominator) = quotient_operands(quotient, facts, bounds)?;
    let below_max = bounds.word_range(quotient).is_some_and(|(_, max)| max < U256::MAX)
        // A word division `x * y / d` reaches `MAX` only for `d == 1`, which leaves no remainder.
        || ((matches!(quotient.kind(), SymExprKind::BinOp(SymBinOp::UDiv, ..))
            || remainder_excludes_max(quotient, x, y, denominator, facts, bounds))
            && has_remainder(x, y, denominator, facts) == Some(true));
    (quotient_fits(quotient, facts, bounds) && below_max).then_some((
        x,
        y,
        denominator,
        Rounding::Ceil,
    ))
}

/// Returns whether a nonzero remainder `r` of `x * y / d` implies that its quotient `q` is not
/// `MAX`: the product fits a word or has a power-of-two factor, or the facts deny
/// `r != 0 && q == MAX` for this division or for one of a product at least as large.
fn remainder_excludes_max<'a>(
    quotient: &'a SymExpr,
    x: &'a SymExpr,
    y: &'a SymExpr,
    denominator: &'a SymExpr,
    facts: &OrderFacts<'a>,
    bounds: &ConstraintContext,
) -> bool {
    bounds.mul_cannot_overflow_256(x, y)
        || [x, y].into_iter().any(|factor| factor.as_const().is_some_and(|k| k.is_power_of_two()))
        || facts.denied.iter().any(|terms| {
            let [first, second] = terms else { return false };
            [(first, second), (second, first)].into_iter().any(|(condition, at_max)| {
                at_max_operand(at_max).is_some_and(|other| {
                    quotient_operands(other, facts, bounds).is_some_and(|(a, b, divisor)| {
                        divisor == denominator
                            && is_remainder_check(condition, other, a, b, divisor, true)
                            && (other == quotient
                                || (quotient_fits(other, facts, bounds)
                                    && wide_product_less_or_equal(x, y, a, b, facts, bounds)))
                    })
                })
            })
        })
}

/// Returns `q` for `q == MAX`, also spelled `~q == 0` (the normal form of `q + 1 < q`) or
/// `MAX - 1 < q`.
fn at_max_operand(expr: &SymBoolExpr) -> Option<&SymExpr> {
    match expr.kind() {
        SymBoolExprKind::Cmp(SymCmpOp::Eq, left, right) => match (left.kind(), right.kind()) {
            (SymExprKind::Not(value), _) if right.as_const() == Some(U256::ZERO) => Some(value),
            _ if right.as_const() == Some(U256::MAX) => Some(left),
            _ => None,
        },
        SymBoolExprKind::Cmp(SymCmpOp::Ult, below, value)
            if below.as_const() == Some(U256::MAX - U256::ONE) =>
        {
            Some(value)
        }
        _ => None,
    }
}

/// Returns `(q, x, y, d)` when `expr` is the quotient `q` of `x * y / d` plus one for a nonzero
/// remainder, selected without a branch: `ite(r != 0, q + 1, q)` or `ite(r == 0, q, q + 1)`.
fn rounded_up_quotient<'a>(
    expr: &'a SymExpr,
    facts: &OrderFacts<'a>,
    bounds: &ConstraintContext,
) -> Option<(&'a SymExpr, &'a SymExpr, &'a SymExpr, &'a SymExpr)> {
    let SymExprKind::Ite(condition, then_value, else_value) = expr.kind() else { return None };
    [(then_value, else_value, true), (else_value, then_value, false)].into_iter().find_map(
        |(incremented, quotient, nonzero)| {
            if increment_operand(incremented) != Some(quotient) {
                return None;
            }
            let (x, y, denominator) = quotient_operands(quotient, facts, bounds)?;
            is_remainder_check(condition, quotient, x, y, denominator, nonzero).then_some((
                quotient,
                x,
                y,
                denominator,
            ))
        },
    )
}

/// Returns whether `condition` is `r != 0` (`r == 0` unless `nonzero`) for the remainder `r` of
/// the quotient's division, also joined with `d != 0`, which a nonzero remainder implies.
fn is_remainder_check(
    condition: &SymBoolExpr,
    quotient: &SymExpr,
    x: &SymExpr,
    y: &SymExpr,
    denominator: &SymExpr,
    nonzero: bool,
) -> bool {
    let is_remainder = |expr: &SymExpr| is_remainder_term(expr, Some(quotient), x, y, denominator);
    let is_zero_check = |check: &SymBoolExpr| check.zero_check_operand().is_some_and(is_remainder);
    match (condition.kind(), nonzero) {
        (SymBoolExprKind::Not(check), true) => is_zero_check(check),
        (SymBoolExprKind::And(terms), true) => {
            terms.iter().any(
                |term| matches!(term.kind(), SymBoolExprKind::Not(check) if is_zero_check(check)),
            ) && terms.iter().all(|term| {
                matches!(
                    term.kind(),
                    SymBoolExprKind::Not(check)
                        if is_zero_check(check) || check.zero_check_operand() == Some(denominator)
                )
            })
        }
        (_, false) => is_zero_check(condition),
        _ => false,
    }
}

/// Returns whether `expr` is the remainder of `x * y / d`: `mulmod(x, y, d)`, its fold
/// `(x * y) & (d - 1)` for a power of two `d`, or `P % d` next to the word quotient `P / d`.
fn is_remainder_term(
    expr: &SymExpr,
    quotient: Option<&SymExpr>,
    x: &SymExpr,
    y: &SymExpr,
    denominator: &SymExpr,
) -> bool {
    match expr.kind() {
        SymExprKind::TernOp(SymTernOp::MulMod, a, b, modulus) => {
            modulus == denominator && same_factors(a, b, x, y)
        }
        SymExprKind::BinOp(SymBinOp::And, product, mask) => {
            denominator.as_const().zip(mask.as_const()).is_some_and(|(denominator, mask)| {
                denominator.is_power_of_two() && mask == denominator - U256::ONE
            }) && is_product_of(product, x, y)
        }
        SymExprKind::BinOp(SymBinOp::URem, product, modulus) => {
            modulus == denominator
                && quotient.is_some_and(|quotient| {
                    matches!(
                        quotient.kind(),
                        SymExprKind::BinOp(SymBinOp::UDiv, numerator, _) if numerator == product
                    )
                })
        }
        _ => false,
    }
}

/// Returns whether the facts assert `mulmod(x, y, d) != 0`, or `false` when they assert it is zero.
fn has_remainder(
    x: &SymExpr,
    y: &SymExpr,
    denominator: &SymExpr,
    facts: &OrderFacts<'_>,
) -> Option<bool> {
    let is_remainder = |expr: &&SymExpr| is_remainder_term(expr, None, x, y, denominator);
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
        || is_quotient_product(left_a, left_b, right_a, right_b, facts, bounds)
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
    facts: &OrderFacts<'a>,
    bounds: &ConstraintContext,
) -> Option<(&'a SymExpr, &'a SymExpr, &'a SymExpr)> {
    match expr.kind() {
        SymExprKind::TernOp(SymTernOp::MulDiv, left, right, denominator) => {
            Some((left, right, denominator))
        }
        SymExprKind::BinOp(op @ (SymBinOp::UDiv | SymBinOp::Shr), product, operand) => {
            let denominator = if *op == SymBinOp::Shr { facts.power(operand)? } else { operand };
            let (left, right) = product_operands(product, facts)?;
            bounds.mul_cannot_overflow_256(left, right).then_some((left, right, denominator))
        }
        _ => None,
    }
}

/// Returns whether a quotient is known not to be truncated to a word.
fn quotient_fits(expr: &SymExpr, facts: &OrderFacts<'_>, bounds: &ConstraintContext) -> bool {
    match expr.kind() {
        SymExprKind::TernOp(SymTernOp::MulDiv, left, right, denominator) => {
            product_quotient_fits(left, right, denominator, facts, bounds)
        }
        _ => true,
    }
}

/// Returns whether `x * y / d` fits a word: the product does, or the facts assert
/// `mul_high(x, y) < d`, also as `mul_high(x, y) >> n == 0` for `d == 1 << n` or as a bound below
/// the range of `d`.
fn product_quotient_fits(
    x: &SymExpr,
    y: &SymExpr,
    denominator: &SymExpr,
    facts: &OrderFacts<'_>,
    bounds: &ConstraintContext,
) -> bool {
    bounds.mul_cannot_overflow_256(x, y)
        || facts
            .less_than
            .iter()
            .any(|(high, bound)| *bound == denominator && is_mul_high_of(high, x, y))
        || facts.less_or_equal.iter().any(|(high, _)| {
            is_mul_high_of(high, x, y)
                && bounds
                    .word_range(high)
                    .zip(bounds.word_range(denominator))
                    .is_some_and(|((_, high_max), (denominator_min, _))| high_max < denominator_min)
        })
        || facts.zero.iter().any(|shifted| {
            matches!(
                shifted.kind(),
                SymExprKind::BinOp(SymBinOp::Shr, high, shift)
                    if facts.power(shift) == Some(denominator)
                        && is_mul_high_of(high, x, y)
            )
        })
}

/// Returns whether `a * b` is `q * d` for a quotient `q` of `x * y / d`, where `(x, y)` are the
/// factors of the other product.
fn is_quotient_product<'a>(
    a: &'a SymExpr,
    b: &'a SymExpr,
    x: &SymExpr,
    y: &SymExpr,
    facts: &OrderFacts<'a>,
    bounds: &ConstraintContext,
) -> bool {
    quotient_product_parts(a, b, x, y, facts, bounds).is_some()
}

/// Returns `(q, d)` when `a * b` is `q * d` for a quotient `q` of `x * y / d`.
fn quotient_product_parts<'a>(
    a: &'a SymExpr,
    b: &'a SymExpr,
    x: &SymExpr,
    y: &SymExpr,
    facts: &OrderFacts<'a>,
    bounds: &ConstraintContext,
) -> Option<(&'a SymExpr, &'a SymExpr)> {
    [(a, b), (b, a)].into_iter().find(|(quotient, divisor)| {
        quotient_operands(quotient, facts, bounds).is_some_and(|(left, right, denominator)| {
            denominator == *divisor && same_factors(left, right, x, y)
        })
    })
}

/// Returns whether `expr` is a nonzero power of two: a constant, or `1 << n` with `n < 256`.
fn is_power_of_two(expr: &SymExpr, bounds: &ConstraintContext) -> bool {
    match expr.kind() {
        SymExprKind::BinOp(SymBinOp::Shl, one, shift) => {
            one.as_const() == Some(U256::ONE)
                && bounds.word_range(shift).is_some_and(|(_, max)| max < U256::from(256))
        }
        _ => expr.as_const().is_some_and(|value| value.is_power_of_two()),
    }
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

fn product_less_than_negation<'a>(
    expr: &'a SymBoolExpr,
    facts: &OrderFacts<'a>,
) -> Option<(&'a SymExpr, &'a SymExpr, &'a SymExpr, &'a SymExpr)> {
    let SymBoolExprKind::Not(value) = expr.kind() else { return None };
    let SymBoolExprKind::Cmp(SymCmpOp::Ult, left, right) = value.kind() else {
        return None;
    };
    let (left_a, left_b) = product_operands(left, facts)?;
    let (right_a, right_b) = product_operands(right, facts)?;
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

/// Returns the factors of a low word: of `a * b`, and of `x << n`, which is `x * 2**n`.
fn product_operands<'a>(
    expr: &'a SymExpr,
    facts: &OrderFacts<'a>,
) -> Option<(&'a SymExpr, &'a SymExpr)> {
    match expr.kind() {
        SymExprKind::BinOp(SymBinOp::Mul, left, right) => Some((left, right)),
        _ => {
            let (value, shift) = shifted_operands(expr)?;
            Some((value, facts.power(shift)?))
        }
    }
}

/// Returns whether `x * y < (u + 1) * c`, refuting `(u + 1) * c <= x * y`, which is how a strict
/// `u < x * y / c` normalizes. `x * y <= u * c < u * c + c` once `c` is nonzero and neither
/// `u + 1` nor the larger product wraps, so `x * y` cannot wrap either.
fn successor_product_exceeds<'a>(
    successor_product: &'a SymExpr,
    product: &'a SymExpr,
    facts: &OrderFacts<'a>,
    bounds: &ConstraintContext,
) -> bool {
    let (Some((successor, c)), Some((x, y))) =
        (product_operands(successor_product, facts), product_operands(product, facts))
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
