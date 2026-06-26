use super::*;

/// Normalizes path constraints into an equivalent, solver-friendlier form.
pub(crate) fn normalize_constraints_for_solver(constraints: &[SymBoolExpr]) -> Vec<SymBoolExpr> {
    let normalized = normalize_constraint_batch(
        constraints.iter().cloned().map(normalize_bool_for_solver),
        constraints.len(),
    );
    if matches!(normalized.as_slice(), [expr] if expr.as_const() == Some(false)) {
        return normalized;
    }

    let context = ConstraintContext::new(&normalized);
    let normalized_len = normalized.len();
    normalize_constraint_batch(
        normalized.into_iter().map(|constraint| context.normalize_bool(constraint)),
        normalized_len,
    )
}

fn normalize_constraint_batch(
    constraints: impl IntoIterator<Item = SymBoolExpr>,
    capacity: usize,
) -> Vec<SymBoolExpr> {
    let mut normalized = Vec::with_capacity(capacity);
    for constraint in constraints {
        if constraint.as_const() == Some(false) {
            return vec![SymBoolExpr::constant(false)];
        }
        collect_normalized_conjunct(constraint, &mut normalized);
    }
    normalized.sort();
    normalized.dedup();
    normalized
}

fn collect_normalized_conjunct(expr: SymBoolExpr, out: &mut Vec<SymBoolExpr>) {
    match expr.kind() {
        SymBoolExprKind::Const(true) => {}
        SymBoolExprKind::And(values) => {
            for value in values.iter().cloned() {
                collect_normalized_conjunct(value, out);
            }
        }
        _ => out.push(expr),
    }
}

/// Normalizes one boolean expression into an equivalent, solver-friendlier form.
pub(crate) fn normalize_bool_for_solver(expr: SymBoolExpr) -> SymBoolExpr {
    expr.fold(&mut normalize_bool_node_for_solver)
}

fn normalize_bool_node_for_solver(expr: SymBoolExpr) -> SymBoolExpr {
    if let Some(normalized) = expr.normalize_udiv_for_solver() {
        return normalized;
    }

    match expr.into_kind() {
        SymBoolExprKind::Not(value) => value.not(),
        SymBoolExprKind::And(values) => SymBoolExpr::and(values.iter().cloned().collect()),
        SymBoolExprKind::Eq(left, right) => {
            let normalized =
                SymBoolExpr::eq(normalize_expr_for_solver(left), normalize_expr_for_solver(right));
            normalized.normalize_udiv_for_solver().unwrap_or(normalized)
        }
        SymBoolExprKind::Cmp(op, left, right) => {
            let normalized = SymBoolExpr::cmp(
                op,
                normalize_expr_for_solver(left),
                normalize_expr_for_solver(right),
            );
            normalized.normalize_udiv_for_solver().unwrap_or(normalized)
        }
        SymBoolExprKind::Const(value) => SymBoolExpr::constant(value),
    }
}

/// Simple facts learned from the normalized conjunction currently being queried.
#[derive(Default)]
struct ConstraintContext {
    upper_bounds: HashMap<SymExpr, U256>,
}

impl ConstraintContext {
    fn new(constraints: &[SymBoolExpr]) -> Self {
        let mut context = Self::default();
        for constraint in constraints {
            context.record_upper_bound_constraint(constraint);
        }
        context
    }

    fn upper_bound(&self, expr: &SymExpr) -> Option<U256> {
        self.upper_bounds.get(expr).copied()
    }

    fn normalize_bool(&self, expr: SymBoolExpr) -> SymBoolExpr {
        match expr.kind() {
            _ if expr.zero_check_operand().is_some_and(|left| self.word_bool_always_true(left)) => {
                SymBoolExpr::constant(false)
            }
            SymBoolExprKind::Not(value)
                if value
                    .zero_check_operand()
                    .is_some_and(|left| self.word_bool_always_true(left)) =>
            {
                SymBoolExpr::constant(true)
            }
            _ => expr,
        }
    }

    fn record_upper_bound_constraint(&mut self, constraint: &SymBoolExpr) {
        if let Some((expr, bound)) = self.upper_bound_constraint(constraint) {
            self.record_upper_bound(expr.clone(), bound);
        }
    }

    fn record_upper_bound(&mut self, expr: SymExpr, bound: U256) {
        self.upper_bounds
            .entry(expr)
            .and_modify(|existing| *existing = (*existing).min(bound))
            .or_insert(bound);
    }

    fn upper_bound_constraint<'a>(
        &self,
        constraint: &'a SymBoolExpr,
    ) -> Option<(&'a SymExpr, U256)> {
        match constraint.kind() {
            SymBoolExprKind::Eq(left, right) => match (left.as_const(), right.as_const()) {
                (_, Some(value)) => Some((left, value)),
                (Some(value), _) => Some((right, value)),
                _ => None,
            },
            SymBoolExprKind::Cmp(op, left, right) => {
                match (*op, left.as_const(), right.as_const()) {
                    (SymBoolExprOp::Ult, _, Some(bound)) => {
                        (!bound.is_zero()).then(|| (left, bound - U256::from(1)))
                    }
                    (SymBoolExprOp::Ule, _, Some(bound)) => Some((left, bound)),
                    (SymBoolExprOp::Ugt, Some(bound), _) => {
                        (!bound.is_zero()).then(|| (right, bound - U256::from(1)))
                    }
                    (SymBoolExprOp::Uge, Some(bound), _) => Some((right, bound)),
                    _ => None,
                }
            }
            SymBoolExprKind::Not(value) => match value.kind() {
                SymBoolExprKind::Cmp(op, left, right) => {
                    match (*op, left.as_const(), right.as_const()) {
                        (SymBoolExprOp::Ugt, _, Some(bound)) => Some((left, bound)),
                        (SymBoolExprOp::Uge, _, Some(bound)) => {
                            (!bound.is_zero()).then(|| (left, bound - U256::from(1)))
                        }
                        (SymBoolExprOp::Ult, Some(bound), _) => Some((right, bound)),
                        (SymBoolExprOp::Ule, Some(bound), _) => {
                            (!bound.is_zero()).then(|| (right, bound - U256::from(1)))
                        }
                        _ => None,
                    }
                }
                _ => None,
            },
            SymBoolExprKind::Const(_) | SymBoolExprKind::And(_) => None,
        }
    }
}

/// Normalizes one word expression into an equivalent, solver-friendlier form.
pub(crate) fn normalize_expr_for_solver(expr: SymExpr) -> SymExpr {
    expr.fold(&mut normalize_expr_node_for_solver)
}

fn normalize_expr_node_for_solver(expr: SymExpr) -> SymExpr {
    if let Some(rebuilt) = rebuild_word_from_extracted_byte_terms(&expr)
        && rebuilt != expr
    {
        return normalize_expr_for_solver(rebuilt);
    }

    match expr.kind() {
        SymExprKind::Op(op, left, right)
            if matches!(
                op,
                SymExprOp::Add | SymExprOp::Mul | SymExprOp::And | SymExprOp::Or | SymExprOp::Xor
            ) && right < left =>
        {
            let SymExprKind::Op(op, left, right) = expr.into_kind() else { unreachable!() };
            SymExpr::op(op, right, left)
        }
        SymExprKind::Ite(_, _, _) => {
            let SymExprKind::Ite(cond, left, right) = expr.into_kind() else { unreachable!() };
            normalize_ite_expr_for_solver(cond, left, right)
        }
        _ => expr,
    }
}

fn normalize_ite_expr_for_solver(cond: SymBoolExpr, left: SymExpr, right: SymExpr) -> SymExpr {
    let cond = normalize_bool_for_solver(cond);
    if left == right {
        return left;
    }
    if let Some(condition) = guarded_self_div_word_condition(&cond, &left, &right) {
        return SymExpr::bool_word(condition);
    }
    if left.as_const() == Some(U256::from(1))
        && right.normalized_bool_word_condition().as_ref() == Some(&cond)
    {
        return right;
    }
    if right.as_const().is_some_and(|value| value.is_zero())
        && left.normalized_bool_word_condition().as_ref() == Some(&cond)
    {
        return left;
    }
    SymExpr::ite(cond, left, right)
}

/// Returns the boolean represented by `a == 0 ? 0 : a / a`.
fn guarded_self_div_word_condition(
    cond: &SymBoolExpr,
    left: &SymExpr,
    right: &SymExpr,
) -> Option<SymBoolExpr> {
    if left.as_const().is_some_and(|value| value.is_zero())
        && self_div_expr_matches_zero_check(cond, right)
    {
        return Some(cond.clone().not());
    }
    None
}

/// Returns whether `expr` is `a / a` for the operand guarded by `cond`.
fn self_div_expr_matches_zero_check(cond: &SymBoolExpr, expr: &SymExpr) -> bool {
    let Some(zero_operand) = cond.zero_check_operand() else { return false };
    let Some((numerator, denominator)) = expr.udiv_operands() else { return false };
    numerator == zero_operand && denominator == zero_operand
}

/// Rebuilds a word from OR-ed byte-extraction terms when the source is recoverable.
pub(crate) fn rebuild_word_from_extracted_byte_terms(expr: &SymExpr) -> Option<SymExpr> {
    let mut terms = Vec::new();
    collect_or_terms(expr, &mut terms);
    if terms.len() <= 1 {
        return None;
    }

    let mut source = None;
    let mut seen = [false; 32];
    for term in terms {
        if term.as_const().is_some_and(|value| value.is_zero()) {
            continue;
        }
        let (term_source, index) = extracted_shifted_byte_term(term)?;
        match &source {
            Some(source) if source != &term_source => return None,
            Some(_) => {}
            None => source = Some(term_source),
        }
        seen[index] = true;
    }

    let source = source?;
    for (index, seen) in seen.into_iter().enumerate() {
        if !seen && source.known_byte(index) != Some(0) {
            return None;
        }
    }
    Some(source)
}

/// Flattens nested bitwise-OR expressions into their leaf terms.
pub(crate) fn collect_or_terms<'a>(expr: &'a SymExpr, terms: &mut Vec<&'a SymExpr>) {
    match expr.kind() {
        SymExprKind::Op(SymExprOp::Or, left, right) => {
            collect_or_terms(left, terms);
            collect_or_terms(right, terms);
        }
        _ => terms.push(expr),
    }
}

/// Returns the source word and byte index for one shifted extracted-byte term.
pub(crate) fn extracted_shifted_byte_term(term: &SymExpr) -> Option<(SymExpr, usize)> {
    match term.kind() {
        SymExprKind::Op(SymExprOp::Shl, byte, shift) => {
            let shift = shift.as_const()?;
            let shift = usize::try_from(shift).expect("checked byte shift");
            if shift % 8 != 0 || shift > 248 {
                return None;
            }
            let index = 31 - shift / 8;
            let source = extracted_unshifted_byte_source(byte, index)?;
            Some((source, index))
        }
        _ => extracted_unshifted_byte_source(term, 31).map(|source| (source, 31)),
    }
}

/// Returns the source word for an unshifted byte extraction at `index`.
pub(crate) fn extracted_unshifted_byte_source(term: &SymExpr, index: usize) -> Option<SymExpr> {
    let expr = strip_low_byte_mask(term)?;
    if index == 31 {
        return Some(expr.clone());
    }
    let SymExprKind::Op(SymExprOp::Shr, source, shift) = expr.kind() else { return None };
    let shift = shift.as_const()?;
    (shift == U256::from((31 - index) * 8)).then(|| source.clone())
}

impl SymExpr {
    fn add_cannot_overflow_256(&self, right: &Self) -> bool {
        self.unsigned_bits().max(right.unsigned_bits()).saturating_add(1) <= 256
    }

    fn word_bool_always_true(&self) -> bool {
        ConstraintContext::default().word_bool_always_true(self)
    }

    pub(crate) fn mul_cannot_overflow_256(&self, right: &Self) -> bool {
        self.unsigned_bits().saturating_add(right.unsigned_bits()) <= 256
    }

    fn unsigned_bits(&self) -> usize {
        match self.kind() {
            SymExprKind::Const(value) => value.bit_len().max(1),
            SymExprKind::Op(SymExprOp::And, left, right) => {
                if let Some(mask) = right.as_const() {
                    left.unsigned_bits().min(mask.bit_len())
                } else if let Some(mask) = left.as_const() {
                    right.unsigned_bits().min(mask.bit_len())
                } else {
                    256
                }
            }
            SymExprKind::Op(SymExprOp::Add, left, right) => {
                left.unsigned_bits().max(right.unsigned_bits()).saturating_add(1).min(256)
            }
            SymExprKind::Op(SymExprOp::Mul, left, right) => {
                left.unsigned_bits().saturating_add(right.unsigned_bits()).min(256)
            }
            SymExprKind::Op(SymExprOp::UDiv, left, _) => left.unsigned_bits(),
            SymExprKind::AddMod { modulus, .. } | SymExprKind::MulMod { modulus, .. } => {
                modulus.unsigned_bits()
            }
            SymExprKind::Ite(_, left, right) => left.unsigned_bits().max(right.unsigned_bits()),
            _ => 256,
        }
    }
}

impl SymBoolExpr {
    fn normalize_udiv_for_solver(&self) -> Option<Self> {
        match self.kind() {
            SymBoolExprKind::Eq(left, right)
                if right.as_const().is_some_and(|value| value.is_zero()) =>
            {
                left.normalized_bool_word_condition().map(Self::not).or_else(|| {
                    if left.word_bool_always_true() {
                        Some(Self::constant(false))
                    } else {
                        Self::normalize_udiv_eq_zero(left, &SymExpr::zero())
                    }
                })
            }
            SymBoolExprKind::Eq(left, right)
                if left.as_const().is_some_and(|value| value.is_zero()) =>
            {
                right.normalized_bool_word_condition().map(Self::not).or_else(|| {
                    if right.word_bool_always_true() {
                        Some(Self::constant(false))
                    } else {
                        Self::normalize_udiv_eq_zero(&SymExpr::zero(), right)
                    }
                })
            }
            SymBoolExprKind::Eq(left, right) if right.as_const() == Some(U256::from(1)) => {
                left.normalized_bool_word_condition()
            }
            SymBoolExprKind::Eq(left, right) if left.as_const() == Some(U256::from(1)) => {
                right.normalized_bool_word_condition()
            }
            SymBoolExprKind::Not(value) => match value.kind() {
                SymBoolExprKind::Cmp(op, left, right) => {
                    Self::normalize_add_overflow_cmp(*op, left, right)
                        .map(Self::not)
                        .or_else(|| Self::normalize_udiv_cmp(*op, left, right).map(Self::not))
                }
                SymBoolExprKind::Eq(left, right)
                    if right.as_const().is_some_and(|value| value.is_zero()) =>
                {
                    if left.word_bool_always_true() {
                        Some(Self::constant(true))
                    } else {
                        Self::normalize_udiv_eq_zero(left, &SymExpr::zero()).map(Self::not)
                    }
                }
                SymBoolExprKind::Eq(left, right)
                    if left.as_const().is_some_and(|value| value.is_zero()) =>
                {
                    if right.word_bool_always_true() {
                        Some(Self::constant(true))
                    } else {
                        Self::normalize_udiv_eq_zero(&SymExpr::zero(), right).map(Self::not)
                    }
                }
                SymBoolExprKind::Eq(left, right) => {
                    Self::normalize_udiv_eq_zero(left, right).map(Self::not)
                }
                _ => None,
            },
            SymBoolExprKind::Eq(left, right) => Self::normalize_udiv_eq_zero(left, right),
            SymBoolExprKind::Cmp(op, left, right) => {
                Self::normalize_add_overflow_cmp(*op, left, right)
                    .or_else(|| Self::normalize_udiv_cmp(*op, left, right))
            }
            SymBoolExprKind::Const(_) | SymBoolExprKind::And(_) => None,
        }
    }

    fn zero_check_operand(&self) -> Option<&SymExpr> {
        match self.kind() {
            SymBoolExprKind::Eq(left, right)
                if right.as_const().is_some_and(|value| value.is_zero()) =>
            {
                Some(left)
            }
            SymBoolExprKind::Eq(left, right)
                if left.as_const().is_some_and(|value| value.is_zero()) =>
            {
                Some(right)
            }
            _ => None,
        }
    }

    fn normalize_add_overflow_cmp(
        op: SymBoolExprOp,
        left: &SymExpr,
        right: &SymExpr,
    ) -> Option<Self> {
        match op {
            SymBoolExprOp::Ugt if left.add_overflow_check(right) => Some(Self::constant(false)),
            SymBoolExprOp::Ult if right.add_overflow_check(left) => Some(Self::constant(false)),
            _ => None,
        }
    }

    fn normalize_udiv_eq_zero(left: &SymExpr, right: &SymExpr) -> Option<Self> {
        if right.as_const().is_some_and(|value| value.is_zero())
            && let Some(condition) = left.normalize_eq_zero_for_solver()
        {
            return Some(condition);
        }
        if left.as_const().is_some_and(|value| value.is_zero())
            && let Some(condition) = right.normalize_eq_zero_for_solver()
        {
            return Some(condition);
        }
        None
    }

    fn normalize_udiv_cmp(op: SymBoolExprOp, left: &SymExpr, right: &SymExpr) -> Option<Self> {
        match (op, left.as_const(), right.as_const()) {
            (SymBoolExprOp::Ugt, _, Some(value)) if value.is_zero() => {
                left.normalize_ne_zero_for_solver()
            }
            (SymBoolExprOp::Uge, _, Some(value)) if value == U256::from(1) => {
                left.normalize_ne_zero_for_solver()
            }
            (SymBoolExprOp::Ule, _, Some(value)) if value.is_zero() => {
                left.normalize_eq_zero_for_solver()
            }
            (SymBoolExprOp::Ult, _, Some(value)) if value == U256::from(1) => {
                left.normalize_eq_zero_for_solver()
            }
            (SymBoolExprOp::Ult, Some(value), _) if value.is_zero() => {
                right.normalize_ne_zero_for_solver()
            }
            (SymBoolExprOp::Ule, Some(value), _) if value == U256::from(1) => {
                right.normalize_ne_zero_for_solver()
            }
            (SymBoolExprOp::Uge, Some(value), _) if value.is_zero() => {
                right.normalize_eq_zero_for_solver()
            }
            (SymBoolExprOp::Ugt, Some(value), _) if value == U256::from(1) => {
                right.normalize_eq_zero_for_solver()
            }
            _ => None,
        }
    }
}

impl SymExpr {
    fn normalized_bool_word_condition(&self) -> Option<SymBoolExpr> {
        strip_low_byte_mask(self)?.bool_word_condition().map(normalize_bool_for_solver)
    }

    fn add_overflow_check(&self, right: &Self) -> bool {
        let Some((base, increment)) = right.add_with_operand(self) else { return false };
        base == self && base.add_cannot_overflow_256(increment)
    }

    fn add_with_operand<'a>(&'a self, operand: &Self) -> Option<(&'a Self, &'a Self)> {
        let SymExprKind::Op(SymExprOp::Add, left, right) = self.kind() else { return None };
        if left == operand {
            Some((left, right))
        } else if right == operand {
            Some((right, left))
        } else {
            None
        }
    }

    fn normalize_eq_zero_for_solver(&self) -> Option<SymBoolExpr> {
        if let Some((numerator, denominator)) = self.udiv_operands() {
            return Some(Self::udiv_zero_condition(numerator, denominator));
        }
        if let SymExprKind::Ite(condition, then_expr, else_expr) = self.kind() {
            let then_zero = then_expr.normalize_eq_zero_for_solver().unwrap_or_else(|| {
                SymBoolExpr::eq(normalize_expr_for_solver(then_expr.clone()), Self::zero())
            });
            let else_zero = else_expr.normalize_eq_zero_for_solver().unwrap_or_else(|| {
                SymBoolExpr::eq(normalize_expr_for_solver(else_expr.clone()), Self::zero())
            });
            if then_zero.contains_udiv() || else_zero.contains_udiv() {
                return None;
            }
            let condition = normalize_bool_for_solver(condition.clone());
            return Some(SymBoolExpr::or(vec![
                SymBoolExpr::and(vec![condition.clone(), then_zero]),
                SymBoolExpr::and(vec![condition.not(), else_zero]),
            ]));
        }
        None
    }

    fn normalize_ne_zero_for_solver(&self) -> Option<SymBoolExpr> {
        if let Some((numerator, denominator)) = self.udiv_operands() {
            return Some(Self::udiv_nonzero_condition(numerator, denominator));
        }
        if let SymExprKind::Ite(condition, then_expr, else_expr) = self.kind() {
            let then_nonzero = then_expr.normalize_ne_zero_for_solver().unwrap_or_else(|| {
                SymBoolExpr::eq(normalize_expr_for_solver(then_expr.clone()), Self::zero()).not()
            });
            let else_nonzero = else_expr.normalize_ne_zero_for_solver().unwrap_or_else(|| {
                SymBoolExpr::eq(normalize_expr_for_solver(else_expr.clone()), Self::zero()).not()
            });
            if then_nonzero.contains_udiv() || else_nonzero.contains_udiv() {
                return None;
            }
            let condition = normalize_bool_for_solver(condition.clone());
            return Some(SymBoolExpr::or(vec![
                SymBoolExpr::and(vec![condition.clone(), then_nonzero]),
                SymBoolExpr::and(vec![condition.not(), else_nonzero]),
            ]));
        }
        None
    }

    fn udiv_operands(&self) -> Option<(&Self, &Self)> {
        match self.kind() {
            SymExprKind::Op(SymExprOp::UDiv, numerator, denominator) => {
                Some((numerator, denominator))
            }
            _ => None,
        }
    }

    fn udiv_zero_condition(numerator: &Self, denominator: &Self) -> SymBoolExpr {
        let numerator = normalize_expr_for_solver(numerator.clone());
        let denominator = normalize_expr_for_solver(denominator.clone());
        SymBoolExpr::or(vec![
            SymBoolExpr::eq(denominator.clone(), Self::zero()),
            SymBoolExpr::cmp(SymBoolExprOp::Ult, numerator, denominator),
        ])
    }

    fn udiv_nonzero_condition(numerator: &Self, denominator: &Self) -> SymBoolExpr {
        let numerator = normalize_expr_for_solver(numerator.clone());
        let denominator = normalize_expr_for_solver(denominator.clone());
        SymBoolExpr::and(vec![
            SymBoolExpr::eq(denominator.clone(), Self::zero()).not(),
            SymBoolExpr::cmp(SymBoolExprOp::Uge, numerator, denominator),
        ])
    }
}

impl ConstraintContext {
    fn word_bool_always_true(&self, expr: &SymExpr) -> bool {
        let mut terms = Vec::new();
        collect_or_terms(expr, &mut terms);
        if terms.len() <= 1 {
            return false;
        }

        let bool_terms = terms
            .iter()
            .filter_map(|term| term.normalized_bool_word_condition())
            .collect::<Vec<_>>();
        if bool_terms.iter().any(|term| {
            let negated = term.clone().not();
            bool_terms.contains(&negated)
        }) {
            return true;
        }
        for zero_term in &bool_terms {
            let Some(zero_operand) = zero_term.zero_check_operand() else { continue };
            if bool_terms.iter().any(|term| self.checked_mul_guard_for_operand(term, zero_operand))
            {
                return true;
            }
        }
        false
    }

    fn checked_mul_guard_for_operand(&self, expr: &SymBoolExpr, zero_operand: &SymExpr) -> bool {
        let SymBoolExprKind::Eq(left, right) = expr.kind() else { return false };
        self.checked_mul_guard_side(left, right, zero_operand)
            || self.checked_mul_guard_side(right, left, zero_operand)
    }

    fn checked_mul_guard_side(
        &self,
        div_expr: &SymExpr,
        expected: &SymExpr,
        zero_operand: &SymExpr,
    ) -> bool {
        let SymExprKind::Ite(condition, then_expr, else_expr) = div_expr.kind() else {
            return false;
        };
        if condition.zero_check_operand().is_none_or(|operand| operand != zero_operand) {
            return false;
        }
        if !then_expr.as_const().is_some_and(|value| value.is_zero()) {
            return false;
        }
        let Some((numerator, denominator)) = else_expr.udiv_operands() else { return false };
        if denominator != zero_operand {
            return false;
        }
        let SymExprKind::Op(SymExprOp::Mul, left, right) = numerator.kind() else {
            return false;
        };
        let other = if left == zero_operand {
            right
        } else if right == zero_operand {
            left
        } else {
            return false;
        };
        other == expected && self.mul_cannot_overflow_256(zero_operand, other)
    }

    fn mul_cannot_overflow_256(&self, left: &SymExpr, right: &SymExpr) -> bool {
        self.unsigned_bits(left).saturating_add(self.unsigned_bits(right)) <= 256
    }

    fn unsigned_bits(&self, expr: &SymExpr) -> usize {
        let bits = match expr.kind() {
            SymExprKind::Const(_)
            | SymExprKind::Var(_)
            | SymExprKind::GasLeft(_)
            | SymExprKind::Keccak { .. }
            | SymExprKind::Hash { .. }
            | SymExprKind::Not(_) => expr.unsigned_bits(),
            SymExprKind::Op(SymExprOp::And, left, right) => {
                if let Some(mask) = right.as_const() {
                    self.unsigned_bits(left).min(mask.bit_len())
                } else if let Some(mask) = left.as_const() {
                    self.unsigned_bits(right).min(mask.bit_len())
                } else {
                    256
                }
            }
            SymExprKind::Op(SymExprOp::Add, left, right) => {
                self.unsigned_bits(left).max(self.unsigned_bits(right)).saturating_add(1).min(256)
            }
            SymExprKind::Op(SymExprOp::Mul, left, right) => {
                self.unsigned_bits(left).saturating_add(self.unsigned_bits(right)).min(256)
            }
            SymExprKind::Op(SymExprOp::UDiv, left, _) => self.unsigned_bits(left),
            SymExprKind::Ite(_, left, right) => {
                self.unsigned_bits(left).max(self.unsigned_bits(right))
            }
            _ => 256,
        };

        self.upper_bound(expr).map(|bound| bits.min(bound.bit_len().max(1))).unwrap_or(bits)
    }
}
