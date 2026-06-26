use super::*;

type LessThanFacts<'a> = HashSet<(&'a SymExpr, &'a SymExpr)>;
type PositiveFacts<'a> = HashSet<&'a SymExpr>;

/// Returns whether monotonic product facts make these constraints unsatisfiable.
#[cfg(test)]
pub(crate) fn product_monotonic_unsat(constraints: &[SymBoolExpr]) -> bool {
    let constraints = normalize_constraints_for_solver(constraints);
    product_monotonic_unsat_normalized(&constraints)
}

/// Returns whether normalized monotonic product facts make constraints unsatisfiable.
pub(crate) fn product_monotonic_unsat_normalized(constraints: &[SymBoolExpr]) -> bool {
    let mut less_than = HashSet::default();
    let mut positive = HashSet::default();
    for constraint in constraints {
        collect_order_facts(constraint, &mut less_than, &mut positive);
    }

    constraints.iter().any(|constraint| {
        product_less_than_negation(constraint).is_some_and(|(left_a, left_b, right_a, right_b)| {
            product_less_than_known(left_a, left_b, right_a, right_b, &less_than, &positive)
        })
    })
}

fn collect_order_facts<'a>(
    expr: &'a SymBoolExpr,
    less_than: &mut LessThanFacts<'a>,
    positive: &mut PositiveFacts<'a>,
) {
    match expr.as_inner() {
        SymBoolExprInner::And(values) => {
            for value in values.iter() {
                collect_order_facts(value, less_than, positive);
            }
        }
        SymBoolExprInner::Cmp(SymBoolExprOp::Ult, left, right) => {
            less_than.insert((left, right));
            if left.as_const().is_some_and(|value| value.is_zero()) {
                positive.insert(right);
            }
        }
        SymBoolExprInner::Cmp(SymBoolExprOp::Ugt, left, right) => {
            less_than.insert((right, left));
            if right.as_const().is_some_and(|value| value.is_zero()) {
                positive.insert(left);
            }
        }
        SymBoolExprInner::Const(_)
        | SymBoolExprInner::Not(_)
        | SymBoolExprInner::Eq(_, _)
        | SymBoolExprInner::Cmp(_, _, _) => {}
    }
}

fn product_less_than_negation(
    expr: &SymBoolExpr,
) -> Option<(&SymExpr, &SymExpr, &SymExpr, &SymExpr)> {
    let SymBoolExprInner::Not(value) = expr.as_inner() else { return None };
    let SymBoolExprInner::Cmp(SymBoolExprOp::Ult, left, right) = value.as_inner() else {
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
) -> bool {
    product_less_than_known_ordered(left_a, left_b, right_a, right_b, less_than, positive)
        || product_less_than_known_ordered(left_b, left_a, right_a, right_b, less_than, positive)
        || product_less_than_known_ordered(left_a, left_b, right_b, right_a, less_than, positive)
        || product_less_than_known_ordered(left_b, left_a, right_b, right_a, less_than, positive)
}

fn product_less_than_known_ordered<'a>(
    left_a: &'a SymExpr,
    left_b: &'a SymExpr,
    right_a: &'a SymExpr,
    right_b: &'a SymExpr,
    less_than: &LessThanFacts<'a>,
    positive: &PositiveFacts<'a>,
) -> bool {
    positive.contains(left_a)
        && positive.contains(left_b)
        && less_than.contains(&(left_a, right_a))
        && less_than.contains(&(left_b, right_b))
        && left_a.mul_cannot_overflow_256(left_b)
        && right_a.mul_cannot_overflow_256(right_b)
}

fn mul_operands(expr: &SymExpr) -> Option<(&SymExpr, &SymExpr)> {
    match expr.as_inner() {
        SymExprInner::Op(SymExprOp::Mul, left, right) => Some((left, right)),
        _ => None,
    }
}
