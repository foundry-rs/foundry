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
    match expr.as_inner() {
        SymBoolExprInner::Const(true) => {}
        SymBoolExprInner::And(values) => {
            for value in values.iter().cloned() {
                collect_normalized_conjunct(value, out);
            }
        }
        _ => out.push(expr),
    }
}

/// Normalizes one boolean expression into an equivalent, solver-friendlier form.
pub(crate) fn normalize_bool_for_solver(expr: SymBoolExpr) -> SymBoolExpr {
    if let Some(normalized) = normalize_udiv_bool_for_solver(&expr) {
        return normalized;
    }

    match expr.into_inner() {
        SymBoolExprInner::Const(value) => SymBoolExpr::constant(value),
        SymBoolExprInner::Not(value) => normalize_bool_for_solver(value).not(),
        SymBoolExprInner::And(values) => {
            SymBoolExpr::and(values.iter().cloned().map(normalize_bool_for_solver).collect())
        }
        SymBoolExprInner::Eq(left, right) => {
            let normalized =
                SymBoolExpr::eq(normalize_expr_for_solver(left), normalize_expr_for_solver(right));
            normalize_udiv_bool_for_solver(&normalized).unwrap_or(normalized)
        }
        SymBoolExprInner::Cmp(op, left, right) => {
            let normalized = SymBoolExpr::cmp(
                op,
                normalize_expr_for_solver(left),
                normalize_expr_for_solver(right),
            );
            normalize_udiv_bool_for_solver(&normalized).unwrap_or(normalized)
        }
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
        match expr.as_inner() {
            _ if zero_check_operand(&expr).is_some_and(|left| self.word_bool_always_true(left)) => {
                SymBoolExpr::constant(false)
            }
            SymBoolExprInner::Not(value)
                if zero_check_operand(value)
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
        match constraint.as_inner() {
            SymBoolExprInner::Eq(left, right) => match (left.as_const(), right.as_const()) {
                (_, Some(value)) => Some((left, value)),
                (Some(value), _) => Some((right, value)),
                _ => None,
            },
            SymBoolExprInner::Cmp(op, left, right) => {
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
            SymBoolExprInner::Not(value) => match value.as_inner() {
                SymBoolExprInner::Cmp(op, left, right) => {
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
            SymBoolExprInner::Const(_) | SymBoolExprInner::And(_) => None,
        }
    }
}

/// Normalizes one word expression into an equivalent, solver-friendlier form.
pub(crate) fn normalize_expr_for_solver(expr: SymExpr) -> SymExpr {
    if let Some(rebuilt) = rebuild_word_from_extracted_byte_terms(&expr)
        && rebuilt != expr
    {
        return normalize_expr_for_solver(rebuilt);
    }

    if matches!(
        expr.as_inner(),
        SymExprInner::Const(_)
            | SymExprInner::Var(_)
            | SymExprInner::GasLeft(_)
            | SymExprInner::Keccak { .. }
            | SymExprInner::Hash { .. }
    ) {
        return expr;
    }

    match expr.into_inner() {
        SymExprInner::Not(value) => SymExpr::not(normalize_expr_for_solver(value)),
        SymExprInner::Op(op, left, right) => {
            let left = normalize_expr_for_solver(left);
            let right = normalize_expr_for_solver(right);
            if matches!(
                op,
                SymExprOp::Add | SymExprOp::Mul | SymExprOp::And | SymExprOp::Or | SymExprOp::Xor
            ) && right < left
            {
                SymExpr::op(op, right, left)
            } else {
                SymExpr::op(op, left, right)
            }
        }
        SymExprInner::AddMod { left, right, modulus } => SymExpr::addmod(
            normalize_expr_for_solver(left),
            normalize_expr_for_solver(right),
            normalize_expr_for_solver(modulus),
        ),
        SymExprInner::MulMod { left, right, modulus } => SymExpr::mulmod(
            normalize_expr_for_solver(left),
            normalize_expr_for_solver(right),
            normalize_expr_for_solver(modulus),
        ),
        SymExprInner::Ite(cond, left, right) => normalize_ite_expr_for_solver(cond, left, right),
        SymExprInner::Const(_)
        | SymExprInner::Var(_)
        | SymExprInner::GasLeft(_)
        | SymExprInner::Keccak { .. }
        | SymExprInner::Hash { .. } => unreachable!(),
    }
}

/// Normalizes a word-valued conditional expression.
pub(crate) fn normalize_ite_expr_for_solver(
    cond: SymBoolExpr,
    left: SymExpr,
    right: SymExpr,
) -> SymExpr {
    let cond = normalize_bool_for_solver(cond);
    let left = normalize_expr_for_solver(left);
    let right = normalize_expr_for_solver(right);
    if left == right {
        return left;
    }
    if let Some(condition) = guarded_self_div_word_condition(&cond, &left, &right) {
        return word_from_bool_expr(condition);
    }
    if left.as_const() == Some(U256::from(1)) && bool_from_word_expr(&right).as_ref() == Some(&cond)
    {
        return right;
    }
    if right.as_const().is_some_and(|value| value.is_zero())
        && bool_from_word_expr(&left).as_ref() == Some(&cond)
    {
        return left;
    }
    SymExpr::ite(cond, left, right)
}

/// Converts a boolean condition into its 0/1 word representation.
fn word_from_bool_expr(condition: SymBoolExpr) -> SymExpr {
    SymExpr::ite(condition, SymExpr::constant(U256::from(1)), SymExpr::constant(U256::ZERO))
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
    let Some(zero_operand) = zero_check_operand(cond) else { return false };
    let Some((numerator, denominator)) = udiv_operands(expr) else { return false };
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
        if !seen && expr_known_byte(&source, index) != Some(0) {
            return None;
        }
    }
    Some(source)
}

/// Flattens nested bitwise-OR expressions into their leaf terms.
pub(crate) fn collect_or_terms<'a>(expr: &'a SymExpr, terms: &mut Vec<&'a SymExpr>) {
    match expr.as_inner() {
        SymExprInner::Op(SymExprOp::Or, left, right) => {
            collect_or_terms(left, terms);
            collect_or_terms(right, terms);
        }
        _ => terms.push(expr),
    }
}

/// Returns the source word and byte index for one shifted extracted-byte term.
pub(crate) fn extracted_shifted_byte_term(term: &SymExpr) -> Option<(SymExpr, usize)> {
    match term.as_inner() {
        SymExprInner::Op(SymExprOp::Shl, byte, shift) => {
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
    let SymExprInner::Op(SymExprOp::Shr, source, shift) = expr.as_inner() else { return None };
    let shift = shift.as_const()?;
    (shift == U256::from((31 - index) * 8)).then(|| source.clone())
}

/// Rewrites exact EVM unsigned-division zero/nonzero predicates without `bvudiv`.
pub(crate) fn normalize_udiv_bool_for_solver(expr: &SymBoolExpr) -> Option<SymBoolExpr> {
    match expr.as_inner() {
        SymBoolExprInner::Eq(left, right)
            if right.as_const().is_some_and(|value| value.is_zero()) =>
        {
            bool_from_word_expr(left).map(SymBoolExpr::not).or_else(|| {
                if word_bool_always_true(left) {
                    Some(SymBoolExpr::constant(false))
                } else {
                    normalize_udiv_eq_zero(left, &SymExpr::constant(U256::ZERO))
                }
            })
        }
        SymBoolExprInner::Eq(left, right)
            if left.as_const().is_some_and(|value| value.is_zero()) =>
        {
            bool_from_word_expr(right).map(SymBoolExpr::not).or_else(|| {
                if word_bool_always_true(right) {
                    Some(SymBoolExpr::constant(false))
                } else {
                    normalize_udiv_eq_zero(&SymExpr::constant(U256::ZERO), right)
                }
            })
        }
        SymBoolExprInner::Eq(left, right) if right.as_const() == Some(U256::from(1)) => {
            bool_from_word_expr(left)
        }
        SymBoolExprInner::Eq(left, right) if left.as_const() == Some(U256::from(1)) => {
            bool_from_word_expr(right)
        }
        SymBoolExprInner::Not(value) => match value.as_inner() {
            SymBoolExprInner::Cmp(op, left, right) => {
                normalize_add_overflow_cmp_for_solver(*op, left, right)
                    .map(SymBoolExpr::not)
                    .or_else(|| {
                        normalize_udiv_cmp_for_solver(*op, left, right).map(SymBoolExpr::not)
                    })
            }
            SymBoolExprInner::Eq(left, right)
                if right.as_const().is_some_and(|value| value.is_zero()) =>
            {
                if word_bool_always_true(left) {
                    Some(SymBoolExpr::constant(true))
                } else {
                    normalize_udiv_eq_zero(left, &SymExpr::constant(U256::ZERO))
                        .map(SymBoolExpr::not)
                }
            }
            SymBoolExprInner::Eq(left, right)
                if left.as_const().is_some_and(|value| value.is_zero()) =>
            {
                if word_bool_always_true(right) {
                    Some(SymBoolExpr::constant(true))
                } else {
                    normalize_udiv_eq_zero(&SymExpr::constant(U256::ZERO), right)
                        .map(SymBoolExpr::not)
                }
            }
            SymBoolExprInner::Eq(left, right) => {
                normalize_udiv_eq_zero(left, right).map(SymBoolExpr::not)
            }
            _ => None,
        },
        SymBoolExprInner::Eq(left, right) => normalize_udiv_eq_zero(left, right),
        SymBoolExprInner::Cmp(op, left, right) => {
            normalize_add_overflow_cmp_for_solver(*op, left, right)
                .or_else(|| normalize_udiv_cmp_for_solver(*op, left, right))
        }
        SymBoolExprInner::Const(_) | SymBoolExprInner::And(_) => None,
    }
}

/// Extracts the boolean condition represented by a word-valued `0`/`1` expression.
pub(crate) fn bool_from_word_expr(expr: &SymExpr) -> Option<SymBoolExpr> {
    let expr = strip_low_byte_mask(expr)?;
    let SymExprInner::Ite(condition, then_expr, else_expr) = expr.as_inner() else { return None };
    match (then_expr.as_const(), else_expr.as_const()) {
        (Some(then_value), Some(else_value))
            if then_value == U256::from(1) && else_value.is_zero() =>
        {
            Some(normalize_bool_for_solver(condition.clone()))
        }
        (Some(then_value), Some(else_value))
            if then_value.is_zero() && else_value == U256::from(1) =>
        {
            Some(normalize_bool_for_solver(condition.clone()).not())
        }
        _ => None,
    }
}

/// Returns whether a word expression syntactically contains unsigned division.
pub(crate) fn expr_contains_udiv(expr: &SymExpr) -> bool {
    expr.visit(&mut |expr| {
        if matches!(expr.as_inner(), SymExprInner::Op(SymExprOp::UDiv, _, _)) {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    })
    .is_break()
}

/// Returns whether a boolean expression syntactically contains unsigned division.
pub(crate) fn bool_contains_udiv(expr: &SymBoolExpr) -> bool {
    expr.visit(&mut |expr| match expr.as_inner() {
        SymBoolExprInner::Eq(left, right) | SymBoolExprInner::Cmp(_, left, right)
            if expr_contains_udiv(left) || expr_contains_udiv(right) =>
        {
            ControlFlow::Break(())
        }
        _ => ControlFlow::Continue(()),
    })
    .is_break()
}

/// Rewrites exact unsigned-addition overflow checks when operand bounds preclude overflow.
pub(crate) fn normalize_add_overflow_cmp_for_solver(
    op: SymBoolExprOp,
    left: &SymExpr,
    right: &SymExpr,
) -> Option<SymBoolExpr> {
    match op {
        SymBoolExprOp::Ugt if add_overflow_check(left, right) => Some(SymBoolExpr::constant(false)),
        SymBoolExprOp::Ult if add_overflow_check(right, left) => Some(SymBoolExpr::constant(false)),
        _ => None,
    }
}

/// Returns whether `left > left + increment` is an impossible overflow check.
pub(crate) fn add_overflow_check(left: &SymExpr, right: &SymExpr) -> bool {
    let Some((base, increment)) = add_with_operand(right, left) else { return false };
    base == left && add_cannot_overflow_256(base, increment)
}

/// Returns the operands if `expr` is an addition involving `operand`.
pub(crate) fn add_with_operand<'a>(
    expr: &'a SymExpr,
    operand: &SymExpr,
) -> Option<(&'a SymExpr, &'a SymExpr)> {
    let SymExprInner::Op(SymExprOp::Add, left, right) = expr.as_inner() else { return None };
    if left == operand {
        Some((left, right))
    } else if right == operand {
        Some((right, left))
    } else {
        None
    }
}

/// Returns whether unsigned addition of two expressions cannot overflow 256 bits.
pub(crate) fn add_cannot_overflow_256(left: &SymExpr, right: &SymExpr) -> bool {
    expr_unsigned_bits(left).max(expr_unsigned_bits(right)).saturating_add(1) <= 256
}

/// Returns whether a word-valued boolean expression is an exact tautology.
pub(crate) fn word_bool_always_true(expr: &SymExpr) -> bool {
    ConstraintContext::default().word_bool_always_true(expr)
}

/// Converts one `0`/`1` word boolean term into its boolean condition.
pub(crate) fn word_bool_term(expr: &SymExpr) -> Option<&SymBoolExpr> {
    let SymExprInner::Ite(condition, then_expr, else_expr) = expr.as_inner() else { return None };
    match (then_expr.as_const(), else_expr.as_const()) {
        (Some(then_value), Some(else_value))
            if then_value == U256::from(1) && else_value.is_zero() =>
        {
            Some(condition)
        }
        _ => None,
    }
}

/// Returns the operand tested by `operand == 0`.
pub(crate) fn zero_check_operand(expr: &SymBoolExpr) -> Option<&SymExpr> {
    match expr.as_inner() {
        SymBoolExprInner::Eq(left, right)
            if right.as_const().is_some_and(|value| value.is_zero()) =>
        {
            Some(left)
        }
        SymBoolExprInner::Eq(left, right)
            if left.as_const().is_some_and(|value| value.is_zero()) =>
        {
            Some(right)
        }
        _ => None,
    }
}

impl ConstraintContext {
    fn word_bool_always_true(&self, expr: &SymExpr) -> bool {
        let mut terms = Vec::new();
        collect_or_terms(expr, &mut terms);
        if terms.len() <= 1 {
            return false;
        }

        let bool_terms = terms.iter().filter_map(|term| word_bool_term(term)).collect::<Vec<_>>();
        if bool_terms.iter().any(|term| {
            let negated = (*term).clone().not();
            bool_terms.iter().any(|other| **other == negated)
        }) {
            return true;
        }
        for zero_term in &bool_terms {
            let Some(zero_operand) = zero_check_operand(zero_term) else { continue };
            if bool_terms.iter().any(|term| self.checked_mul_guard_for_operand(term, zero_operand))
            {
                return true;
            }
        }
        false
    }

    fn checked_mul_guard_for_operand(&self, expr: &SymBoolExpr, zero_operand: &SymExpr) -> bool {
        let SymBoolExprInner::Eq(left, right) = expr.as_inner() else { return false };
        self.checked_mul_guard_side(left, right, zero_operand)
            || self.checked_mul_guard_side(right, left, zero_operand)
    }

    fn checked_mul_guard_side(
        &self,
        div_expr: &SymExpr,
        expected: &SymExpr,
        zero_operand: &SymExpr,
    ) -> bool {
        let SymExprInner::Ite(condition, then_expr, else_expr) = div_expr.as_inner() else {
            return false;
        };
        if zero_check_operand(condition).is_none_or(|operand| operand != zero_operand) {
            return false;
        }
        if !then_expr.as_const().is_some_and(|value| value.is_zero()) {
            return false;
        }
        let Some((numerator, denominator)) = udiv_operands(else_expr) else { return false };
        if denominator != zero_operand {
            return false;
        }
        let SymExprInner::Op(SymExprOp::Mul, left, right) = numerator.as_inner() else {
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
        self.expr_unsigned_bits(left).saturating_add(self.expr_unsigned_bits(right)) <= 256
    }

    fn expr_unsigned_bits(&self, expr: &SymExpr) -> usize {
        let bits = match expr.as_inner() {
            SymExprInner::Const(_)
            | SymExprInner::Var(_)
            | SymExprInner::GasLeft(_)
            | SymExprInner::Keccak { .. }
            | SymExprInner::Hash { .. }
            | SymExprInner::Not(_) => expr_unsigned_bits(expr),
            SymExprInner::Op(SymExprOp::And, left, right) => {
                if let Some(mask) = right.as_const() {
                    self.expr_unsigned_bits(left).min(mask.bit_len())
                } else if let Some(mask) = left.as_const() {
                    self.expr_unsigned_bits(right).min(mask.bit_len())
                } else {
                    256
                }
            }
            SymExprInner::Op(SymExprOp::Add, left, right) => self
                .expr_unsigned_bits(left)
                .max(self.expr_unsigned_bits(right))
                .saturating_add(1)
                .min(256),
            SymExprInner::Op(SymExprOp::Mul, left, right) => self
                .expr_unsigned_bits(left)
                .saturating_add(self.expr_unsigned_bits(right))
                .min(256),
            SymExprInner::Op(SymExprOp::UDiv, left, _) => self.expr_unsigned_bits(left),
            SymExprInner::Ite(_, left, right) => {
                self.expr_unsigned_bits(left).max(self.expr_unsigned_bits(right))
            }
            _ => 256,
        };

        self.upper_bound(expr).map(|bound| bits.min(bound.bit_len().max(1))).unwrap_or(bits)
    }
}

/// Returns whether unsigned multiplication of two expressions cannot overflow 256 bits.
pub(crate) fn mul_cannot_overflow_256(left: &SymExpr, right: &SymExpr) -> bool {
    expr_unsigned_bits(left).saturating_add(expr_unsigned_bits(right)) <= 256
}

/// Returns a conservative unsigned bit-width upper bound for an expression.
pub(crate) fn expr_unsigned_bits(expr: &SymExpr) -> usize {
    match expr.as_inner() {
        SymExprInner::Const(value) => value.bit_len().max(1),
        SymExprInner::Op(SymExprOp::And, left, right) => {
            if let Some(mask) = right.as_const() {
                expr_unsigned_bits(left).min(mask.bit_len())
            } else if let Some(mask) = left.as_const() {
                expr_unsigned_bits(right).min(mask.bit_len())
            } else {
                256
            }
        }
        SymExprInner::Op(SymExprOp::Add, left, right) => {
            expr_unsigned_bits(left).max(expr_unsigned_bits(right)).saturating_add(1).min(256)
        }
        SymExprInner::Op(SymExprOp::Mul, left, right) => {
            expr_unsigned_bits(left).saturating_add(expr_unsigned_bits(right)).min(256)
        }
        SymExprInner::Op(SymExprOp::UDiv, left, _) => expr_unsigned_bits(left),
        SymExprInner::AddMod { modulus, .. } | SymExprInner::MulMod { modulus, .. } => {
            expr_unsigned_bits(modulus)
        }
        SymExprInner::Ite(_, left, right) => {
            expr_unsigned_bits(left).max(expr_unsigned_bits(right))
        }
        _ => 256,
    }
}

/// Rewrites `udiv(a, b) == 0` predicates using EVM division-by-zero semantics.
pub(crate) fn normalize_udiv_eq_zero(left: &SymExpr, right: &SymExpr) -> Option<SymBoolExpr> {
    if right.as_const().is_some_and(|value| value.is_zero())
        && let Some(condition) = normalize_expr_eq_zero_for_solver(left)
    {
        return Some(condition);
    }
    if left.as_const().is_some_and(|value| value.is_zero())
        && let Some(condition) = normalize_expr_eq_zero_for_solver(right)
    {
        return Some(condition);
    }
    None
}

/// Rewrites `expr == 0` when `expr` contains exactly-normalizable unsigned division.
pub(crate) fn normalize_expr_eq_zero_for_solver(expr: &SymExpr) -> Option<SymBoolExpr> {
    if let Some((numerator, denominator)) = udiv_operands(expr) {
        return Some(udiv_zero_condition(numerator, denominator));
    }
    if let SymExprInner::Ite(condition, then_expr, else_expr) = expr.as_inner() {
        let then_zero = normalize_expr_eq_zero_for_solver(then_expr).unwrap_or_else(|| {
            SymBoolExpr::eq(
                normalize_expr_for_solver(then_expr.clone()),
                SymExpr::constant(U256::ZERO),
            )
        });
        let else_zero = normalize_expr_eq_zero_for_solver(else_expr).unwrap_or_else(|| {
            SymBoolExpr::eq(
                normalize_expr_for_solver(else_expr.clone()),
                SymExpr::constant(U256::ZERO),
            )
        });
        if bool_contains_udiv(&then_zero) || bool_contains_udiv(&else_zero) {
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

/// Rewrites `expr != 0` when `expr` contains exactly-normalizable unsigned division.
pub(crate) fn normalize_expr_ne_zero_for_solver(expr: &SymExpr) -> Option<SymBoolExpr> {
    if let Some((numerator, denominator)) = udiv_operands(expr) {
        return Some(udiv_nonzero_condition(numerator, denominator));
    }
    if let SymExprInner::Ite(condition, then_expr, else_expr) = expr.as_inner() {
        let then_nonzero = normalize_expr_ne_zero_for_solver(then_expr).unwrap_or_else(|| {
            SymBoolExpr::eq(
                normalize_expr_for_solver(then_expr.clone()),
                SymExpr::constant(U256::ZERO),
            )
            .not()
        });
        let else_nonzero = normalize_expr_ne_zero_for_solver(else_expr).unwrap_or_else(|| {
            SymBoolExpr::eq(
                normalize_expr_for_solver(else_expr.clone()),
                SymExpr::constant(U256::ZERO),
            )
            .not()
        });
        if bool_contains_udiv(&then_nonzero) || bool_contains_udiv(&else_nonzero) {
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

/// Rewrites `udiv(a, b)` zero/nonzero comparisons using EVM division-by-zero semantics.
pub(crate) fn normalize_udiv_cmp_for_solver(
    op: SymBoolExprOp,
    left: &SymExpr,
    right: &SymExpr,
) -> Option<SymBoolExpr> {
    match (op, left.as_const(), right.as_const()) {
        (SymBoolExprOp::Ugt, _, Some(value)) if value.is_zero() => {
            normalize_expr_ne_zero_for_solver(left)
        }
        (SymBoolExprOp::Uge, _, Some(value)) if value == U256::from(1) => {
            normalize_expr_ne_zero_for_solver(left)
        }
        (SymBoolExprOp::Ule, _, Some(value)) if value.is_zero() => {
            normalize_expr_eq_zero_for_solver(left)
        }
        (SymBoolExprOp::Ult, _, Some(value)) if value == U256::from(1) => {
            normalize_expr_eq_zero_for_solver(left)
        }
        (SymBoolExprOp::Ult, Some(value), _) if value.is_zero() => {
            normalize_expr_ne_zero_for_solver(right)
        }
        (SymBoolExprOp::Ule, Some(value), _) if value == U256::from(1) => {
            normalize_expr_ne_zero_for_solver(right)
        }
        (SymBoolExprOp::Uge, Some(value), _) if value.is_zero() => {
            normalize_expr_eq_zero_for_solver(right)
        }
        (SymBoolExprOp::Ugt, Some(value), _) if value == U256::from(1) => {
            normalize_expr_eq_zero_for_solver(right)
        }
        _ => None,
    }
}

/// Returns the operands for an unsigned division expression.
pub(crate) fn udiv_operands(expr: &SymExpr) -> Option<(&SymExpr, &SymExpr)> {
    match expr.as_inner() {
        SymExprInner::Op(SymExprOp::UDiv, numerator, denominator) => Some((numerator, denominator)),
        _ => None,
    }
}

/// Builds the exact condition for EVM `udiv(numerator, denominator) == 0`.
pub(crate) fn udiv_zero_condition(numerator: &SymExpr, denominator: &SymExpr) -> SymBoolExpr {
    let numerator = normalize_expr_for_solver(numerator.clone());
    let denominator = normalize_expr_for_solver(denominator.clone());
    SymBoolExpr::or(vec![
        SymBoolExpr::eq(denominator.clone(), SymExpr::constant(U256::ZERO)),
        SymBoolExpr::cmp(SymBoolExprOp::Ult, numerator, denominator),
    ])
}

/// Builds the exact condition for EVM `udiv(numerator, denominator) != 0`.
pub(crate) fn udiv_nonzero_condition(numerator: &SymExpr, denominator: &SymExpr) -> SymBoolExpr {
    let numerator = normalize_expr_for_solver(numerator.clone());
    let denominator = normalize_expr_for_solver(denominator.clone());
    SymBoolExpr::and(vec![
        SymBoolExpr::eq(denominator.clone(), SymExpr::constant(U256::ZERO)).not(),
        SymBoolExpr::cmp(SymBoolExprOp::Uge, numerator, denominator),
    ])
}
