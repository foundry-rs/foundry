use super::{normalize::mul_cannot_overflow_256, *};

type LessThanFacts<'a> = HashSet<(&'a Expr, &'a Expr)>;
type PositiveFacts<'a> = HashSet<&'a Expr>;

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

fn collect_order_facts<'a>(
    expr: &'a BoolExpr,
    less_than: &mut LessThanFacts<'a>,
    positive: &mut PositiveFacts<'a>,
) {
    match expr.as_inner() {
        BoolExprInner::And(values) => {
            for value in values.iter() {
                collect_order_facts(value, less_than, positive);
            }
        }
        BoolExprInner::Cmp(BoolExprOp::Ult, left, right) => {
            less_than.insert((left, right));
            if left.as_const().is_some_and(|value| value.is_zero()) {
                positive.insert(right);
            }
        }
        BoolExprInner::Cmp(BoolExprOp::Ugt, left, right) => {
            less_than.insert((right, left));
            if right.as_const().is_some_and(|value| value.is_zero()) {
                positive.insert(left);
            }
        }
        BoolExprInner::Const(_)
        | BoolExprInner::Not(_)
        | BoolExprInner::Eq(_, _)
        | BoolExprInner::Cmp(_, _, _) => {}
    }
}

fn product_less_than_negation(expr: &BoolExpr) -> Option<(&Expr, &Expr, &Expr, &Expr)> {
    let BoolExprInner::Not(value) = expr.as_inner() else { return None };
    let BoolExprInner::Cmp(BoolExprOp::Ult, left, right) = value.as_inner() else {
        return None;
    };
    let (left_a, left_b) = mul_operands(left)?;
    let (right_a, right_b) = mul_operands(right)?;
    Some((left_a, left_b, right_a, right_b))
}

fn product_less_than_known<'a>(
    left_a: &'a Expr,
    left_b: &'a Expr,
    right_a: &'a Expr,
    right_b: &'a Expr,
    less_than: &LessThanFacts<'a>,
    positive: &PositiveFacts<'a>,
) -> bool {
    product_less_than_known_ordered(left_a, left_b, right_a, right_b, less_than, positive)
        || product_less_than_known_ordered(left_b, left_a, right_a, right_b, less_than, positive)
        || product_less_than_known_ordered(left_a, left_b, right_b, right_a, less_than, positive)
        || product_less_than_known_ordered(left_b, left_a, right_b, right_a, less_than, positive)
}

fn product_less_than_known_ordered<'a>(
    left_a: &'a Expr,
    left_b: &'a Expr,
    right_a: &'a Expr,
    right_b: &'a Expr,
    less_than: &LessThanFacts<'a>,
    positive: &PositiveFacts<'a>,
) -> bool {
    positive.contains(left_a)
        && positive.contains(left_b)
        && less_than.contains(&(left_a, right_a))
        && less_than.contains(&(left_b, right_b))
        && mul_cannot_overflow_256(left_a, left_b)
        && mul_cannot_overflow_256(right_a, right_b)
}

fn mul_operands(expr: &Expr) -> Option<(&Expr, &Expr)> {
    match expr.as_inner() {
        ExprInner::Op(ExprOp::Mul, left, right) => Some((left, right)),
        _ => None,
    }
}
