//! Bounded sparse-polynomial identity reasoning over EVM words.

use super::*;

pub(super) fn polynomial_identity(left: &SymExpr, right: &SymExpr) -> bool {
    if !polynomial_normalization_can_help(left) && !polynomial_normalization_can_help(right) {
        return false;
    }
    matches!(
        (Polynomial::from_expr(left), Polynomial::from_expr(right)),
        (Some(left), Some(right)) if left == right
    )
}

fn polynomial_normalization_can_help(expr: &SymExpr) -> bool {
    let crosses_sum_product_boundary = match expr.kind() {
        SymExprKind::BinOp(SymBinOp::Mul, left, right) => {
            matches!(left.kind(), SymExprKind::BinOp(SymBinOp::Add | SymBinOp::Sub, ..))
                || matches!(right.kind(), SymExprKind::BinOp(SymBinOp::Add | SymBinOp::Sub, ..))
        }
        SymExprKind::BinOp(SymBinOp::Add | SymBinOp::Sub, left, right) => {
            matches!(left.kind(), SymExprKind::BinOp(SymBinOp::Mul, ..))
                || matches!(right.kind(), SymExprKind::BinOp(SymBinOp::Mul, ..))
                || matches!(
                    left.kind(),
                    SymExprKind::BinOp(SymBinOp::Shl, _, shift)
                        if shift.as_const().is_some_and(|shift| shift < U256::from(256))
                )
                || matches!(
                    right.kind(),
                    SymExprKind::BinOp(SymBinOp::Shl, _, shift)
                        if shift.as_const().is_some_and(|shift| shift < U256::from(256))
                )
        }
        _ => false,
    };
    if !crosses_sum_product_boundary {
        return false;
    }

    fn ring_shape(
        expr: &SymExpr,
        shapes: &mut HashMap<SymExpr, Option<(usize, usize)>>,
        remaining: &mut usize,
    ) -> Option<(usize, usize)> {
        if let Some(shape) = shapes.get(expr) {
            return *shape;
        }
        if *remaining == 0 {
            return None;
        }
        *remaining -= 1;
        let shape = (|| match expr.kind() {
            SymExprKind::Const(_) | SymExprKind::Var(_) => Some((0, 0)),
            SymExprKind::BinOp(
                op @ (SymBinOp::Add | SymBinOp::Sub | SymBinOp::Mul),
                left,
                right,
            ) => {
                let left = ring_shape(left, shapes, remaining)?;
                let right = ring_shape(right, shapes, remaining)?;
                let operations = left.0.saturating_add(right.0).saturating_add(1);
                let multiplications = left
                    .1
                    .saturating_add(right.1)
                    .saturating_add(usize::from(*op == SymBinOp::Mul));
                Some((operations, multiplications))
            }
            SymExprKind::BinOp(SymBinOp::Shl, value, shift)
                if shift.as_const().is_some_and(|shift| shift < U256::from(256)) =>
            {
                let shape = ring_shape(value, shapes, remaining)?;
                Some((shape.0.saturating_add(1), shape.1.saturating_add(1)))
            }
            _ => None,
        })();
        shapes.insert(expr.clone(), shape);
        shape
    }

    let mut shapes = HashMap::default();
    let mut remaining = MAX_LOCAL_ANALYSIS_NODES;
    ring_shape(expr, &mut shapes, &mut remaining)
        .is_some_and(|(operations, multiplications)| operations > 1 && multiplications > 0)
}

// Keep distributive expansion predictably bounded. The motivating accounting identity needs two
// terms with two factors; these limits leave ample room for ordinary identities without allowing
// adversarial expressions to explode.
const MAX_POLYNOMIAL_TERMS: usize = 32;
const MAX_MONOMIAL_FACTORS: usize = 8;
const MAX_POLYNOMIAL_PRODUCTS: usize = 256;

type Monomial = Vec<SymExpr>;

/// A sparse polynomial over the EVM word ring Z/(2^256).
///
/// Addition, subtraction, and multiplication of EVM words obey the ring laws even when they
/// wrap. Canonicalizing small expressions here lets the solver recognize nonlinear algebraic
/// identities without replacing bit-vector semantics with unbounded integer arithmetic.
#[derive(Clone, PartialEq, Eq)]
struct Polynomial {
    terms: HashMap<Monomial, U256>,
}

impl Polynomial {
    fn from_expr(expr: &SymExpr) -> Option<Self> {
        let mut remaining = MAX_LOCAL_ANALYSIS_NODES;
        Self::from_expr_cached(expr, &mut HashMap::default(), &mut remaining)
    }

    fn from_expr_cached(
        expr: &SymExpr,
        polynomials: &mut HashMap<SymExpr, Option<Self>>,
        remaining: &mut usize,
    ) -> Option<Self> {
        if let Some(polynomial) = polynomials.get(expr) {
            return polynomial.clone();
        }
        if *remaining == 0 {
            polynomials.insert(expr.clone(), None);
            return None;
        }
        *remaining -= 1;
        let polynomial =
            (|| match expr.kind() {
                SymExprKind::Const(value) => Some(Self::constant(*value)),
                SymExprKind::BinOp(SymBinOp::Add, left, right) => {
                    Self::from_expr_cached(left, polynomials, remaining)?
                        .add(Self::from_expr_cached(right, polynomials, remaining)?)
                }
                SymExprKind::BinOp(SymBinOp::Sub, left, right) => {
                    Self::from_expr_cached(left, polynomials, remaining)?
                        .sub(Self::from_expr_cached(right, polynomials, remaining)?)
                }
                SymExprKind::BinOp(SymBinOp::Mul, left, right) => {
                    Self::from_expr_cached(left, polynomials, remaining)?
                        .mul(Self::from_expr_cached(right, polynomials, remaining)?)
                }
                SymExprKind::BinOp(SymBinOp::Shl, value, shift)
                    if let Some(shift) = shift.as_const()
                        && shift < U256::from(256) =>
                {
                    let coefficient = U256::ONE << usize::try_from(shift).ok()?;
                    Self::from_expr_cached(value, polynomials, remaining)?
                        .mul(Self::constant(coefficient))
                }
                _ => {
                    let terms = HashMap::from_iter([(vec![expr.clone()], U256::ONE)]);
                    Some(Self { terms })
                }
            })();
        polynomials.insert(expr.clone(), polynomial.clone());
        polynomial
    }

    fn constant(value: U256) -> Self {
        let mut terms = HashMap::default();
        if !value.is_zero() {
            terms.insert(Vec::new(), value);
        }
        Self { terms }
    }

    fn add(mut self, right: Self) -> Option<Self> {
        for (monomial, coefficient) in right.terms {
            self.add_term(monomial, coefficient);
            if self.terms.len() > MAX_POLYNOMIAL_TERMS {
                return None;
            }
        }
        Some(self)
    }

    fn sub(mut self, right: Self) -> Option<Self> {
        for (monomial, coefficient) in right.terms {
            self.add_term(monomial, U256::ZERO.wrapping_sub(coefficient));
            if self.terms.len() > MAX_POLYNOMIAL_TERMS {
                return None;
            }
        }
        Some(self)
    }

    fn mul(self, right: Self) -> Option<Self> {
        let products = self.terms.len().checked_mul(right.terms.len())?;
        if products > MAX_POLYNOMIAL_PRODUCTS {
            return None;
        }

        let mut out = Self { terms: HashMap::default() };
        for (left_monomial, left_coefficient) in &self.terms {
            for (right_monomial, right_coefficient) in &right.terms {
                let factor_count = left_monomial.len().checked_add(right_monomial.len())?;
                if factor_count > MAX_MONOMIAL_FACTORS {
                    return None;
                }
                let mut monomial = Vec::with_capacity(factor_count);
                monomial.extend(left_monomial.iter().cloned());
                monomial.extend(right_monomial.iter().cloned());
                SymExpr::sort_interned_factors(&mut monomial);
                out.add_term(monomial, left_coefficient.wrapping_mul(*right_coefficient));
                if out.terms.len() > MAX_POLYNOMIAL_TERMS {
                    return None;
                }
            }
        }
        Some(out)
    }

    fn add_term(&mut self, monomial: Monomial, coefficient: U256) {
        if coefficient.is_zero() {
            return;
        }
        let coefficient =
            self.terms.get(&monomial).copied().unwrap_or_default().wrapping_add(coefficient);
        if coefficient.is_zero() {
            self.terms.remove(&monomial);
        } else {
            self.terms.insert(monomial, coefficient);
        }
    }
}
