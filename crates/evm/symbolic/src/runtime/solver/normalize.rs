use super::*;

/// Normalizes path constraints into an equivalent, solver-friendlier form.
pub(crate) fn normalize_constraints_for_solver(constraints: &[BoolExpr]) -> Vec<BoolExpr> {
    let normalized = normalize_constraint_batch(
        constraints.iter().cloned().map(normalize_bool_for_solver),
        constraints.len(),
    );
    if matches!(normalized.as_slice(), [BoolExpr::Const(false)]) {
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
    constraints: impl IntoIterator<Item = BoolExpr>,
    capacity: usize,
) -> Vec<BoolExpr> {
    let mut normalized = Vec::with_capacity(capacity);
    for constraint in constraints {
        if matches!(constraint, BoolExpr::Const(false)) {
            return vec![BoolExpr::Const(false)];
        }
        collect_normalized_conjunct(constraint, &mut normalized);
    }
    normalized.sort();
    normalized.dedup();
    normalized
}

fn collect_normalized_conjunct(expr: BoolExpr, out: &mut Vec<BoolExpr>) {
    match expr {
        BoolExpr::Const(true) => {}
        BoolExpr::And(values) => {
            for value in values {
                collect_normalized_conjunct(value, out);
            }
        }
        value => out.push(value),
    }
}

/// Normalizes one boolean expression into an equivalent, solver-friendlier form.
pub(crate) fn normalize_bool_for_solver(expr: BoolExpr) -> BoolExpr {
    if let Some(normalized) = normalize_udiv_bool_for_solver(&expr) {
        return normalized;
    }

    match expr {
        BoolExpr::Const(value) => BoolExpr::Const(value),
        BoolExpr::Not(value) => normalize_bool_for_solver(*value).not(),
        BoolExpr::And(values) => {
            BoolExpr::and(values.into_iter().map(normalize_bool_for_solver).collect())
        }
        BoolExpr::Eq(left, right) => {
            let normalized =
                BoolExpr::Eq(normalize_expr_for_solver(left), normalize_expr_for_solver(right));
            normalize_udiv_bool_for_solver(&normalized).unwrap_or_else(|| match normalized {
                BoolExpr::Eq(left, right) => BoolExpr::eq(left, right),
                _ => unreachable!(),
            })
        }
        BoolExpr::Cmp(op, left, right) => {
            let normalized = BoolExpr::Cmp(
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
    upper_bounds: BTreeMap<Expr, U256>,
}

impl ConstraintContext {
    fn new(constraints: &[BoolExpr]) -> Self {
        let mut context = Self::default();
        for constraint in constraints {
            context.record_upper_bound_constraint(constraint);
        }
        context
    }

    fn upper_bound(&self, expr: &Expr) -> Option<U256> {
        self.upper_bounds.get(expr).copied()
    }

    fn normalize_bool(&self, expr: BoolExpr) -> BoolExpr {
        match &expr {
            BoolExpr::Eq(left, Expr::Const(value)) | BoolExpr::Eq(Expr::Const(value), left)
                if value.is_zero() && self.word_bool_always_true(left) =>
            {
                BoolExpr::Const(false)
            }
            BoolExpr::Not(value) => match value.as_ref() {
                BoolExpr::Eq(left, Expr::Const(value)) | BoolExpr::Eq(Expr::Const(value), left)
                    if value.is_zero() && self.word_bool_always_true(left) =>
                {
                    BoolExpr::Const(true)
                }
                _ => expr,
            },
            _ => expr,
        }
    }

    fn record_upper_bound_constraint(&mut self, constraint: &BoolExpr) {
        if let Some((expr, bound)) = self.upper_bound_constraint(constraint) {
            self.record_upper_bound(expr.clone(), bound);
        }
    }

    fn record_upper_bound(&mut self, expr: Expr, bound: U256) {
        self.upper_bounds
            .entry(expr)
            .and_modify(|existing| *existing = (*existing).min(bound))
            .or_insert(bound);
    }

    fn upper_bound_constraint<'a>(&self, constraint: &'a BoolExpr) -> Option<(&'a Expr, U256)> {
        match constraint {
            BoolExpr::Eq(left, Expr::Const(value)) | BoolExpr::Eq(Expr::Const(value), left) => {
                Some((left, *value))
            }
            BoolExpr::Cmp(op, left, right) => match (*op, left, right) {
                (BoolExprOp::Ult, expr, Expr::Const(bound)) => {
                    (!bound.is_zero()).then(|| (expr, *bound - U256::from(1)))
                }
                (BoolExprOp::Ule, expr, Expr::Const(bound)) => Some((expr, *bound)),
                (BoolExprOp::Ugt, Expr::Const(bound), expr) => {
                    (!bound.is_zero()).then(|| (expr, *bound - U256::from(1)))
                }
                (BoolExprOp::Uge, Expr::Const(bound), expr) => Some((expr, *bound)),
                _ => None,
            },
            BoolExpr::Not(value) => match value.as_ref() {
                BoolExpr::Cmp(BoolExprOp::Ugt, left, Expr::Const(bound)) => Some((left, *bound)),
                BoolExpr::Cmp(BoolExprOp::Uge, left, Expr::Const(bound)) => {
                    (!bound.is_zero()).then(|| (left, *bound - U256::from(1)))
                }
                BoolExpr::Cmp(BoolExprOp::Ult, Expr::Const(bound), right) => Some((right, *bound)),
                BoolExpr::Cmp(BoolExprOp::Ule, Expr::Const(bound), right) => {
                    (!bound.is_zero()).then(|| (right, *bound - U256::from(1)))
                }
                _ => None,
            },
            BoolExpr::Eq(_, _) | BoolExpr::Const(_) | BoolExpr::And(_) => None,
        }
    }
}

/// Normalizes one word expression into an equivalent, solver-friendlier form.
pub(crate) fn normalize_expr_for_solver(expr: Expr) -> Expr {
    if let Some(rebuilt) = rebuild_word_from_extracted_byte_terms(&expr)
        && rebuilt != expr
    {
        return normalize_expr_for_solver(rebuilt);
    }

    match expr {
        Expr::Const(_) | Expr::Var(_) | Expr::GasLeft(_) | Expr::Keccak(_) | Expr::Hash(_) => expr,
        Expr::Not(value) => Expr::Not(Box::new(normalize_expr_for_solver(*value))),
        Expr::Op(op, left, right) => {
            let left = normalize_expr_for_solver(*left);
            let right = normalize_expr_for_solver(*right);
            if matches!(op, ExprOp::Add | ExprOp::Mul | ExprOp::And | ExprOp::Or | ExprOp::Xor)
                && right < left
            {
                Expr::op(op, right, left)
            } else {
                Expr::op(op, left, right)
            }
        }
        Expr::AddMod { left, right, modulus } => Expr::addmod(
            normalize_expr_for_solver(*left),
            normalize_expr_for_solver(*right),
            normalize_expr_for_solver(*modulus),
        ),
        Expr::MulMod { left, right, modulus } => Expr::mulmod(
            normalize_expr_for_solver(*left),
            normalize_expr_for_solver(*right),
            normalize_expr_for_solver(*modulus),
        ),
        Expr::Ite(cond, left, right) => normalize_ite_expr_for_solver(*cond, *left, *right),
    }
}

/// Normalizes a word-valued conditional expression.
pub(crate) fn normalize_ite_expr_for_solver(cond: BoolExpr, left: Expr, right: Expr) -> Expr {
    let cond = normalize_bool_for_solver(cond);
    let left = normalize_expr_for_solver(left);
    let right = normalize_expr_for_solver(right);
    if left == right {
        return left;
    }
    if let Some(condition) = guarded_self_div_word_condition(&cond, &left, &right) {
        return word_from_bool_expr(condition);
    }
    if matches!(left, Expr::Const(value) if value == U256::from(1))
        && bool_from_word_expr(&right).as_ref() == Some(&cond)
    {
        return right;
    }
    if matches!(right, Expr::Const(value) if value.is_zero())
        && bool_from_word_expr(&left).as_ref() == Some(&cond)
    {
        return left;
    }
    Expr::Ite(Box::new(cond), Box::new(left), Box::new(right))
}

/// Converts a boolean condition into its 0/1 word representation.
fn word_from_bool_expr(condition: BoolExpr) -> Expr {
    Expr::Ite(
        Box::new(condition),
        Box::new(Expr::Const(U256::from(1))),
        Box::new(Expr::Const(U256::ZERO)),
    )
}

/// Returns the boolean represented by `a == 0 ? 0 : a / a`.
fn guarded_self_div_word_condition(cond: &BoolExpr, left: &Expr, right: &Expr) -> Option<BoolExpr> {
    if matches!(left, Expr::Const(value) if value.is_zero())
        && self_div_expr_matches_zero_check(cond, right)
    {
        return Some(cond.clone().not());
    }
    None
}

/// Returns whether `expr` is `a / a` for the operand guarded by `cond`.
fn self_div_expr_matches_zero_check(cond: &BoolExpr, expr: &Expr) -> bool {
    let Some(zero_operand) = zero_check_operand(cond) else { return false };
    let Some((numerator, denominator)) = udiv_operands(expr) else { return false };
    numerator == zero_operand && denominator == zero_operand
}

/// Rebuilds a word from OR-ed byte-extraction terms when the source is recoverable.
pub(crate) fn rebuild_word_from_extracted_byte_terms(expr: &Expr) -> Option<Expr> {
    let mut terms = Vec::new();
    collect_or_terms(expr, &mut terms);
    if terms.len() <= 1 {
        return None;
    }

    let mut source = None;
    let mut seen = [false; 32];
    for term in terms {
        if matches!(term, Expr::Const(value) if value.is_zero()) {
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
pub(crate) fn collect_or_terms<'a>(expr: &'a Expr, terms: &mut Vec<&'a Expr>) {
    match expr {
        Expr::Op(ExprOp::Or, left, right) => {
            collect_or_terms(left, terms);
            collect_or_terms(right, terms);
        }
        expr => terms.push(expr),
    }
}

/// Returns the source word and byte index for one shifted extracted-byte term.
pub(crate) fn extracted_shifted_byte_term(term: &Expr) -> Option<(Expr, usize)> {
    match term {
        Expr::Op(ExprOp::Shl, byte, shift) => {
            let Expr::Const(shift) = shift.as_ref() else { return None };
            let shift = shift.to::<usize>();
            if shift % 8 != 0 || shift > 248 {
                return None;
            }
            let index = 31 - shift / 8;
            let source = extracted_unshifted_byte_source(byte, index)?;
            Some((source, index))
        }
        term => extracted_unshifted_byte_source(term, 31).map(|source| (source, 31)),
    }
}

/// Returns the source word for an unshifted byte extraction at `index`.
pub(crate) fn extracted_unshifted_byte_source(term: &Expr, index: usize) -> Option<Expr> {
    let expr = strip_low_byte_mask(term)?;
    if index == 31 {
        return Some(expr.clone());
    }
    let Expr::Op(ExprOp::Shr, source, shift) = expr else { return None };
    let Expr::Const(shift) = shift.as_ref() else { return None };
    (*shift == U256::from((31 - index) * 8)).then(|| *source.clone())
}

/// Rewrites exact EVM unsigned-division zero/nonzero predicates without `bvudiv`.
pub(crate) fn normalize_udiv_bool_for_solver(expr: &BoolExpr) -> Option<BoolExpr> {
    match expr {
        BoolExpr::Eq(left, Expr::Const(value)) if value.is_zero() => {
            bool_from_word_expr(left).map(BoolExpr::not).or_else(|| {
                if word_bool_always_true(left) {
                    Some(BoolExpr::Const(false))
                } else {
                    normalize_udiv_eq_zero(left, &Expr::Const(U256::ZERO))
                }
            })
        }
        BoolExpr::Eq(Expr::Const(value), right) if value.is_zero() => {
            bool_from_word_expr(right).map(BoolExpr::not).or_else(|| {
                if word_bool_always_true(right) {
                    Some(BoolExpr::Const(false))
                } else {
                    normalize_udiv_eq_zero(&Expr::Const(U256::ZERO), right)
                }
            })
        }
        BoolExpr::Eq(left, Expr::Const(value)) if *value == U256::from(1) => {
            bool_from_word_expr(left)
        }
        BoolExpr::Eq(Expr::Const(value), right) if *value == U256::from(1) => {
            bool_from_word_expr(right)
        }
        BoolExpr::Not(value) => match value.as_ref() {
            BoolExpr::Cmp(op, left, right) => {
                normalize_add_overflow_cmp_for_solver(*op, left, right)
                    .map(BoolExpr::not)
                    .or_else(|| normalize_udiv_cmp_for_solver(*op, left, right).map(BoolExpr::not))
            }
            BoolExpr::Eq(left, Expr::Const(value)) if value.is_zero() => {
                if word_bool_always_true(left) {
                    Some(BoolExpr::Const(true))
                } else {
                    normalize_udiv_eq_zero(left, &Expr::Const(U256::ZERO)).map(BoolExpr::not)
                }
            }
            BoolExpr::Eq(Expr::Const(value), right) if value.is_zero() => {
                if word_bool_always_true(right) {
                    Some(BoolExpr::Const(true))
                } else {
                    normalize_udiv_eq_zero(&Expr::Const(U256::ZERO), right).map(BoolExpr::not)
                }
            }
            BoolExpr::Eq(left, right) => normalize_udiv_eq_zero(left, right).map(BoolExpr::not),
            _ => None,
        },
        BoolExpr::Eq(left, right) => normalize_udiv_eq_zero(left, right),
        BoolExpr::Cmp(op, left, right) => normalize_add_overflow_cmp_for_solver(*op, left, right)
            .or_else(|| normalize_udiv_cmp_for_solver(*op, left, right)),
        BoolExpr::Const(_) | BoolExpr::And(_) => None,
    }
}

/// Extracts the boolean condition represented by a word-valued `0`/`1` expression.
pub(crate) fn bool_from_word_expr(expr: &Expr) -> Option<BoolExpr> {
    let expr = strip_low_byte_mask(expr)?;
    let Expr::Ite(condition, then_expr, else_expr) = expr else { return None };
    match (then_expr.as_ref(), else_expr.as_ref()) {
        (Expr::Const(then_value), Expr::Const(else_value))
            if *then_value == U256::from(1) && else_value.is_zero() =>
        {
            Some(normalize_bool_for_solver((**condition).clone()))
        }
        (Expr::Const(then_value), Expr::Const(else_value))
            if then_value.is_zero() && *else_value == U256::from(1) =>
        {
            Some(normalize_bool_for_solver((**condition).clone()).not())
        }
        _ => None,
    }
}

/// Returns whether a word expression syntactically contains unsigned division.
pub(crate) fn expr_contains_udiv(expr: &Expr) -> bool {
    match expr {
        Expr::Const(_) | Expr::Var(_) | Expr::GasLeft(_) => false,
        Expr::Keccak(hash) => {
            expr_contains_udiv(&hash.len) || hash.bytes.iter().any(expr_contains_udiv)
        }
        Expr::Hash(hash) => hash.bytes.iter().any(expr_contains_udiv),
        Expr::Not(value) => expr_contains_udiv(value),
        Expr::Op(op, left, right) => {
            matches!(op, ExprOp::UDiv) || expr_contains_udiv(left) || expr_contains_udiv(right)
        }
        Expr::AddMod { left, right, modulus } | Expr::MulMod { left, right, modulus } => {
            expr_contains_udiv(left) || expr_contains_udiv(right) || expr_contains_udiv(modulus)
        }
        Expr::Ite(condition, then_expr, else_expr) => {
            bool_contains_udiv(condition)
                || expr_contains_udiv(then_expr)
                || expr_contains_udiv(else_expr)
        }
    }
}

/// Returns whether a boolean expression syntactically contains unsigned division.
pub(crate) fn bool_contains_udiv(expr: &BoolExpr) -> bool {
    match expr {
        BoolExpr::Const(_) => false,
        BoolExpr::Not(value) => bool_contains_udiv(value),
        BoolExpr::And(values) => values.iter().any(bool_contains_udiv),
        BoolExpr::Eq(left, right) | BoolExpr::Cmp(_, left, right) => {
            expr_contains_udiv(left) || expr_contains_udiv(right)
        }
    }
}

/// Rewrites exact unsigned-addition overflow checks when operand bounds preclude overflow.
pub(crate) fn normalize_add_overflow_cmp_for_solver(
    op: BoolExprOp,
    left: &Expr,
    right: &Expr,
) -> Option<BoolExpr> {
    match op {
        BoolExprOp::Ugt if add_overflow_check(left, right) => Some(BoolExpr::Const(false)),
        BoolExprOp::Ult if add_overflow_check(right, left) => Some(BoolExpr::Const(false)),
        _ => None,
    }
}

/// Returns whether `left > left + increment` is an impossible overflow check.
pub(crate) fn add_overflow_check(left: &Expr, right: &Expr) -> bool {
    let Some((base, increment)) = add_with_operand(right, left) else { return false };
    base == left && add_cannot_overflow_256(base, increment)
}

/// Returns the operands if `expr` is an addition involving `operand`.
pub(crate) fn add_with_operand<'a>(expr: &'a Expr, operand: &Expr) -> Option<(&'a Expr, &'a Expr)> {
    let Expr::Op(ExprOp::Add, left, right) = expr else { return None };
    if left.as_ref() == operand {
        Some((left, right))
    } else if right.as_ref() == operand {
        Some((right, left))
    } else {
        None
    }
}

/// Returns whether unsigned addition of two expressions cannot overflow 256 bits.
pub(crate) fn add_cannot_overflow_256(left: &Expr, right: &Expr) -> bool {
    expr_unsigned_bits(left).max(expr_unsigned_bits(right)).saturating_add(1) <= 256
}

/// Returns whether a word-valued boolean expression is an exact tautology.
pub(crate) fn word_bool_always_true(expr: &Expr) -> bool {
    ConstraintContext::default().word_bool_always_true(expr)
}

/// Converts one `0`/`1` word boolean term into its boolean condition.
pub(crate) fn word_bool_term(expr: &Expr) -> Option<&BoolExpr> {
    let Expr::Ite(condition, then_expr, else_expr) = expr else { return None };
    match (then_expr.as_ref(), else_expr.as_ref()) {
        (Expr::Const(then_value), Expr::Const(else_value))
            if *then_value == U256::from(1) && else_value.is_zero() =>
        {
            Some(condition)
        }
        _ => None,
    }
}

/// Returns the operand tested by `operand == 0`.
pub(crate) fn zero_check_operand(expr: &BoolExpr) -> Option<&Expr> {
    match expr {
        BoolExpr::Eq(left, Expr::Const(value)) if value.is_zero() => Some(left),
        BoolExpr::Eq(Expr::Const(value), right) if value.is_zero() => Some(right),
        _ => None,
    }
}

impl ConstraintContext {
    fn word_bool_always_true(&self, expr: &Expr) -> bool {
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

    fn checked_mul_guard_for_operand(&self, expr: &BoolExpr, zero_operand: &Expr) -> bool {
        let BoolExpr::Eq(left, right) = expr else { return false };
        self.checked_mul_guard_side(left, right, zero_operand)
            || self.checked_mul_guard_side(right, left, zero_operand)
    }

    fn checked_mul_guard_side(
        &self,
        div_expr: &Expr,
        expected: &Expr,
        zero_operand: &Expr,
    ) -> bool {
        let Expr::Ite(condition, then_expr, else_expr) = div_expr else { return false };
        if zero_check_operand(condition).is_none_or(|operand| operand != zero_operand) {
            return false;
        }
        if !matches!(then_expr.as_ref(), Expr::Const(value) if value.is_zero()) {
            return false;
        }
        let Some((numerator, denominator)) = udiv_operands(else_expr) else { return false };
        if denominator != zero_operand {
            return false;
        }
        let Expr::Op(ExprOp::Mul, left, right) = numerator else { return false };
        let other = if left.as_ref() == zero_operand {
            right.as_ref()
        } else if right.as_ref() == zero_operand {
            left.as_ref()
        } else {
            return false;
        };
        other == expected && self.mul_cannot_overflow_256(zero_operand, other)
    }

    fn mul_cannot_overflow_256(&self, left: &Expr, right: &Expr) -> bool {
        self.expr_unsigned_bits(left).saturating_add(self.expr_unsigned_bits(right)) <= 256
    }

    fn expr_unsigned_bits(&self, expr: &Expr) -> usize {
        let bits = match expr {
            Expr::Const(_)
            | Expr::Var(_)
            | Expr::GasLeft(_)
            | Expr::Keccak(_)
            | Expr::Hash(_)
            | Expr::Not(_) => expr_unsigned_bits(expr),
            Expr::Op(ExprOp::And, left, right) => match (left.as_ref(), right.as_ref()) {
                (expr, Expr::Const(mask)) | (Expr::Const(mask), expr) => {
                    self.expr_unsigned_bits(expr).min(mask.bit_len())
                }
                _ => 256,
            },
            Expr::Op(ExprOp::Add, left, right) => self
                .expr_unsigned_bits(left)
                .max(self.expr_unsigned_bits(right))
                .saturating_add(1)
                .min(256),
            Expr::Op(ExprOp::Mul, left, right) => self
                .expr_unsigned_bits(left)
                .saturating_add(self.expr_unsigned_bits(right))
                .min(256),
            Expr::Op(ExprOp::UDiv, left, _) => self.expr_unsigned_bits(left),
            Expr::Ite(_, left, right) => {
                self.expr_unsigned_bits(left).max(self.expr_unsigned_bits(right))
            }
            _ => 256,
        };

        self.upper_bound(expr).map(|bound| bits.min(bound.bit_len().max(1))).unwrap_or(bits)
    }
}

/// Returns whether unsigned multiplication of two expressions cannot overflow 256 bits.
pub(crate) fn mul_cannot_overflow_256(left: &Expr, right: &Expr) -> bool {
    expr_unsigned_bits(left).saturating_add(expr_unsigned_bits(right)) <= 256
}

/// Returns a conservative unsigned bit-width upper bound for an expression.
pub(crate) fn expr_unsigned_bits(expr: &Expr) -> usize {
    match expr {
        Expr::Const(value) => value.bit_len().max(1),
        Expr::Op(ExprOp::And, left, right) => match (left.as_ref(), right.as_ref()) {
            (expr, Expr::Const(mask)) | (Expr::Const(mask), expr) => {
                expr_unsigned_bits(expr).min(mask.bit_len())
            }
            _ => 256,
        },
        Expr::Op(ExprOp::Add, left, right) => {
            expr_unsigned_bits(left).max(expr_unsigned_bits(right)).saturating_add(1).min(256)
        }
        Expr::Op(ExprOp::Mul, left, right) => {
            expr_unsigned_bits(left).saturating_add(expr_unsigned_bits(right)).min(256)
        }
        Expr::Op(ExprOp::UDiv, left, _) => expr_unsigned_bits(left),
        Expr::AddMod { modulus, .. } | Expr::MulMod { modulus, .. } => expr_unsigned_bits(modulus),
        Expr::Ite(_, left, right) => expr_unsigned_bits(left).max(expr_unsigned_bits(right)),
        _ => 256,
    }
}

/// Rewrites `udiv(a, b) == 0` predicates using EVM division-by-zero semantics.
pub(crate) fn normalize_udiv_eq_zero(left: &Expr, right: &Expr) -> Option<BoolExpr> {
    if matches!(right, Expr::Const(value) if value.is_zero())
        && let Some(condition) = normalize_expr_eq_zero_for_solver(left)
    {
        return Some(condition);
    }
    if matches!(left, Expr::Const(value) if value.is_zero())
        && let Some(condition) = normalize_expr_eq_zero_for_solver(right)
    {
        return Some(condition);
    }
    None
}

/// Rewrites `expr == 0` when `expr` contains exactly-normalizable unsigned division.
pub(crate) fn normalize_expr_eq_zero_for_solver(expr: &Expr) -> Option<BoolExpr> {
    if let Some((numerator, denominator)) = udiv_operands(expr) {
        return Some(udiv_zero_condition(numerator, denominator));
    }
    if let Expr::Ite(condition, then_expr, else_expr) = expr {
        let then_zero = normalize_expr_eq_zero_for_solver(then_expr).unwrap_or_else(|| {
            BoolExpr::eq(normalize_expr_for_solver((**then_expr).clone()), Expr::Const(U256::ZERO))
        });
        let else_zero = normalize_expr_eq_zero_for_solver(else_expr).unwrap_or_else(|| {
            BoolExpr::eq(normalize_expr_for_solver((**else_expr).clone()), Expr::Const(U256::ZERO))
        });
        if bool_contains_udiv(&then_zero) || bool_contains_udiv(&else_zero) {
            return None;
        }
        return Some(BoolExpr::or(vec![
            BoolExpr::and(vec![normalize_bool_for_solver((**condition).clone()), then_zero]),
            BoolExpr::and(vec![normalize_bool_for_solver((**condition).clone()).not(), else_zero]),
        ]));
    }
    None
}

/// Rewrites `expr != 0` when `expr` contains exactly-normalizable unsigned division.
pub(crate) fn normalize_expr_ne_zero_for_solver(expr: &Expr) -> Option<BoolExpr> {
    if let Some((numerator, denominator)) = udiv_operands(expr) {
        return Some(udiv_nonzero_condition(numerator, denominator));
    }
    if let Expr::Ite(condition, then_expr, else_expr) = expr {
        let then_nonzero = normalize_expr_ne_zero_for_solver(then_expr).unwrap_or_else(|| {
            BoolExpr::eq(normalize_expr_for_solver((**then_expr).clone()), Expr::Const(U256::ZERO))
                .not()
        });
        let else_nonzero = normalize_expr_ne_zero_for_solver(else_expr).unwrap_or_else(|| {
            BoolExpr::eq(normalize_expr_for_solver((**else_expr).clone()), Expr::Const(U256::ZERO))
                .not()
        });
        if bool_contains_udiv(&then_nonzero) || bool_contains_udiv(&else_nonzero) {
            return None;
        }
        return Some(BoolExpr::or(vec![
            BoolExpr::and(vec![normalize_bool_for_solver((**condition).clone()), then_nonzero]),
            BoolExpr::and(vec![
                normalize_bool_for_solver((**condition).clone()).not(),
                else_nonzero,
            ]),
        ]));
    }
    None
}

/// Rewrites `udiv(a, b)` zero/nonzero comparisons using EVM division-by-zero semantics.
pub(crate) fn normalize_udiv_cmp_for_solver(
    op: BoolExprOp,
    left: &Expr,
    right: &Expr,
) -> Option<BoolExpr> {
    match (op, left, right) {
        (BoolExprOp::Ugt, div, Expr::Const(value)) if value.is_zero() => {
            normalize_expr_ne_zero_for_solver(div)
        }
        (BoolExprOp::Uge, div, Expr::Const(value)) if *value == U256::from(1) => {
            normalize_expr_ne_zero_for_solver(div)
        }
        (BoolExprOp::Ule, div, Expr::Const(value)) if value.is_zero() => {
            normalize_expr_eq_zero_for_solver(div)
        }
        (BoolExprOp::Ult, div, Expr::Const(value)) if *value == U256::from(1) => {
            normalize_expr_eq_zero_for_solver(div)
        }
        (BoolExprOp::Ult, Expr::Const(value), div) if value.is_zero() => {
            normalize_expr_ne_zero_for_solver(div)
        }
        (BoolExprOp::Ule, Expr::Const(value), div) if *value == U256::from(1) => {
            normalize_expr_ne_zero_for_solver(div)
        }
        (BoolExprOp::Uge, Expr::Const(value), div) if value.is_zero() => {
            normalize_expr_eq_zero_for_solver(div)
        }
        (BoolExprOp::Ugt, Expr::Const(value), div) if *value == U256::from(1) => {
            normalize_expr_eq_zero_for_solver(div)
        }
        _ => None,
    }
}

/// Returns the operands for an unsigned division expression.
pub(crate) fn udiv_operands(expr: &Expr) -> Option<(&Expr, &Expr)> {
    match expr {
        Expr::Op(ExprOp::UDiv, numerator, denominator) => Some((numerator, denominator)),
        _ => None,
    }
}

/// Builds the exact condition for EVM `udiv(numerator, denominator) == 0`.
pub(crate) fn udiv_zero_condition(numerator: &Expr, denominator: &Expr) -> BoolExpr {
    BoolExpr::or(vec![
        BoolExpr::eq(normalize_expr_for_solver(denominator.clone()), Expr::Const(U256::ZERO)),
        BoolExpr::cmp(
            BoolExprOp::Ult,
            normalize_expr_for_solver(numerator.clone()),
            normalize_expr_for_solver(denominator.clone()),
        ),
    ])
}

/// Builds the exact condition for EVM `udiv(numerator, denominator) != 0`.
pub(crate) fn udiv_nonzero_condition(numerator: &Expr, denominator: &Expr) -> BoolExpr {
    BoolExpr::and(vec![
        BoolExpr::eq(normalize_expr_for_solver(denominator.clone()), Expr::Const(U256::ZERO)).not(),
        BoolExpr::cmp(
            BoolExprOp::Uge,
            normalize_expr_for_solver(numerator.clone()),
            normalize_expr_for_solver(denominator.clone()),
        ),
    ])
}
