use super::{opt::ConstraintContext, *};

type LessThanFacts<'a> = HashSet<(&'a SymExpr, &'a SymExpr)>;
type LessOrEqualFacts<'a> = HashSet<(&'a SymExpr, &'a SymExpr)>;
type PositiveFacts<'a> = HashSet<&'a SymExpr>;

#[derive(Default)]
struct OrderFacts<'a> {
    less_than: LessThanFacts<'a>,
    less_or_equal: LessOrEqualFacts<'a>,
    positive: PositiveFacts<'a>,
}

/// Returns whether monotonic product facts make these constraints unsatisfiable.
#[cfg(test)]
pub(crate) fn product_monotonic_unsat(cx: &mut SymCx, constraints: &[SymBoolExpr]) -> bool {
    let constraints = normalize_constraints_for_solver(cx, constraints);
    product_monotonic_unsat_normalized(&constraints)
}

/// Returns whether normalized monotonic product facts make constraints unsatisfiable.
pub(crate) fn product_monotonic_unsat_normalized(constraints: &[SymBoolExpr]) -> bool {
    let facts = order_facts(constraints.iter());
    let bounds = ConstraintContext::new(constraints);

    constraints.iter().any(|constraint| {
        reversed_strict_comparison(constraint)
            .is_some_and(|(left, right)| expr_less_or_equal(right, left, &facts, &bounds))
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
pub(super) fn remove_implied_monotonic_constraints(
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

    match (left.kind(), right.kind()) {
        (
            SymExprKind::BinOp(SymBinOp::UDiv, left_num, left_den),
            SymExprKind::BinOp(SymBinOp::UDiv, right_num, right_den),
        ) if left_den == right_den => expr_less_or_equal(left_num, right_num, facts, bounds),
        (
            SymExprKind::BinOp(SymBinOp::Mul, left_a, left_b),
            SymExprKind::BinOp(SymBinOp::Mul, right_a, right_b),
        ) if bounds.mul_cannot_overflow_256(left_a, left_b)
            && bounds.mul_cannot_overflow_256(right_a, right_b) =>
        {
            product_less_or_equal_known(left_a, left_b, right_a, right_b, facts, bounds)
        }
        _ => false,
    }
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
