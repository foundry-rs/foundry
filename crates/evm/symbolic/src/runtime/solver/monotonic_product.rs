use super::{normalize::mul_cannot_overflow_256, *};

/// Returns whether monotonic product facts make these constraints unsatisfiable.
#[cfg(test)]
pub(crate) fn product_monotonic_unsat(constraints: &[BoolExpr]) -> bool {
    let constraints = normalize_constraints_for_solver(constraints);
    product_monotonic_unsat_normalized(&constraints)
}

/// Returns whether normalized monotonic product facts make constraints unsatisfiable.
pub(crate) fn product_monotonic_unsat_normalized(constraints: &[BoolExpr]) -> bool {
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

/// Collects simple unsigned ordering facts from normalized constraints.
pub(crate) fn collect_order_facts(
    expr: &BoolExpr,
    less_than: &mut HashSet<(Expr, Expr)>,
    positive: &mut HashSet<Expr>,
) {
    match expr {
        BoolExpr::And(values) => {
            for value in values.iter() {
                collect_order_facts(value, less_than, positive);
            }
        }
        BoolExpr::Cmp(BoolExprOp::Ult, left, right) => {
            less_than.insert(((**left).clone(), (**right).clone()));
            if matches!(left.as_ref(), Expr::Const(value) if value.is_zero()) {
                positive.insert((**right).clone());
            }
        }
        BoolExpr::Cmp(BoolExprOp::Ugt, left, right) => {
            less_than.insert(((**right).clone(), (**left).clone()));
            if matches!(right.as_ref(), Expr::Const(value) if value.is_zero()) {
                positive.insert((**left).clone());
            }
        }
        BoolExpr::Const(_) | BoolExpr::Not(_) | BoolExpr::Eq(_, _) | BoolExpr::Cmp(_, _, _) => {}
    }
}

/// Extracts `!(a * b < c * d)` as product operands.
pub(crate) fn product_less_than_negation(expr: &BoolExpr) -> Option<(&Expr, &Expr, &Expr, &Expr)> {
    let BoolExpr::Not(value) = expr else { return None };
    let BoolExpr::Cmp(BoolExprOp::Ult, left, right) = value.as_ref() else { return None };
    let (left_a, left_b) = mul_operands(left)?;
    let (right_a, right_b) = mul_operands(right)?;
    Some((left_a, left_b, right_a, right_b))
}

/// Returns whether known facts imply `left_a * left_b < right_a * right_b`.
pub(crate) fn product_less_than_known(
    left_a: &Expr,
    left_b: &Expr,
    right_a: &Expr,
    right_b: &Expr,
    less_than: &HashSet<(Expr, Expr)>,
    positive: &HashSet<Expr>,
) -> bool {
    product_less_than_known_ordered(left_a, left_b, right_a, right_b, less_than, positive)
        || product_less_than_known_ordered(left_b, left_a, right_a, right_b, less_than, positive)
        || product_less_than_known_ordered(left_a, left_b, right_b, right_a, less_than, positive)
        || product_less_than_known_ordered(left_b, left_a, right_b, right_a, less_than, positive)
}

/// Checks the ordered monotonicity case `0 < a < c && 0 < b < d`.
pub(crate) fn product_less_than_known_ordered(
    left_a: &Expr,
    left_b: &Expr,
    right_a: &Expr,
    right_b: &Expr,
    less_than: &HashSet<(Expr, Expr)>,
    positive: &HashSet<Expr>,
) -> bool {
    positive.contains(left_a)
        && positive.contains(left_b)
        && less_than.contains(&(left_a.clone(), right_a.clone()))
        && less_than.contains(&(left_b.clone(), right_b.clone()))
        && mul_cannot_overflow_256(left_a, left_b)
        && mul_cannot_overflow_256(right_a, right_b)
}

/// Returns the operands for an unsigned multiplication expression.
pub(crate) fn mul_operands(expr: &Expr) -> Option<(&Expr, &Expr)> {
    match expr {
        Expr::Op(ExprOp::Mul, left, right) => Some((left, right)),
        _ => None,
    }
}
