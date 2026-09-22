//! Constraint and expression normalization for solver queries.

use super::*;

mod polynomial;
mod rounding;

use polynomial::polynomial_identity;

/// Normalizes path constraints into an equivalent, solver-friendlier form.
#[cfg(test)]
pub(crate) fn normalize_constraints_for_solver(
    cx: &mut SymCx,
    constraints: &[SymBoolExpr],
) -> Vec<SymBoolExpr> {
    normalize_constraints_for_solver_with(cx, constraints, |cx, constraint| {
        normalize_bool_for_solver(cx, constraint.clone())
    })
}

/// Reuses context-free normalization results while retaining per-query contextual rewrites.
pub(super) fn normalize_constraints_for_solver_cached(
    cx: &mut SymCx,
    constraints: &[SymBoolExpr],
    normalization_cache: &mut HashMap<SymBoolExpr, SymBoolExpr>,
) -> Vec<SymBoolExpr> {
    normalize_constraints_for_solver_with(cx, constraints, |cx, constraint| {
        if let Some(normalized) = normalization_cache.get(constraint) {
            return normalized.clone();
        }
        let normalized = normalize_bool_for_solver(cx, constraint.clone());
        // These are strong hash-consed handles, so bound their lifetime like the SAT cache.
        if normalization_cache.len() < SYMBOLIC_SOLVER_SAT_CACHE_MAX_ENTRIES {
            normalization_cache.insert(constraint.clone(), normalized.clone());
        }
        normalized
    })
}

fn normalize_constraints_for_solver_with(
    cx: &mut SymCx,
    constraints: &[SymBoolExpr],
    mut normalize: impl FnMut(&mut SymCx, &SymBoolExpr) -> SymBoolExpr,
) -> Vec<SymBoolExpr> {
    let mut changed_conjuncts = HashSet::default();
    let normalized = normalize_constraint_batch(
        constraints.iter().map(|constraint| {
            let normalized = normalize(cx, constraint);
            if normalized != *constraint {
                mark_conjuncts(&normalized, &mut changed_conjuncts);
            }
            normalized
        }),
        constraints.len(),
    );
    if matches!(normalized.as_slice(), [expr] if expr.as_const() == Some(false)) {
        return normalized;
    }

    // Context-dependent rewrites must not contribute facts to the context that proves them. Mark
    // candidates by syntax rather than by whether the full context happens to prove a rewrite:
    // contradictory bounds can make an interval unavailable until another candidate is removed.
    let retained_count = normalized
        .iter()
        .filter(|constraint| !ConstraintContext::requires_independent_context(constraint))
        .count();
    let retained = normalized
        .iter()
        .filter(|constraint| !ConstraintContext::requires_independent_context(constraint));
    let context =
        ConstraintContext::from_constraints_with_lower_bounds(retained, retained_count, false);
    let normalized_len = normalized.len();
    let normalized = normalize_constraint_batch(
        normalized.into_iter().map(|constraint| {
            let changed = changed_conjuncts.contains(&constraint);
            context.normalize_bool(cx, constraint, changed)
        }),
        normalized_len,
    );
    normalize_bounded_comparisons(cx, normalized)
}

/// Simplifies predicates using only the other, still-retained conjuncts.
fn normalize_bounded_comparisons(
    cx: &mut SymCx,
    mut constraints: Vec<SymBoolExpr>,
) -> Vec<SymBoolExpr> {
    // Later predicates can expose guards needed by earlier ones. Revisit the retained
    // conjunction, but bound the work; unfinished simplification is still sound SMT input.
    for _ in 0..MAX_CONTEXTUAL_PASSES {
        let previous = constraints.clone();
        let mut index = 0;
        while index < constraints.len() {
            let context = ConstraintContext::for_rewrite(cx, &constraints, index);
            constraints[index] = context.normalize_bool(cx, constraints[index].clone(), false);
            // Revisit each newly exposed conjunct with the retained supporting facts. Otherwise a
            // division rewrite can leave a simple contradiction hidden until the SMT fallback.
            if let SymBoolExprKind::And(terms) = constraints[index].kind() {
                let terms = terms.to_vec();
                constraints.splice(index..=index, terms);
                continue;
            }
            match context.bounded_bool_value(&constraints[index]) {
                Some(false) => return vec![SymBoolExpr::constant(cx, false)],
                Some(true) => {
                    constraints.remove(index);
                }
                None => index += 1,
            }
        }
        // Contextual rewrites may change sort order or expose a conjunction.
        let count = constraints.len();
        constraints = normalize_constraint_batch(constraints, count);
        if constraints == previous || constraints.iter().any(|c| c.as_const() == Some(false)) {
            break;
        }
    }
    constraints
}

fn mark_conjuncts(expr: &SymBoolExpr, out: &mut HashSet<SymBoolExpr>) {
    let mut pending = vec![expr.clone()];
    while let Some(expr) = pending.pop() {
        if !out.insert(expr.clone()) {
            continue;
        }
        if let SymBoolExprKind::And(values) = expr.kind() {
            pending.extend(values.iter().cloned());
        }
    }
}

fn normalize_constraint_batch(
    constraints: impl IntoIterator<Item = SymBoolExpr>,
    capacity: usize,
) -> Vec<SymBoolExpr> {
    let mut normalized = Vec::with_capacity(capacity);
    for constraint in constraints {
        if constraint.as_const() == Some(false) {
            return vec![constraint];
        }
        constraint.push_normalized_conjuncts(&mut normalized);
    }
    sort_dedup_bool_exprs(&mut normalized);
    normalized
}

fn sort_dedup_bool_exprs(exprs: &mut Vec<SymBoolExpr>) {
    // Hash-consing already caches deterministic structural hashes. Only render full structural
    // keys for the exceedingly rare case where two distinct expressions collide.
    exprs.sort_unstable_by(bool_expr_cmp);
    exprs.dedup();
}

fn bool_expr_cmp(left: &SymBoolExpr, right: &SymBoolExpr) -> std::cmp::Ordering {
    if left == right {
        return std::cmp::Ordering::Equal;
    }
    left.stable_hash_cmp(right)
        .then_with(|| bool_structural_key(left).cmp(&bool_structural_key(right)))
}

fn bool_structural_key(expr: &SymBoolExpr) -> String {
    let mut key = String::new();
    write_bool_structural_key(&mut key, expr);
    key
}

fn write_bool_structural_key(out: &mut String, expr: &SymBoolExpr) {
    match expr.kind() {
        SymBoolExprKind::Const(value) => {
            let _ = write!(out, "0:{value}");
        }
        SymBoolExprKind::Not(value) => {
            out.push_str("1:");
            write_bool_structural_key(out, value);
        }
        SymBoolExprKind::And(values) => {
            let _ = write!(out, "2:{}:", values.len());
            for value in values.iter() {
                write_bool_structural_key(out, value);
                out.push(';');
            }
        }
        SymBoolExprKind::Cmp(op, left, right) => {
            let _ = write!(out, "3:{}:", cmp_op_key(*op));
            write_expr_structural_key(out, left);
            out.push(':');
            write_expr_structural_key(out, right);
        }
    }
}

fn write_expr_structural_key(out: &mut String, expr: &SymExpr) {
    match expr.kind() {
        SymExprKind::Const(value) => {
            let _ = write!(out, "0:{value:064x}");
        }
        SymExprKind::Var(name) => {
            let _ = write!(out, "1:{}", name.id());
        }
        SymExprKind::GasLeft(symbol) => {
            let _ = write!(out, "2:{}", symbol.id());
        }
        SymExprKind::Keccak { name, len, bytes } => {
            let _ = write!(out, "3:{}:", name.id());
            write_expr_structural_key(out, len);
            write_exprs_structural_key(out, bytes);
        }
        SymExprKind::Hash { name, algorithm, bytes } => {
            let _ = write!(out, "4:{}:{algorithm}:", name.id());
            write_exprs_structural_key(out, bytes);
        }
        SymExprKind::Not(value) => {
            out.push_str("5:");
            write_expr_structural_key(out, value);
        }
        SymExprKind::BinOp(op, left, right) => {
            let _ = write!(out, "6:{}:", expr_binop_key(*op));
            write_expr_structural_key(out, left);
            out.push(':');
            write_expr_structural_key(out, right);
        }
        SymExprKind::TernOp(op, left, right, modulus) => {
            let _ = write!(out, "7:{}:", expr_ternop_key(*op));
            write_expr_structural_key(out, left);
            out.push(':');
            write_expr_structural_key(out, right);
            out.push(':');
            write_expr_structural_key(out, modulus);
        }
        SymExprKind::Ite(condition, then_expr, else_expr) => {
            out.push_str("9:");
            write_bool_structural_key(out, condition);
            out.push(':');
            write_expr_structural_key(out, then_expr);
            out.push(':');
            write_expr_structural_key(out, else_expr);
        }
    }
}

fn write_exprs_structural_key(out: &mut String, exprs: &[SymExpr]) {
    let _ = write!(out, "{}:", exprs.len());
    for expr in exprs {
        write_expr_structural_key(out, expr);
        out.push(';');
    }
}

const fn cmp_op_key(op: SymCmpOp) -> u8 {
    match op {
        SymCmpOp::Eq => 0,
        SymCmpOp::Ult => 1,
        SymCmpOp::Ugt => 2,
        SymCmpOp::Ule => 3,
        SymCmpOp::Uge => 4,
        SymCmpOp::Slt => 5,
        SymCmpOp::Sgt => 6,
    }
}

const fn expr_binop_key(op: SymBinOp) -> u8 {
    match op {
        SymBinOp::Add => 0,
        SymBinOp::Sub => 1,
        SymBinOp::Mul => 2,
        SymBinOp::UDiv => 3,
        SymBinOp::URem => 4,
        SymBinOp::SDiv => 5,
        SymBinOp::SRem => 6,
        SymBinOp::And => 7,
        SymBinOp::Or => 8,
        SymBinOp::Xor => 9,
        SymBinOp::Shl => 10,
        SymBinOp::Shr => 11,
        SymBinOp::Sar => 12,
    }
}

const fn expr_ternop_key(op: SymTernOp) -> u8 {
    match op {
        SymTernOp::AddMod => 0,
        SymTernOp::MulMod => 1,
    }
}

/// Returns whether canonically ordered normalized constraints contain a direct contradiction.
pub(super) fn constraints_are_directly_unsat(cx: &mut SymCx, constraints: &[SymBoolExpr]) -> bool {
    let mut derived = Vec::new();
    for constraint in constraints {
        let Some(fact) = bitwise_bool_word_fact(cx, constraint) else {
            continue;
        };
        if let SymBoolExprKind::And(values) = fact.kind() {
            // A positive conjunction implies each member independently. Retain the aggregate for
            // exact matches, but expose its members to the direct contradiction check as well.
            derived.extend(values.iter().cloned());
        }
        derived.push(fact);
    }
    let contains = |expected: &SymBoolExpr| {
        constraints.binary_search_by(|candidate| bool_expr_cmp(candidate, expected)).is_ok()
            || derived.contains(expected)
    };
    constraints.iter().chain(&derived).any(|constraint| match constraint.kind() {
        SymBoolExprKind::Const(false) => true,
        SymBoolExprKind::Not(inner)
            if let SymBoolExprKind::And(values) = inner.kind()
                && values.iter().all(&contains) =>
        {
            true
        }
        SymBoolExprKind::Not(inner) => contains(inner),
        _ => {
            let negated = constraint.clone().not(cx);
            contains(&negated)
        }
    })
}

fn bitwise_bool_word_fact(cx: &mut SymCx, constraint: &SymBoolExpr) -> Option<SymBoolExpr> {
    match constraint.kind() {
        SymBoolExprKind::Cmp(SymCmpOp::Eq, left, right)
            if right.as_const().is_some_and(|value| value.is_zero()) =>
        {
            left.bitwise_bool_word_condition(cx).map(|condition| condition.not(cx))
        }
        SymBoolExprKind::Not(inner) => {
            let SymBoolExprKind::Cmp(SymCmpOp::Eq, left, right) = inner.kind() else {
                return None;
            };
            if !right.as_const().is_some_and(|value| value.is_zero()) {
                return None;
            }
            left.bitwise_bool_word_condition(cx)
        }
        _ => None,
    }
}

/// Returns whether every expression in `subset` appears in `superset`.
pub(super) fn sorted_bool_exprs_are_subset(
    subset: &[SymBoolExpr],
    superset: &[SymBoolExpr],
) -> bool {
    if subset.len() > superset.len() {
        return false;
    }

    let superset: HashSet<_> = superset.iter().collect();
    subset.iter().all(|expected| superset.contains(expected))
}

/// Normalizes one boolean expression into an equivalent, solver-friendlier form.
pub(crate) fn normalize_bool_for_solver(cx: &mut SymCx, expr: SymBoolExpr) -> SymBoolExpr {
    expr.fold(cx, &mut normalize_bool_node_for_solver)
}

impl SymBoolExpr {
    fn push_normalized_conjuncts(self, out: &mut Vec<Self>) {
        match self.kind() {
            SymBoolExprKind::Const(true) => {}
            SymBoolExprKind::And(values) => {
                for value in values.iter().cloned() {
                    value.push_normalized_conjuncts(out);
                }
            }
            _ => out.push(self),
        }
    }
}

fn normalize_bool_node_for_solver(cx: &mut SymCx, expr: SymBoolExpr) -> SymBoolExpr {
    if let Some(normalized) = expr.normalize_udiv_for_solver(cx) {
        return normalized;
    }

    match expr.kind() {
        SymBoolExprKind::Not(value) => match value.kind() {
            SymBoolExprKind::Cmp(SymCmpOp::Ult, left, right)
                if matches!(left.kind(), SymExprKind::Not(_)) =>
            {
                normalize_cmp_for_solver(cx, SymCmpOp::Ule, right.clone(), left.clone())
            }
            _ => expr,
        },
        SymBoolExprKind::Cmp(op, left, right) => {
            let left = normalize_expr_for_solver(cx, left.clone());
            let right = normalize_expr_for_solver(cx, right.clone());
            if *op == SymCmpOp::Eq && polynomial_identity(&left, &right) {
                return SymBoolExpr::constant(cx, true);
            }
            let normalized = normalize_cmp_for_solver(cx, *op, left, right);
            normalized.normalize_udiv_for_solver(cx).unwrap_or(normalized)
        }
        _ => expr,
    }
}

fn normalize_cmp_for_solver(
    cx: &mut SymCx,
    op: SymCmpOp,
    left: SymExpr,
    right: SymExpr,
) -> SymBoolExpr {
    if op == SymCmpOp::Eq {
        for (quotient, expected) in [(&left, &right), (&right, &left)] {
            if let Some((denominator, value)) =
                ConstraintContext::mul_div_identity_operands(quotient, expected)
                && let Some(factor) = denominator.as_const().filter(|value| !value.is_zero())
            {
                // For constant k > 0, (x * k mod 2^256) / k == x iff x <= MAX / k.
                // The quotient cannot exceed MAX / k; conversely this bound prevents wrapping.
                // Retain that exact bound instead of asking SMT to solve the overflow check.
                // `SymExpr::binop` folds a zero factor away, so the non-zero filter is only a
                // defensive guard against `MAX / 0` should that folding ever change.
                return SymBoolExpr::cmp_word_const(cx, SymCmpOp::Ule, value, U256::MAX / factor);
            }
        }
        if right.as_const().is_some_and(|value| value.is_zero())
            && let SymExprKind::BinOp(SymBinOp::Sub, minuend, subtrahend) = left.kind()
        {
            // Word subtraction is zero exactly when both operands are equal, including at the
            // modular boundary. Solc commonly lowers optimized equality checks to this shape.
            return SymBoolExpr::eq(cx, minuend.clone(), subtrahend.clone());
        }
        if left.as_const().is_some_and(|value| value.is_zero())
            && let SymExprKind::BinOp(SymBinOp::Sub, minuend, subtrahend) = right.kind()
        {
            return SymBoolExpr::eq(cx, minuend.clone(), subtrahend.clone());
        }
    }

    let (left, right) =
        if matches!(op, SymCmpOp::Ult | SymCmpOp::Ule | SymCmpOp::Ugt | SymCmpOp::Uge) {
            // Complement reverses unsigned order: ~x = MAX - x. Move it onto
            // the constant so interval analysis can see Solidity's addition guard.
            match (left.kind(), right.kind()) {
                (SymExprKind::Not(value), SymExprKind::Const(limit)) => {
                    (SymExpr::constant(cx, !*limit), value.clone())
                }
                (SymExprKind::Const(limit), SymExprKind::Not(value)) => {
                    (value.clone(), SymExpr::constant(cx, !*limit))
                }
                _ => (left, right),
            }
        } else {
            (left, right)
        };

    match op {
        // `a > b => b < a`.
        SymCmpOp::Ugt => SymBoolExpr::cmp(cx, SymCmpOp::Ult, right, left),
        // `a >= b => b <= a`.
        SymCmpOp::Uge => SymBoolExpr::cmp(cx, SymCmpOp::Ule, right, left),
        // `a >s b => b <s a`.
        SymCmpOp::Sgt => SymBoolExpr::cmp(cx, SymCmpOp::Slt, right, left),
        SymCmpOp::Eq | SymCmpOp::Ult | SymCmpOp::Ule | SymCmpOp::Slt => {
            SymBoolExpr::cmp(cx, op, left, right)
        }
    }
}

/// Simple facts learned from the normalized conjunction currently being queried.
#[derive(Default)]
pub(super) struct ConstraintContext {
    upper_bounds: HashMap<SymExpr, U256>,
    lower_bounds: HashMap<SymExpr, U256>,
    unsigned_lower_bounds: HashMap<SymExpr, U256>,
    exact_values: HashMap<SymExpr, U256>,
    conflicting_exact_values: HashSet<SymExpr>,
    non_wrapping_products: HashSet<(SymExpr, SymExpr)>,
}

#[derive(Clone, Copy)]
struct WordInterval {
    min: U256,
    max: U256,
}

// These analyses are solver optimizations, so exceeding their local work budget must only make
// them decline a rewrite. Keeping the bound shared and private prevents deeply nested bytecode
// expressions from turning a proof shortcut into unbounded Rust recursion.
const MAX_LOCAL_ANALYSIS_NODES: usize = 256;
const MAX_CONTEXTUAL_PASSES: usize = 4;

impl WordInterval {
    fn new(min: U256, max: U256) -> Option<Self> {
        (min <= max).then_some(Self { min, max })
    }

    const fn exact(value: U256) -> Self {
        Self { min: value, max: value }
    }

    fn with_bounds(self, lower: Option<U256>, upper: Option<U256>) -> Option<Self> {
        Self::new(
            self.min.max(lower.unwrap_or(U256::ZERO)),
            self.max.min(upper.unwrap_or(U256::MAX)),
        )
    }
}

impl ConstraintContext {
    pub(super) fn new(constraints: &[SymBoolExpr]) -> Self {
        Self::from_constraints(constraints.iter(), constraints.len())
    }

    fn from_constraints<'a>(
        constraints: impl Clone + Iterator<Item = &'a SymBoolExpr>,
        constraint_count: usize,
    ) -> Self {
        Self::from_constraints_with_lower_bounds(constraints, constraint_count, true)
    }

    /// Builds a rewrite context from the other retained conjuncts, never the predicate itself.
    fn for_rewrite(cx: &mut SymCx, constraints: &[SymBoolExpr], index: usize) -> Self {
        let supporting = constraints
            .iter()
            .enumerate()
            .filter_map(|(i, constraint)| (i != index).then_some(constraint));
        let mut context = Self::from_constraints(supporting.clone(), constraints.len() - 1);
        // One successful product guard may bound an operand used in another guard.
        for _ in 0..MAX_CONTEXTUAL_PASSES {
            let mut changed = false;
            for constraint in supporting.clone() {
                changed |= context.record_non_wrapping_product(cx, constraint);
            }
            if !changed {
                break;
            }
        }
        // Product bounds can then turn scaled zero checks into exact operand facts.
        for constraint in supporting {
            context.record_scaled_zero_fact(cx, constraint);
        }
        context
    }

    fn from_constraints_with_lower_bounds<'a>(
        constraints: impl Clone + Iterator<Item = &'a SymBoolExpr>,
        constraint_count: usize,
        promote_unsigned_bounds: bool,
    ) -> Self {
        let mut context = Self::default();
        for constraint in constraints.clone() {
            context.record_exact_value_constraint(constraint);
            context.record_upper_bound_constraint(constraint);
            context.record_lower_bound_constraint(constraint);
            context.record_unsigned_lower_bound_constraint(constraint, promote_unsigned_bounds);
        }
        // A bounded number of rounds closes ordinary order chains. Relational propagation keeps
        // strict comparisons weak (`a < b` propagates only `a <= upper(b)`), so inconsistent
        // cycles cannot tighten a bound one integer at a time across the uint256 domain.
        for _ in 0..constraint_count {
            let mut changed = false;
            for constraint in constraints.clone() {
                changed |= context.propagate_order_bounds(constraint);
            }
            if !changed {
                break;
            }
        }
        context
    }

    fn upper_bound(&self, expr: &SymExpr) -> Option<U256> {
        self.upper_bounds.get(expr).copied()
    }

    fn lower_bound(&self, expr: &SymExpr) -> Option<U256> {
        self.lower_bounds.get(expr).copied()
    }

    /// Conservatively identifies every conjunct that path facts may rewrite.
    fn requires_independent_context(expr: &SymBoolExpr) -> bool {
        let root_candidate = match expr.kind() {
            SymBoolExprKind::Cmp(op, left, right) => match op {
                SymCmpOp::Eq => {
                    Self::mul_div_identity_operands(left, right).is_some()
                        || Self::mul_div_identity_operands(right, left).is_some()
                        || Self::masked_word_side_eq_self_shape(left, right).is_some()
                        || Self::masked_word_side_eq_self_shape(right, left).is_some()
                }
                SymCmpOp::Ult | SymCmpOp::Ule => {
                    Self::udiv_comparison_operands(*op, left, right).is_some()
                }
                SymCmpOp::Slt | SymCmpOp::Sgt => true,
                SymCmpOp::Ugt | SymCmpOp::Uge => false,
            },
            SymBoolExprKind::Not(value) => match value.kind() {
                SymBoolExprKind::Cmp(op, left, right)
                    if Self::udiv_comparison_operands(*op, left, right).is_some() =>
                {
                    true
                }
                SymBoolExprKind::Cmp(SymCmpOp::Slt | SymCmpOp::Sgt, _, _) => true,
                _ => value.zero_check_operand().is_some_and(|word| {
                    matches!(word.kind(), SymExprKind::BinOp(SymBinOp::Or, _, _))
                }),
            },
            SymBoolExprKind::Const(_) | SymBoolExprKind::And(_) => false,
        };
        root_candidate
            || expr.contains_udiv()
            || expr.visit_unique_bool(|word| matches!(word.kind(), SymExprKind::Ite(_, _, _)))
    }

    fn normalize_bool(
        &self,
        cx: &mut SymCx,
        expr: SymBoolExpr,
        context_free_changed: bool,
    ) -> SymBoolExpr {
        let may_normalize_word = !self.is_exact_value_constraint(&expr)
            && expr.visit_unique_bool(|word| self.may_normalize_word(word));
        let expr = if may_normalize_word {
            let expr = expr.fold_exprs(cx, &mut |cx, expr| self.normalize_word(cx, expr));
            normalize_bool_for_solver(cx, expr)
        } else if context_free_changed {
            // The first pass can create new Boolean predicates, such as an overflow comparison
            // while eliminating a division. Normalize those predicates before applying facts.
            normalize_bool_for_solver(cx, expr)
        } else {
            expr
        };
        if let Some(normalized) = self.normalize_signed_add_comparison(cx, &expr) {
            return normalized;
        }
        if let Some(value) = self.rounding_comparison_value(&expr) {
            return SymBoolExpr::constant(cx, value);
        }
        if let SymBoolExprKind::Not(value) = expr.kind()
            && let Some(normalized) = self.normalize_signed_add_comparison(cx, value)
        {
            return normalized.not(cx);
        }
        if let SymBoolExprKind::Cmp(op, left, right) = expr.kind()
            && let Some(normalized) = self.normalize_udiv_comparison(cx, *op, left, right)
        {
            return normalized;
        }
        if let SymBoolExprKind::Not(value) = expr.kind()
            && let SymBoolExprKind::Cmp(op, left, right) = value.kind()
            && let Some(normalized) = self.normalize_udiv_comparison(cx, *op, left, right)
        {
            return normalized.not(cx);
        }

        match expr.kind() {
            SymBoolExprKind::Not(value) if self.unsigned_bool_always_true(value) => {
                SymBoolExpr::constant(cx, false)
            }
            SymBoolExprKind::Cmp(SymCmpOp::Eq, left, right)
                if self.mul_div_identity(left, right) || self.mul_div_identity(right, left) =>
            {
                SymBoolExpr::constant(cx, true)
            }
            SymBoolExprKind::Not(value)
                if matches!(
                    value.kind(),
                    SymBoolExprKind::Cmp(SymCmpOp::Eq, left, right)
                        if self.mul_div_identity(left, right)
                            || self.mul_div_identity(right, left)
                ) =>
            {
                SymBoolExpr::constant(cx, false)
            }
            SymBoolExprKind::Cmp(SymCmpOp::Eq, left, right)
                if self.masked_word_eq_self(left, right) =>
            {
                // `x & mask == x => true` when the current context proves `x <= mask`.
                SymBoolExpr::constant(cx, true)
            }
            SymBoolExprKind::Not(value) if self.masked_eq_self_condition(value) => {
                // `x & mask != x => false` when the current context proves `x <= mask`.
                SymBoolExpr::constant(cx, false)
            }
            _ if expr
                .zero_check_operand()
                .is_some_and(|left| self.word_bool_always_true(cx, left)) =>
            {
                // `always_true_word == 0 => false`.
                SymBoolExpr::constant(cx, false)
            }
            SymBoolExprKind::Not(value)
                if value
                    .zero_check_operand()
                    .is_some_and(|left| self.word_bool_always_true(cx, left)) =>
            {
                // `always_true_word != 0 => true`.
                SymBoolExpr::constant(cx, true)
            }
            _ => expr,
        }
    }

    fn record_exact_value_constraint(&mut self, constraint: &SymBoolExpr) {
        let SymBoolExprKind::Cmp(SymCmpOp::Eq, left, right) = constraint.kind() else {
            return;
        };
        let Some((expr, value)) = const_side_bound(left, right) else {
            return;
        };
        if !matches!(expr.kind(), SymExprKind::Var(_))
            || self.conflicting_exact_values.contains(expr)
        {
            return;
        }
        if self.exact_values.get(expr).is_some_and(|current| *current != value) {
            self.exact_values.remove(expr);
            self.conflicting_exact_values.insert(expr.clone());
        } else {
            self.exact_values.insert(expr.clone(), value);
        }
    }

    fn exact_value(&self, expr: &SymExpr) -> Option<U256> {
        self.exact_values.get(expr).copied()
    }

    fn may_normalize_word(&self, expr: &SymExpr) -> bool {
        if self.exact_values.contains_key(expr)
            || Self::mul_div_operands(expr).is_some()
            || Self::ceil_div_product(expr).is_some()
        {
            return true;
        }
        match expr.kind() {
            SymExprKind::Ite(_, _, _) => true,
            SymExprKind::BinOp(SymBinOp::Or, left, right) => {
                left.as_const() == Some(U256::ONE) || right.as_const() == Some(U256::ONE)
            }
            SymExprKind::BinOp(SymBinOp::Mul, _, _) => Self::constant_mul_operands(expr)
                .is_some_and(|(value, _)| Self::constant_mul_operands(value).is_some()),
            SymExprKind::BinOp(SymBinOp::UDiv, numerator, denominator) => {
                Self::rounded_product_operands(numerator).is_some()
                    || (denominator.as_const().is_some_and(|value| !value.is_zero())
                        && Self::constant_mul_operands(numerator).is_some())
            }
            _ => false,
        }
    }

    fn normalize_word(&self, cx: &mut SymCx, expr: SymExpr) -> SymExpr {
        if let Some(value) = self.exact_value(&expr) {
            return SymExpr::constant(cx, value);
        }
        if let SymExprKind::Ite(condition, then_value, else_value) = expr.kind() {
            if let Some(value) = self.bounded_bool_value(condition) {
                return if value { then_value.clone() } else { else_value.clone() };
            }
            if let Some(condition) = self.normalize_signed_add_comparison(cx, condition) {
                return SymExpr::ite(cx, condition, then_value.clone(), else_value.clone());
            }
        }
        if let Some(value) = self.quotient_of_rounded_product(&expr) {
            return value.clone();
        }
        if let SymExprKind::BinOp(SymBinOp::And, value, mask) = expr.kind()
            && mask.as_const() == Some(U256::ONE)
            && value.normalized_bool_word_condition(cx).is_some()
        {
            return value.clone();
        }
        if let SymExprKind::BinOp(SymBinOp::Or, left, right) = expr.kind()
            && ((left.as_const() == Some(U256::from(1))
                && right.normalized_bool_word_condition(cx).is_some())
                || (right.as_const() == Some(U256::from(1))
                    && left.normalized_bool_word_condition(cx).is_some()))
        {
            return SymExpr::one(cx);
        }
        if let Some((value, outer_factor)) = Self::constant_mul_operands(&expr)
            && let Some((value, inner_factor)) = Self::constant_mul_operands(value)
        {
            let factor = SymExpr::constant(cx, inner_factor.wrapping_mul(outer_factor));
            return SymExpr::binop(cx, SymBinOp::Mul, value.clone(), factor);
        }
        if let Some((value, factor)) =
            self.exact_ceil_div_factor(&expr).or_else(|| self.exact_scaled_div_factor(&expr))
        {
            let factor = SymExpr::constant(cx, factor);
            return SymExpr::binop(cx, SymBinOp::Mul, value.clone(), factor);
        }
        if let Some((denominator, other)) = Self::mul_div_operands(&expr)
            && self.interval(denominator).is_some_and(|interval| !interval.min.is_zero())
            && self.mul_cannot_overflow_256(denominator, other)
        {
            return other.clone();
        }
        expr
    }

    fn bounded_bool_value(&self, expr: &SymBoolExpr) -> Option<bool> {
        match expr.kind() {
            SymBoolExprKind::Const(value) => Some(*value),
            SymBoolExprKind::Not(value) => self.bounded_bool_value(value).map(|value| !value),
            SymBoolExprKind::Cmp(op, left, right) => {
                let left = self.interval(left)?;
                let right = self.interval(right)?;
                if *op == SymCmpOp::Eq {
                    return if left.max < right.min || right.max < left.min {
                        Some(false)
                    } else if left.min == left.max && right.min == right.max {
                        Some(left.min == right.min)
                    } else {
                        None
                    };
                }
                if matches!(op, SymCmpOp::Slt | SymCmpOp::Sgt)
                    && (left.min.bit(255) != left.max.bit(255)
                        || right.min.bit(255) != right.max.bit(255))
                {
                    return None;
                }
                let (always, possible) =
                    if matches!(op, SymCmpOp::Ult | SymCmpOp::Ule | SymCmpOp::Slt) {
                        (op.eval(left.max, right.min), op.eval(left.min, right.max))
                    } else {
                        (op.eval(left.min, right.max), op.eval(left.max, right.min))
                    };
                if always {
                    Some(true)
                } else if !possible {
                    Some(false)
                } else {
                    None
                }
            }
            SymBoolExprKind::And(_) => None,
        }
    }

    /// Simplifies signed addition guards once the signs of the summands are established.
    fn normalize_signed_add_comparison(
        &self,
        cx: &mut SymCx,
        expr: &SymBoolExpr,
    ) -> Option<SymBoolExpr> {
        let SymBoolExprKind::Cmp(op, left, right) = expr.kind() else { return None };
        let (sum, base) = match op {
            SymCmpOp::Slt => (left, right),
            SymCmpOp::Sgt => (right, left),
            _ => return None,
        };
        let signed_max = U256::MAX >> 1;
        if let Some((_, increment)) = sum.add_with_operand(base) {
            let base_range = self.interval(base)?;
            let increment_range = self.interval(increment)?;
            // Opposite-sign addition cannot overflow: adding a nonnegative value cannot make
            // a negative summand smaller, and adding a negative value makes a nonnegative one
            // smaller.
            if base_range.min > signed_max && increment_range.max <= signed_max {
                return Some(SymBoolExpr::constant(cx, false));
            }
            if base_range.max <= signed_max && increment_range.min > signed_max {
                return Some(SymBoolExpr::constant(cx, true));
            }
        } else if base.as_const() != Some(U256::ZERO) {
            return None;
        }
        if base.as_const() == Some(U256::ZERO)
            && let SymExprKind::BinOp(SymBinOp::Add, left, right) = sum.kind()
        {
            for (positive, negative) in [(left, right), (right, left)] {
                if let SymExprKind::BinOp(SymBinOp::Sub, zero, amount) = negative.kind()
                    && zero.as_const() == Some(U256::ZERO)
                    && self.interval(positive).is_some_and(|range| range.max <= signed_max)
                    && self.interval(amount).is_some_and(|range| range.max <= signed_max)
                {
                    // For a,b in [0, int256::MAX], signed(a + (-b)) < 0 iff a < b.
                    return Some(SymBoolExpr::cmp(
                        cx,
                        SymCmpOp::Ult,
                        positive.clone(),
                        amount.clone(),
                    ));
                }
            }
        }
        // Use the sum's canonical operand order for both the overflow guard and a subsequent
        // signed-to-unsigned cast. Their conditions must normalize to the same predicate.
        let SymExprKind::BinOp(SymBinOp::Add, increment, base) = sum.kind() else {
            return None;
        };
        if self.interval(base)?.max > signed_max || self.interval(increment)?.max > signed_max {
            return None;
        }
        // With nonnegative summands their unsigned sum cannot wrap. It is signed-less than a
        // summand exactly when it crosses the signed maximum; equality (zero increment) is safe.
        let limit = SymExpr::constant(cx, signed_max);
        let remaining = SymExpr::binop(cx, SymBinOp::Sub, limit, base.clone());
        Some(SymBoolExpr::cmp(cx, SymCmpOp::Ult, remaining, increment.clone()))
    }

    fn exact_ceil_div_factor<'a>(&self, expr: &'a SymExpr) -> Option<(&'a SymExpr, U256)> {
        let (product, denominator) = Self::ceil_div_product(expr)?;
        let (value, multiplier) = Self::constant_mul_operands(product)?;
        self.max_scaled_product(value, multiplier)?.checked_add(denominator)?;
        let factor = multiplier.checked_div(denominator)?;
        (multiplier % denominator).is_zero().then_some((value, factor))
    }

    /// Recognizes `(product + scale - 1) / scale` without assuming the arithmetic cannot wrap.
    fn ceil_div_product(expr: &SymExpr) -> Option<(&SymExpr, U256)> {
        let (numerator, denominator) = expr.udiv_operands()?;
        let scale = denominator.as_const().filter(|value| !value.is_zero())?;
        if let SymExprKind::BinOp(SymBinOp::Sub, sum, one) = numerator.kind()
            && one.as_const() == Some(U256::ONE)
            && let SymExprKind::BinOp(SymBinOp::Add, product, rounding) = sum.kind()
            && rounding.as_const() == Some(scale)
            && matches!(product.kind(), SymExprKind::BinOp(SymBinOp::Mul, _, _))
        {
            Some((product, scale))
        } else {
            None
        }
    }

    fn exact_scaled_div_factor<'a>(&self, expr: &'a SymExpr) -> Option<(&'a SymExpr, U256)> {
        let (numerator, denominator) = expr.udiv_operands()?;
        let denominator = denominator.as_const().filter(|value| !value.is_zero())?;
        let (value, multiplier) = Self::constant_mul_operands(numerator)?;
        if !(multiplier % denominator).is_zero()
            || self.max_scaled_product(value, multiplier).is_none()
        {
            return None;
        }
        Some((value, multiplier / denominator))
    }

    fn max_scaled_product(&self, value: &SymExpr, multiplier: U256) -> Option<U256> {
        self.interval(value)?.max.checked_mul(multiplier)
    }

    fn constant_mul_operands(expr: &SymExpr) -> Option<(&SymExpr, U256)> {
        let SymExprKind::BinOp(SymBinOp::Mul, left, right) = expr.kind() else {
            return None;
        };
        right
            .as_const()
            .map(|factor| (left, factor))
            .or_else(|| left.as_const().map(|factor| (right, factor)))
    }

    fn is_exact_value_constraint(&self, constraint: &SymBoolExpr) -> bool {
        let SymBoolExprKind::Cmp(SymCmpOp::Eq, left, right) = constraint.kind() else {
            return false;
        };
        const_side_bound(left, right)
            .is_some_and(|(expr, value)| self.exact_value(expr) == Some(value))
    }

    fn masked_eq_self_condition(&self, expr: &SymBoolExpr) -> bool {
        match expr.kind() {
            SymBoolExprKind::Cmp(SymCmpOp::Eq, left, right) => {
                self.masked_word_eq_self(left, right)
            }
            _ => false,
        }
    }

    fn masked_word_eq_self(&self, left: &SymExpr, right: &SymExpr) -> bool {
        self.masked_word_side_eq_self(left, right) || self.masked_word_side_eq_self(right, left)
    }

    fn masked_word_side_eq_self(&self, masked: &SymExpr, value: &SymExpr) -> bool {
        Self::masked_word_side_eq_self_shape(masked, value)
            .is_some_and(|bits| self.unsigned_bits(value) <= bits)
    }

    fn masked_word_side_eq_self_shape(masked: &SymExpr, value: &SymExpr) -> Option<usize> {
        let SymExprKind::BinOp(SymBinOp::And, left, right) = masked.kind() else {
            return None;
        };
        let (source, mask) = right
            .as_const()
            .map(|mask| (left, mask))
            .or_else(|| left.as_const().map(|mask| (right, mask)))?;
        let bits = mask_low_bits(mask)?;
        (source == value).then_some(bits)
    }

    fn record_upper_bound_constraint(&mut self, constraint: &SymBoolExpr) {
        if let Some((expr, bound)) = self.upper_bound_constraint(constraint) {
            self.record_upper_bound(expr.clone(), bound);
        }
    }

    fn record_upper_bound(&mut self, expr: SymExpr, bound: U256) -> bool {
        match self.upper_bounds.entry(expr) {
            alloy_primitives::map::Entry::Occupied(mut entry) if bound < *entry.get() => {
                entry.insert(bound);
                true
            }
            alloy_primitives::map::Entry::Vacant(entry) => {
                entry.insert(bound);
                true
            }
            alloy_primitives::map::Entry::Occupied(_) => false,
        }
    }

    fn record_lower_bound_constraint(&mut self, constraint: &SymBoolExpr) {
        if let Some((expr, bound)) = self.lower_bound_constraint(constraint) {
            self.record_lower_bound(expr.clone(), bound);
        }
    }

    fn record_unsigned_lower_bound_constraint(
        &mut self,
        constraint: &SymBoolExpr,
        promote_to_interval: bool,
    ) {
        if let Some((expr, bound)) = self.unsigned_lower_bound_constraint(constraint) {
            let entry = self.unsigned_lower_bounds.entry(expr.clone()).or_default();
            *entry = (*entry).max(bound);
            // Batch normalization keeps theorem bounds separate from general intervals.
            // A rewrite supported only by other retained conjuncts may also use these bounds
            // for interval deductions.
            if promote_to_interval {
                self.record_lower_bound(expr.clone(), bound);
            }
        }
    }

    fn unsigned_lower_bound_constraint<'a>(
        &self,
        constraint: &'a SymBoolExpr,
    ) -> Option<(&'a SymExpr, U256)> {
        match constraint.kind() {
            SymBoolExprKind::Cmp(op, left, right) => match *op {
                SymCmpOp::Eq => const_side_bound(left, right),
                SymCmpOp::Ult => {
                    left.as_const()?.checked_add(U256::ONE).map(|bound| (right, bound))
                }
                SymCmpOp::Ule => left.as_const().map(|bound| (right, bound)),
                SymCmpOp::Ugt => {
                    right.as_const()?.checked_add(U256::ONE).map(|bound| (left, bound))
                }
                SymCmpOp::Uge => right.as_const().map(|bound| (left, bound)),
                SymCmpOp::Slt | SymCmpOp::Sgt => None,
            },
            SymBoolExprKind::Not(value) => match value.kind() {
                SymBoolExprKind::Cmp(SymCmpOp::Ult, left, right) => {
                    right.as_const().map(|bound| (left, bound))
                }
                SymBoolExprKind::Cmp(SymCmpOp::Ule, left, right) => {
                    right.as_const()?.checked_add(U256::ONE).map(|bound| (left, bound))
                }
                SymBoolExprKind::Cmp(SymCmpOp::Ugt, left, right) => {
                    left.as_const().map(|bound| (right, bound))
                }
                SymBoolExprKind::Cmp(SymCmpOp::Uge, left, right) => {
                    left.as_const()?.checked_add(U256::ONE).map(|bound| (right, bound))
                }
                _ => None,
            },
            _ => None,
        }
    }

    fn record_lower_bound(&mut self, expr: SymExpr, bound: U256) -> bool {
        match self.lower_bounds.entry(expr) {
            alloy_primitives::map::Entry::Occupied(mut entry) if bound > *entry.get() => {
                entry.insert(bound);
                true
            }
            alloy_primitives::map::Entry::Vacant(entry) => {
                entry.insert(bound);
                true
            }
            alloy_primitives::map::Entry::Occupied(_) => false,
        }
    }

    fn propagate_order_bounds(&mut self, constraint: &SymBoolExpr) -> bool {
        match constraint.kind() {
            SymBoolExprKind::Cmp(op, left, right) => match op {
                SymCmpOp::Ult | SymCmpOp::Ule => self.propagate_less_or_equal_bounds(left, right),
                SymCmpOp::Ugt | SymCmpOp::Uge => self.propagate_less_or_equal_bounds(right, left),
                SymCmpOp::Eq => {
                    let changed = self.propagate_less_or_equal_bounds(left, right);
                    self.propagate_less_or_equal_bounds(right, left) || changed
                }
                SymCmpOp::Slt | SymCmpOp::Sgt => false,
            },
            SymBoolExprKind::Not(value) => match value.kind() {
                SymBoolExprKind::Cmp(op, left, right) => match op {
                    SymCmpOp::Ult | SymCmpOp::Ule => {
                        self.propagate_less_or_equal_bounds(right, left)
                    }
                    SymCmpOp::Ugt | SymCmpOp::Uge => {
                        self.propagate_less_or_equal_bounds(left, right)
                    }
                    SymCmpOp::Eq | SymCmpOp::Slt | SymCmpOp::Sgt => false,
                },
                _ => false,
            },
            SymBoolExprKind::Const(_) | SymBoolExprKind::And(_) => false,
        }
    }

    /// Propagates interval bounds through the known unsigned relation `left <= right`.
    fn propagate_less_or_equal_bounds(&mut self, left: &SymExpr, right: &SymExpr) -> bool {
        let upper = self.upper_bound(right);
        let lower = self.lower_bound(left);
        let upper_changed = upper.is_some_and(|bound| self.record_upper_bound(left.clone(), bound));
        let lower_changed =
            lower.is_some_and(|bound| self.record_lower_bound(right.clone(), bound));
        upper_changed || lower_changed
    }

    fn upper_bound_constraint<'a>(
        &self,
        constraint: &'a SymBoolExpr,
    ) -> Option<(&'a SymExpr, U256)> {
        match constraint.kind() {
            SymBoolExprKind::Cmp(op, left, right) => match *op {
                SymCmpOp::Eq => const_side_bound(left, right),
                SymCmpOp::Ult => match (left.as_const(), right.as_const()) {
                    (_, Some(bound)) => (!bound.is_zero()).then(|| (left, bound - U256::from(1))),
                    _ => None,
                },
                SymCmpOp::Ule => match (left.as_const(), right.as_const()) {
                    (_, Some(bound)) => Some((left, bound)),
                    _ => None,
                },
                SymCmpOp::Ugt => match (left.as_const(), right.as_const()) {
                    (Some(bound), _) => (!bound.is_zero()).then(|| (right, bound - U256::from(1))),
                    _ => None,
                },
                SymCmpOp::Uge => match (left.as_const(), right.as_const()) {
                    (Some(bound), _) => Some((right, bound)),
                    _ => None,
                },
                SymCmpOp::Slt | SymCmpOp::Sgt => None,
            },
            SymBoolExprKind::Not(value) => match value.kind() {
                SymBoolExprKind::Cmp(op, left, right) => match *op {
                    SymCmpOp::Ugt => match (left.as_const(), right.as_const()) {
                        (_, Some(bound)) => Some((left, bound)),
                        _ => None,
                    },
                    SymCmpOp::Uge => match (left.as_const(), right.as_const()) {
                        (_, Some(bound)) => {
                            (!bound.is_zero()).then(|| (left, bound - U256::from(1)))
                        }
                        _ => None,
                    },
                    SymCmpOp::Ult => match (left.as_const(), right.as_const()) {
                        (Some(bound), _) => Some((right, bound)),
                        _ => None,
                    },
                    SymCmpOp::Ule => match (left.as_const(), right.as_const()) {
                        (Some(bound), _) => {
                            (!bound.is_zero()).then(|| (right, bound - U256::from(1)))
                        }
                        _ => None,
                    },
                    SymCmpOp::Eq | SymCmpOp::Slt | SymCmpOp::Sgt => None,
                },
                _ => None,
            },
            SymBoolExprKind::Const(_) | SymBoolExprKind::And(_) => None,
        }
    }

    fn lower_bound_constraint<'a>(
        &self,
        constraint: &'a SymBoolExpr,
    ) -> Option<(&'a SymExpr, U256)> {
        match constraint.kind() {
            SymBoolExprKind::Cmp(SymCmpOp::Eq, left, right) => const_side_bound(left, right),
            SymBoolExprKind::Not(value) => match value.kind() {
                SymBoolExprKind::Cmp(SymCmpOp::Eq, left, right) => {
                    if right.as_const().is_some_and(|value| value.is_zero()) {
                        Some((left, U256::from(1)))
                    } else if left.as_const().is_some_and(|value| value.is_zero()) {
                        Some((right, U256::from(1)))
                    } else {
                        None
                    }
                }
                _ => None,
            },
            _ => None,
        }
    }

    fn unsigned_bool_always_true(&self, expr: &SymBoolExpr) -> bool {
        match expr.kind() {
            SymBoolExprKind::Cmp(op, left, right) => {
                self.unsigned_cmp_always_true(*op, left, right)
            }
            _ => false,
        }
    }

    fn unsigned_cmp_always_true(&self, op: SymCmpOp, left: &SymExpr, right: &SymExpr) -> bool {
        if op == SymCmpOp::Eq
            && (self.mul_div_identity(left, right) || self.mul_div_identity(right, left))
        {
            return true;
        }
        let Some(left) = self.interval(left) else {
            return false;
        };
        let Some(right) = self.interval(right) else {
            return false;
        };
        match op {
            SymCmpOp::Ult => left.max < right.min,
            SymCmpOp::Ule => left.max <= right.min,
            SymCmpOp::Ugt => left.min > right.max,
            SymCmpOp::Uge => left.min >= right.max,
            SymCmpOp::Eq | SymCmpOp::Slt | SymCmpOp::Sgt => false,
        }
    }

    fn mul_div_identity(&self, quotient: &SymExpr, expected: &SymExpr) -> bool {
        let Some((denominator, other)) = Self::mul_div_identity_operands(quotient, expected) else {
            return false;
        };

        self.interval(denominator).is_some_and(|interval| !interval.min.is_zero())
            && self.mul_cannot_overflow_256(denominator, other)
    }

    fn mul_div_identity_operands<'a>(
        quotient: &'a SymExpr,
        expected: &SymExpr,
    ) -> Option<(&'a SymExpr, &'a SymExpr)> {
        let (denominator, other) = Self::mul_div_operands(quotient)?;
        (other == expected).then_some((denominator, other))
    }

    fn mul_div_operands(quotient: &SymExpr) -> Option<(&SymExpr, &SymExpr)> {
        let (numerator, denominator) = quotient.udiv_operands()?;
        let SymExprKind::BinOp(SymBinOp::Mul, left, right) = numerator.kind() else {
            return None;
        };
        let other = if left == denominator {
            right
        } else if right == denominator {
            left
        } else {
            return None;
        };
        Some((denominator, other))
    }

    fn udiv_comparison_operands<'a>(
        op: SymCmpOp,
        left: &'a SymExpr,
        right: &'a SymExpr,
    ) -> Option<(&'a SymExpr, &'a SymExpr, &'a SymExpr, bool)> {
        if !matches!(op, SymCmpOp::Ult | SymCmpOp::Ule) {
            return None;
        }
        if let Some((numerator, denominator)) = left.udiv_operands()
            && denominator.as_const().is_some_and(|value| !value.is_zero())
            && !right.contains_udiv()
        {
            return Some((numerator, denominator, right, true));
        }
        if let Some((numerator, denominator)) = right.udiv_operands()
            && denominator.as_const().is_some_and(|value| !value.is_zero())
            && !left.contains_udiv()
        {
            return Some((numerator, denominator, left, false));
        }
        None
    }

    fn normalize_udiv_comparison(
        &self,
        cx: &mut SymCx,
        op: SymCmpOp,
        left: &SymExpr,
        right: &SymExpr,
    ) -> Option<SymBoolExpr> {
        let (numerator, denominator, threshold, quotient_on_left) =
            Self::udiv_comparison_operands(op, left, right)?;
        let increment_threshold =
            matches!((op, quotient_on_left), (SymCmpOp::Ule, true) | (SymCmpOp::Ult, false));
        let threshold = if increment_threshold {
            // Prove the successor cannot wrap before constructing the word addition.
            self.interval(threshold)?.max.checked_add(U256::ONE)?;
            let one = SymExpr::one(cx);
            SymExpr::binop(cx, SymBinOp::Add, threshold.clone(), one)
        } else {
            threshold.clone()
        };
        if !self.mul_cannot_overflow_256(&threshold, denominator) {
            return None;
        }

        let scaled_threshold = SymExpr::binop(cx, SymBinOp::Mul, threshold, denominator.clone());
        Some(if quotient_on_left {
            // `n / d < k => n < k * d`; `n / d <= k => n < (k + 1) * d`.
            SymBoolExpr::cmp(cx, SymCmpOp::Ult, numerator.clone(), scaled_threshold)
        } else {
            // `k <= n / d => k * d <= n`; `k < n / d => (k + 1) * d <= n`.
            SymBoolExpr::cmp(cx, SymCmpOp::Ule, scaled_threshold, numerator.clone())
        })
    }

    fn interval(&self, expr: &SymExpr) -> Option<WordInterval> {
        let mut intervals = HashMap::default();
        let mut remaining = MAX_LOCAL_ANALYSIS_NODES;
        self.interval_cached(expr, &mut intervals, &mut remaining)
    }

    fn interval_cached(
        &self,
        expr: &SymExpr,
        intervals: &mut HashMap<SymExpr, Option<WordInterval>>,
        remaining: &mut usize,
    ) -> Option<WordInterval> {
        if let Some(interval) = intervals.get(expr) {
            return *interval;
        }

        let lower = self.lower_bound(expr);
        let upper = self.upper_bound(expr);
        let explicit_bounds = || {
            if lower.is_none() && upper.is_none() {
                return None;
            }
            WordInterval::new(lower.unwrap_or(U256::ZERO), upper.unwrap_or(U256::MAX))
        };
        if *remaining == 0 {
            let interval = explicit_bounds();
            intervals.insert(expr.clone(), interval);
            return interval;
        }
        *remaining -= 1;

        let interval =
            self.structural_interval(expr, intervals, remaining).or_else(explicit_bounds);
        let interval = interval.and_then(|interval| interval.with_bounds(lower, upper));
        intervals.insert(expr.clone(), interval);
        interval
    }

    fn structural_interval(
        &self,
        expr: &SymExpr,
        intervals: &mut HashMap<SymExpr, Option<WordInterval>>,
        remaining: &mut usize,
    ) -> Option<WordInterval> {
        match expr.kind() {
            SymExprKind::Const(value) => Some(WordInterval::exact(*value)),
            SymExprKind::BinOp(SymBinOp::And, left, right) => {
                let mask = left.as_const().or_else(|| right.as_const())?;
                Some(WordInterval { min: U256::ZERO, max: mask })
            }
            SymExprKind::BinOp(SymBinOp::Add, left, right) => {
                let left = self.interval_cached(left, intervals, remaining)?;
                let right = self.interval_cached(right, intervals, remaining)?;
                Some(WordInterval {
                    min: left.min.checked_add(right.min)?,
                    max: left.max.checked_add(right.max)?,
                })
            }
            SymExprKind::BinOp(SymBinOp::Sub, left, right) => {
                if let Some(interval) =
                    self.rounding_error_interval(left, right, intervals, remaining)
                {
                    return Some(interval);
                }
                let left = self.interval_cached(left, intervals, remaining)?;
                let right = self.interval_cached(right, intervals, remaining)?;
                if left.max < right.min {
                    // Every subtraction wraps exactly once, so the unsigned image is contiguous.
                    return Some(WordInterval {
                        min: left.min.wrapping_sub(right.max),
                        max: left.max.wrapping_sub(right.min),
                    });
                }
                if left.min < right.max {
                    return None;
                }
                Some(WordInterval {
                    min: left.min.checked_sub(right.max)?,
                    max: left.max.checked_sub(right.min)?,
                })
            }
            SymExprKind::BinOp(SymBinOp::Mul, left, right) => {
                let guarded = self.has_non_wrapping_product(left, right);
                let left = self.interval_cached(left, intervals, remaining)?;
                let right = self.interval_cached(right, intervals, remaining)?;
                Some(WordInterval {
                    min: left.min.checked_mul(right.min)?,
                    // A retained overflow guard correlates the factors. Their independent
                    // maxima can overflow even though every feasible product fits.
                    max: left
                        .max
                        .checked_mul(right.max)
                        .or_else(|| guarded.then_some(U256::MAX))?,
                })
            }
            SymExprKind::BinOp(SymBinOp::UDiv, numerator, denominator) => {
                // Even an otherwise unbounded numerator is a uint256 word. Division by a
                // positive denominator bounds the quotient regardless of numerator wrapping.
                let numerator = self
                    .interval_cached(numerator, intervals, remaining)
                    .unwrap_or(WordInterval { min: U256::ZERO, max: U256::MAX });
                let denominator = self.interval_cached(denominator, intervals, remaining)?;
                if denominator.min.is_zero() {
                    return None;
                }
                Some(WordInterval {
                    min: numerator.min / denominator.max,
                    max: numerator.max / denominator.min,
                })
            }
            SymExprKind::BinOp(SymBinOp::Shr, value, shift) => {
                let shift = shift.as_const()?;
                if shift >= U256::from(256) {
                    return Some(WordInterval::exact(U256::ZERO));
                }
                let value = self.interval_cached(value, intervals, remaining)?;
                let shift = shift.to::<usize>();
                Some(WordInterval { min: value.min >> shift, max: value.max >> shift })
            }
            SymExprKind::Ite(_, left, right) => {
                let left = self.interval_cached(left, intervals, remaining)?;
                let right = self.interval_cached(right, intervals, remaining)?;
                Some(WordInterval { min: left.min.min(right.min), max: left.max.max(right.max) })
            }
            _ => None,
        }
    }
}

fn const_side_bound<'a>(left: &'a SymExpr, right: &'a SymExpr) -> Option<(&'a SymExpr, U256)> {
    right
        .as_const()
        .map(|value| (left, value))
        .or_else(|| left.as_const().map(|value| (right, value)))
}

/// Normalizes one word expression into an equivalent, solver-friendlier form.
pub(crate) fn normalize_expr_for_solver(cx: &mut SymCx, expr: SymExpr) -> SymExpr {
    if expr.contains_ite() { expr.fold(cx, &mut normalize_expr_node_for_solver) } else { expr }
}

fn normalize_expr_node_for_solver(cx: &mut SymCx, expr: SymExpr) -> SymExpr {
    match expr.kind() {
        SymExprKind::Ite(cond, left, right) => {
            normalize_ite_expr_for_solver(cx, cond.clone(), left.clone(), right.clone())
        }
        _ => expr,
    }
}

fn normalize_ite_expr_for_solver(
    cx: &mut SymCx,
    cond: SymBoolExpr,
    left: SymExpr,
    right: SymExpr,
) -> SymExpr {
    let cond = normalize_bool_for_solver(cx, cond);
    if left == right {
        // `ite(c, a, a) => a`.
        return left;
    }
    if left.as_const() == Some(U256::from(1))
        && right.normalized_bool_word_condition(cx).as_ref() == Some(&cond)
    {
        // `ite(c, 1, bool_word(c)) => bool_word(c)`.
        return right;
    }
    if right.as_const().is_some_and(|value| value.is_zero())
        && left.normalized_bool_word_condition(cx).as_ref() == Some(&cond)
    {
        // `ite(c, bool_word(c), 0) => bool_word(c)`.
        return left;
    }
    SymExpr::ite(cx, cond, left, right)
}

impl SymExpr {
    fn add_cannot_overflow_256(&self, right: &Self) -> bool {
        self.unsigned_bits().max(right.unsigned_bits()).saturating_add(1) <= 256
    }

    fn word_bool_always_true(&self, cx: &mut SymCx) -> bool {
        ConstraintContext::default().word_bool_always_true(cx, self)
    }
}

impl SymBoolExpr {
    fn normalize_udiv_for_solver(&self, cx: &mut SymCx) -> Option<Self> {
        if let SymBoolExprKind::Cmp(op, left, right) = self.kind()
            && let Some(normalized) = Self::normalize_const_over_self_udiv_cmp(cx, *op, left, right)
        {
            return Some(normalized);
        }
        if let SymBoolExprKind::Not(value) = self.kind()
            && let SymBoolExprKind::Cmp(op, left, right) = value.kind()
            && let Some(normalized) = Self::normalize_const_over_self_udiv_cmp(cx, *op, left, right)
        {
            return Some(normalized.not(cx));
        }

        match self.kind() {
            SymBoolExprKind::Cmp(SymCmpOp::Eq, left, right)
                if right.as_const().is_some_and(|value| value.is_zero()) =>
            {
                left.normalized_bool_word_condition(cx).map(|value| value.not(cx)).or_else(|| {
                    if left.word_bool_always_true(cx) {
                        // `always_true_word == 0 => false`.
                        Some(Self::constant(cx, false))
                    } else {
                        let zero = SymExpr::zero(cx);
                        Self::normalize_udiv_eq_zero(cx, left, &zero)
                    }
                })
            }
            SymBoolExprKind::Cmp(SymCmpOp::Eq, left, right)
                if right.as_const() == Some(U256::from(1)) =>
            {
                // `bool_word(c) == 1 => c`.
                left.normalized_bool_word_condition(cx)
            }
            SymBoolExprKind::Not(value) => match value.kind() {
                SymBoolExprKind::Cmp(SymCmpOp::Eq, left, right)
                    if right.as_const().is_some_and(|value| value.is_zero()) =>
                {
                    if left.word_bool_always_true(cx) {
                        // `always_true_word != 0 => true`.
                        Some(Self::constant(cx, true))
                    } else {
                        let zero = SymExpr::zero(cx);
                        Self::normalize_udiv_eq_zero(cx, left, &zero).map(|value| value.not(cx))
                    }
                }
                SymBoolExprKind::Cmp(SymCmpOp::Eq, left, right) => {
                    Self::normalize_udiv_eq_zero(cx, left, right).map(|value| value.not(cx))
                }
                SymBoolExprKind::Cmp(op, left, right) => {
                    Self::normalize_add_overflow_cmp(cx, *op, left, right)
                        .map(|value| value.not(cx))
                        .or_else(|| {
                            Self::normalize_udiv_cmp(cx, *op, left, right)
                                .map(|value| value.not(cx))
                        })
                }
                _ => None,
            },
            SymBoolExprKind::Cmp(SymCmpOp::Eq, left, right) => {
                Self::normalize_udiv_eq_zero(cx, left, right)
            }
            SymBoolExprKind::Cmp(op, left, right) => {
                Self::normalize_add_overflow_cmp(cx, *op, left, right)
                    .or_else(|| Self::normalize_udiv_cmp(cx, *op, left, right))
            }
            SymBoolExprKind::Const(_) | SymBoolExprKind::And(_) => None,
        }
    }

    fn normalize_add_overflow_cmp(
        cx: &mut SymCx,
        op: SymCmpOp,
        left: &SymExpr,
        right: &SymExpr,
    ) -> Option<Self> {
        if let Some(normalized) = Self::normalize_sub_underflow_cmp(cx, op, left, right) {
            return Some(normalized);
        }
        // Strict forms test overflow and non-strict forms its complement; addition wraps iff the
        // increment exceeds `~base`.
        let (base, increment, overflow) = match op {
            SymCmpOp::Ugt => {
                right.add_with_operand(left).map(|(_, increment)| (left, increment, true))
            }
            SymCmpOp::Ult => {
                left.add_with_operand(right).map(|(_, increment)| (right, increment, true))
            }
            SymCmpOp::Uge => {
                left.add_with_operand(right).map(|(_, increment)| (right, increment, false))
            }
            SymCmpOp::Ule => {
                right.add_with_operand(left).map(|(_, increment)| (left, increment, false))
            }
            SymCmpOp::Eq | SymCmpOp::Slt | SymCmpOp::Sgt => None,
        }?;
        if base.add_cannot_overflow_256(increment) {
            return Some(Self::constant(cx, !overflow));
        }

        let limit = match base.kind() {
            SymExprKind::BinOp(SymBinOp::Sub, max, value) if max.as_const() == Some(U256::MAX) => {
                value.clone()
            }
            _ => SymExpr::not(cx, base.clone()),
        };
        Some(if overflow {
            Self::cmp(cx, SymCmpOp::Ult, limit, increment.clone())
        } else {
            Self::cmp(cx, SymCmpOp::Ule, increment.clone(), limit)
        })
    }

    fn normalize_sub_underflow_cmp(
        cx: &mut SymCx,
        op: SymCmpOp,
        left: &SymExpr,
        right: &SymExpr,
    ) -> Option<Self> {
        let (base, difference, underflow) = match op {
            SymCmpOp::Ult => (left, right, true),
            SymCmpOp::Ugt => (right, left, true),
            SymCmpOp::Ule => (right, left, false),
            SymCmpOp::Uge => (left, right, false),
            _ => return None,
        };
        let SymExprKind::BinOp(SymBinOp::Sub, minuend, subtrahend) = difference.kind() else {
            return None;
        };
        if minuend != base {
            return None;
        }
        // Unsigned modular subtraction wraps exactly when the subtrahend exceeds the minuend.
        Some(if underflow {
            Self::cmp(cx, SymCmpOp::Ult, base.clone(), subtrahend.clone())
        } else {
            Self::cmp(cx, SymCmpOp::Ule, subtrahend.clone(), base.clone())
        })
    }

    fn normalize_udiv_eq_zero(cx: &mut SymCx, left: &SymExpr, right: &SymExpr) -> Option<Self> {
        if right.as_const().is_some_and(|value| value.is_zero())
            && let Some(condition) = left.normalize_eq_zero_for_solver(cx)
        {
            // `word_bool(c) == 0 => !c`.
            return Some(condition);
        }
        None
    }

    fn normalize_udiv_cmp(
        cx: &mut SymCx,
        op: SymCmpOp,
        left: &SymExpr,
        right: &SymExpr,
    ) -> Option<Self> {
        match op {
            SymCmpOp::Ugt => match (left.as_const(), right.as_const()) {
                // `a > 0 => a != 0`.
                (_, Some(value)) if value.is_zero() => left
                    .normalize_ne_zero_for_solver(cx)
                    .or_else(|| Some(Self::eq_zero(cx, left).not(cx))),
                // `1 > a => a == 0`.
                (Some(value), _) if value == U256::from(1) => right
                    .normalize_eq_zero_for_solver(cx)
                    .or_else(|| Some(Self::eq_zero(cx, right))),
                _ => None,
            },
            SymCmpOp::Uge => match (left.as_const(), right.as_const()) {
                // `a >= 1 => a != 0`.
                (_, Some(value)) if value == U256::from(1) => left
                    .normalize_ne_zero_for_solver(cx)
                    .or_else(|| Some(Self::eq_zero(cx, left).not(cx))),
                // `0 >= a => a == 0`.
                (Some(value), _) if value.is_zero() => right
                    .normalize_eq_zero_for_solver(cx)
                    .or_else(|| Some(Self::eq_zero(cx, right))),
                _ => None,
            },
            SymCmpOp::Ule => match (left.as_const(), right.as_const()) {
                // `a <= 0 => a == 0`.
                (_, Some(value)) if value.is_zero() => {
                    left.normalize_eq_zero_for_solver(cx).or_else(|| Some(Self::eq_zero(cx, left)))
                }
                // `1 <= a => a != 0`.
                (Some(value), _) if value == U256::from(1) => right
                    .normalize_ne_zero_for_solver(cx)
                    .or_else(|| Some(Self::eq_zero(cx, right).not(cx))),
                _ => None,
            },
            SymCmpOp::Ult => match (left.as_const(), right.as_const()) {
                // `a < 1 => a == 0`.
                (_, Some(value)) if value == U256::from(1) => {
                    left.normalize_eq_zero_for_solver(cx).or_else(|| Some(Self::eq_zero(cx, left)))
                }
                // `0 < a => a != 0`.
                (Some(value), _) if value.is_zero() => right
                    .normalize_ne_zero_for_solver(cx)
                    .or_else(|| Some(Self::eq_zero(cx, right).not(cx))),
                _ => None,
            },
            SymCmpOp::Eq | SymCmpOp::Slt | SymCmpOp::Sgt => None,
        }
    }

    fn normalize_const_over_self_udiv_cmp(
        cx: &mut SymCx,
        op: SymCmpOp,
        left: &SymExpr,
        right: &SymExpr,
    ) -> Option<Self> {
        let (value, quotient, complement) = match op {
            // `a <= c / a`.
            SymCmpOp::Ule => (left, right, false),
            // `c / a < a`, the complement of `a <= c / a`.
            SymCmpOp::Ult => (right, left, true),
            SymCmpOp::Eq | SymCmpOp::Ugt | SymCmpOp::Uge | SymCmpOp::Slt | SymCmpOp::Sgt => {
                return None;
            }
        };
        let (numerator, denominator) = match quotient.kind() {
            SymExprKind::BinOp(SymBinOp::UDiv, numerator, denominator) => (numerator, denominator),
            SymExprKind::Ite(condition, zero, division)
                if zero.as_const().is_some_and(|value| value.is_zero()) =>
            {
                let (numerator, denominator) = division.udiv_operands()?;
                if condition.zero_check_operand() != Some(denominator) {
                    return None;
                }
                (numerator, denominator)
            }
            _ => return None,
        };
        if denominator != value {
            return None;
        }

        let threshold = numerator.as_const()?.root(2);
        let threshold = SymExpr::constant(cx, threshold);
        Some(if complement {
            Self::cmp(cx, SymCmpOp::Ult, threshold, value.clone())
        } else {
            Self::cmp(cx, SymCmpOp::Ule, value.clone(), threshold)
        })
    }

    fn eq_zero(cx: &mut SymCx, expr: &SymExpr) -> Self {
        let zero = SymExpr::zero(cx);
        Self::eq(cx, expr.clone(), zero)
    }
}

impl SymExpr {
    fn normalized_bool_word_condition(&self, cx: &mut SymCx) -> Option<SymBoolExpr> {
        self.strip_low_byte_mask()
            .bool_word_condition()
            .map(|condition| normalize_bool_for_solver(cx, condition))
    }

    fn add_with_operand<'a>(&'a self, operand: &Self) -> Option<(&'a Self, &'a Self)> {
        let SymExprKind::BinOp(SymBinOp::Add, left, right) = self.kind() else {
            return None;
        };
        if left == operand {
            Some((left, right))
        } else if right == operand {
            Some((right, left))
        } else {
            None
        }
    }

    fn normalize_eq_zero_for_solver(&self, cx: &mut SymCx) -> Option<SymBoolExpr> {
        if let Some((numerator, denominator)) = self.udiv_operands() {
            // `a / b == 0 => b == 0 || a < b`.
            return Some(Self::udiv_zero_condition(cx, numerator, denominator));
        }
        if let SymExprKind::Ite(condition, then_expr, else_expr) = self.kind() {
            let then_zero = match then_expr.normalize_eq_zero_for_solver(cx) {
                Some(condition) => condition,
                None => {
                    let then_expr = normalize_expr_for_solver(cx, then_expr.clone());
                    let zero = Self::zero(cx);
                    SymBoolExpr::eq(cx, then_expr, zero)
                }
            };
            let else_zero = match else_expr.normalize_eq_zero_for_solver(cx) {
                Some(condition) => condition,
                None => {
                    let else_expr = normalize_expr_for_solver(cx, else_expr.clone());
                    let zero = Self::zero(cx);
                    SymBoolExpr::eq(cx, else_expr, zero)
                }
            };
            if then_zero.contains_udiv() || else_zero.contains_udiv() {
                return None;
            }
            // `ite(c, a, b) == 0 => (c && a == 0) || (!c && b == 0)`.
            let condition = normalize_bool_for_solver(cx, condition.clone());
            let then_condition = SymBoolExpr::and(cx, vec![condition.clone(), then_zero]);
            let not_condition = condition.not(cx);
            let else_condition = SymBoolExpr::and(cx, vec![not_condition, else_zero]);
            return Some(SymBoolExpr::or(cx, vec![then_condition, else_condition]));
        }
        None
    }

    fn normalize_ne_zero_for_solver(&self, cx: &mut SymCx) -> Option<SymBoolExpr> {
        if let Some((numerator, denominator)) = self.udiv_operands() {
            // `a / b != 0 => b != 0 && a >= b`.
            return Some(Self::udiv_nonzero_condition(cx, numerator, denominator));
        }
        if let SymExprKind::Ite(condition, then_expr, else_expr) = self.kind() {
            let then_nonzero = match then_expr.normalize_ne_zero_for_solver(cx) {
                Some(condition) => condition,
                None => {
                    let then_expr = normalize_expr_for_solver(cx, then_expr.clone());
                    let zero = Self::zero(cx);
                    SymBoolExpr::eq(cx, then_expr, zero).not(cx)
                }
            };
            let else_nonzero = match else_expr.normalize_ne_zero_for_solver(cx) {
                Some(condition) => condition,
                None => {
                    let else_expr = normalize_expr_for_solver(cx, else_expr.clone());
                    let zero = Self::zero(cx);
                    SymBoolExpr::eq(cx, else_expr, zero).not(cx)
                }
            };
            if then_nonzero.contains_udiv() || else_nonzero.contains_udiv() {
                return None;
            }
            // `ite(c, a, b) != 0 => (c && a != 0) || (!c && b != 0)`.
            let condition = normalize_bool_for_solver(cx, condition.clone());
            let then_condition = SymBoolExpr::and(cx, vec![condition.clone(), then_nonzero]);
            let not_condition = condition.not(cx);
            let else_condition = SymBoolExpr::and(cx, vec![not_condition, else_nonzero]);
            return Some(SymBoolExpr::or(cx, vec![then_condition, else_condition]));
        }
        None
    }

    fn udiv_zero_condition(cx: &mut SymCx, numerator: &Self, denominator: &Self) -> SymBoolExpr {
        let numerator = normalize_expr_for_solver(cx, numerator.clone());
        let denominator = normalize_expr_for_solver(cx, denominator.clone());
        let zero = Self::zero(cx);
        let denominator_zero = SymBoolExpr::eq(cx, denominator.clone(), zero);
        let below_denominator = SymBoolExpr::cmp(cx, SymCmpOp::Ult, numerator, denominator);
        SymBoolExpr::or(cx, vec![denominator_zero, below_denominator])
    }

    fn udiv_nonzero_condition(cx: &mut SymCx, numerator: &Self, denominator: &Self) -> SymBoolExpr {
        let numerator = normalize_expr_for_solver(cx, numerator.clone());
        let denominator = normalize_expr_for_solver(cx, denominator.clone());
        let zero = Self::zero(cx);
        let denominator_nonzero = SymBoolExpr::eq(cx, denominator.clone(), zero).not(cx);
        let at_least_denominator = SymBoolExpr::cmp(cx, SymCmpOp::Uge, numerator, denominator);
        SymBoolExpr::and(cx, vec![denominator_nonzero, at_least_denominator])
    }
}

impl ConstraintContext {
    fn word_bool_always_true(&self, cx: &mut SymCx, expr: &SymExpr) -> bool {
        let mut terms = Vec::new();
        expr.push_or_terms(&mut terms);
        if terms.len() <= 1 {
            return false;
        }

        let bool_terms = terms
            .iter()
            .filter_map(|term| term.normalized_bool_word_condition(cx))
            .collect::<Vec<_>>();
        if bool_terms.iter().any(|term| {
            let negated = term.clone().not(cx);
            bool_terms.contains(&negated)
        }) {
            // `c || !c => true`.
            return true;
        }
        for zero_term in &bool_terms {
            if bool_terms
                .iter()
                .any(|term| self.checked_mul_guard_for_zero_condition(term, zero_term))
            {
                // `a == 0 || guarded_mul_div(a) => true`.
                return true;
            }
        }
        false
    }

    /// Records operand facts from independently justified, non-wrapping scaled zero checks.
    fn record_scaled_zero_fact(&mut self, cx: &mut SymCx, constraint: &SymBoolExpr) {
        let (condition, nonzero) = match constraint.kind() {
            SymBoolExprKind::Not(inner) => (inner, true),
            _ => (constraint, false),
        };
        if matches!(condition.kind(), SymBoolExprKind::Cmp(SymCmpOp::Ult, _, _))
            && let Some(value) = self.bounded_zero_check_operand(condition).cloned()
        {
            if nonzero {
                self.record_lower_bound(value, U256::ONE);
            } else {
                self.record_upper_bound(value.clone(), U256::ZERO);
                let zero = SymExpr::zero(cx);
                let exact = SymBoolExpr::eq(cx, value, zero);
                self.record_exact_value_constraint(&exact);
            }
        }
    }

    /// Recognizes zero checks exposed by normalizing a scaled balance division.
    fn bounded_zero_check_operand<'a>(&self, expr: &'a SymBoolExpr) -> Option<&'a SymExpr> {
        if let Some(value) = expr.zero_check_operand() {
            return Some(value);
        }
        let SymBoolExprKind::Cmp(SymCmpOp::Ult, product, limit) = expr.kind() else {
            return None;
        };
        let (value, scale) = Self::constant_mul_operands(product)?;
        // x * scale < scale iff x == 0, but only for a positive scale and no wrap.
        if scale.is_zero() || limit.as_const() != Some(scale) {
            return None;
        }
        self.max_scaled_product(value, scale)?;
        Some(value)
    }

    /// Checks a zero predicate against the actual divisor of a multiplication guard.
    fn zero_check_for_operand(&self, condition: &SymBoolExpr, operand: &SymExpr) -> bool {
        if self.bounded_zero_check_operand(condition) == Some(operand) {
            return true;
        }
        // For a positive constant d, n / d == 0 iff n < d. Normalization
        // exposes this comparison before the Solidity multiplication guard.
        if let Some((numerator, denominator)) = operand.udiv_operands()
            && denominator.as_const().is_some_and(|d| !d.is_zero())
            && let SymBoolExprKind::Cmp(SymCmpOp::Ult, left, right) = condition.kind()
        {
            return left == numerator && right == denominator;
        }
        false
    }

    fn checked_mul_guard_for_zero_condition(
        &self,
        expr: &SymBoolExpr,
        zero_condition: &SymBoolExpr,
    ) -> bool {
        let SymBoolExprKind::Cmp(SymCmpOp::Eq, left, right) = expr.kind() else {
            return false;
        };
        [(left, right), (right, left)].into_iter().any(|(quotient, expected)| {
            matches!(quotient.kind(), SymExprKind::Ite(_, _, _))
                && self
                    .checked_quotient_factors(quotient, expected, Some(zero_condition))
                    .is_some_and(|(left, right)| self.mul_cannot_overflow_256(left, right))
        })
    }

    /// Matches the quotient side shared by multiplication guard proofs and retained guard facts.
    fn checked_quotient_factors<'a>(
        &self,
        quotient: &'a SymExpr,
        expected: &SymExpr,
        zero_condition: Option<&SymBoolExpr>,
    ) -> Option<(&'a SymExpr, &'a SymExpr)> {
        let (quotient, branch_condition) =
            if let SymExprKind::Ite(condition, zero, quotient) = quotient.kind() {
                if zero.as_const() != Some(U256::ZERO) || zero_condition.is_none() {
                    return None;
                }
                (quotient, Some(condition))
            } else {
                (quotient, None)
            };
        let (divisor, other) = Self::mul_div_identity_operands(quotient, expected)?;
        (zero_condition.is_none_or(|condition| self.zero_check_for_operand(condition, divisor))
            && branch_condition
                .is_none_or(|condition| self.zero_check_for_operand(condition, divisor)))
        .then_some((divisor, other))
    }

    /// Learns multiplication safety from a retained successful Solidity overflow check.
    fn record_non_wrapping_product(&mut self, cx: &mut SymCx, constraint: &SymBoolExpr) -> bool {
        let fact = bitwise_bool_word_fact(cx, constraint).unwrap_or_else(|| constraint.clone());
        let factors = if let Some(factors) = self.checked_product_factors(&fact, None) {
            Some(factors)
        } else if let SymBoolExprKind::Not(inner) = fact.kind()
            && let SymBoolExprKind::And(terms) = inner.kind()
            && terms.len() == 2
        {
            // Only the exact two-way disjunction is a multiplication guard. An additional
            // alternative would let this predicate hold even when the product wraps.
            let first = terms[0].clone().not(cx);
            let second = terms[1].clone().not(cx);
            self.checked_product_factors(&second, Some(&first))
                .or_else(|| self.checked_product_factors(&first, Some(&second)))
        } else {
            None
        };
        if let Some((left, right)) = factors {
            // A successful product fits in one word. A positive lower bound on either
            // factor therefore bounds the other, even if it started as a full-width word.
            // The supporting guard remains in the conjunction; it cannot prove itself.
            let mut changed = false;
            for (value, factor) in [(&left, &right), (&right, &left)] {
                if let Some(range) = self.interval(factor)
                    && !range.min.is_zero()
                {
                    changed |= self.record_upper_bound(value.clone(), U256::MAX / range.min);
                }
            }
            self.non_wrapping_products.insert((left, right)) || changed
        } else {
            false
        }
    }

    fn checked_product_factors(
        &self,
        predicate: &SymBoolExpr,
        zero_condition: Option<&SymBoolExpr>,
    ) -> Option<(SymExpr, SymExpr)> {
        let SymBoolExprKind::Cmp(SymCmpOp::Eq, left, right) = predicate.kind() else {
            return None;
        };
        for (quotient, expected) in [(left, right), (right, left)] {
            if let Some((divisor, other)) =
                self.checked_quotient_factors(quotient, expected, zero_condition)
            {
                // For divisor > 0, (divisor * other mod 2^256) / divisor == other
                // implies the true product fits. For divisor == 0 the product is zero.
                return Some((divisor.clone(), other.clone()));
            }
        }
        None
    }

    fn has_non_wrapping_product(&self, left: &SymExpr, right: &SymExpr) -> bool {
        self.non_wrapping_products.contains(&(left.clone(), right.clone()))
            || self.non_wrapping_products.contains(&(right.clone(), left.clone()))
    }

    pub(super) fn mul_cannot_overflow_256(&self, left: &SymExpr, right: &SymExpr) -> bool {
        if self.has_non_wrapping_product(left, right) {
            return true;
        }
        let mut intervals = HashMap::default();
        let mut remaining = MAX_LOCAL_ANALYSIS_NODES;
        if self
            .interval_cached(left, &mut intervals, &mut remaining)
            .zip(self.interval_cached(right, &mut intervals, &mut remaining))
            .is_some_and(|(left, right)| left.max.checked_mul(right.max).is_some())
        {
            return true;
        }

        let mut bit_widths = HashMap::default();
        let mut remaining = MAX_LOCAL_ANALYSIS_NODES;
        self.unsigned_bits_cached(left, &mut bit_widths, &mut remaining)
            .zip(self.unsigned_bits_cached(right, &mut bit_widths, &mut remaining))
            .is_some_and(|(left, right)| left.saturating_add(right) <= 256)
    }

    pub(super) fn unsigned_bits(&self, expr: &SymExpr) -> usize {
        let mut bit_widths = HashMap::default();
        let mut remaining = MAX_LOCAL_ANALYSIS_NODES;
        self.unsigned_bits_cached(expr, &mut bit_widths, &mut remaining).unwrap_or(256)
    }

    fn unsigned_bits_cached(
        &self,
        expr: &SymExpr,
        bit_widths: &mut HashMap<SymExpr, usize>,
        remaining: &mut usize,
    ) -> Option<usize> {
        if let Some(bits) = bit_widths.get(expr) {
            return Some(*bits);
        }
        if *remaining == 0 {
            return None;
        }
        *remaining -= 1;

        let bits = match expr.kind() {
            SymExprKind::Const(value) => value.bit_len().max(1),
            SymExprKind::Var(_)
            | SymExprKind::GasLeft(_)
            | SymExprKind::Keccak { .. }
            | SymExprKind::Hash { .. }
            | SymExprKind::Not(_) => 256,
            SymExprKind::BinOp(SymBinOp::And, left, right) => {
                if let Some(mask) = right.as_const() {
                    self.unsigned_bits_cached(left, bit_widths, remaining)?.min(mask.bit_len())
                } else {
                    256
                }
            }
            SymExprKind::BinOp(SymBinOp::Add, left, right) => self
                .unsigned_bits_cached(left, bit_widths, remaining)?
                .max(self.unsigned_bits_cached(right, bit_widths, remaining)?)
                .saturating_add(1)
                .min(256),
            SymExprKind::BinOp(SymBinOp::Mul, left, right) => self
                .unsigned_bits_cached(left, bit_widths, remaining)?
                .saturating_add(self.unsigned_bits_cached(right, bit_widths, remaining)?)
                .min(256),
            SymExprKind::BinOp(SymBinOp::UDiv, left, _) => {
                self.unsigned_bits_cached(left, bit_widths, remaining)?
            }
            SymExprKind::Ite(_, left, right) => self
                .unsigned_bits_cached(left, bit_widths, remaining)?
                .max(self.unsigned_bits_cached(right, bit_widths, remaining)?),
            _ => 256,
        };

        let bits =
            self.upper_bound(expr).map(|bound| bits.min(bound.bit_len().max(1))).unwrap_or(bits);
        bit_widths.insert(expr.clone(), bits);
        Some(bits)
    }
}

#[cfg(test)]
mod tests;
