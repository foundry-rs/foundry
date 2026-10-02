//! Bounded fallback model generation for hard arithmetic constraints.

use super::*;

impl SymBoolExpr {
    pub(crate) fn contains_hard_arith(&self) -> bool {
        self.visit_bool(is_hard_arith_node)
    }

    fn contains_symbolic_hash(&self) -> bool {
        self.visit_bool(|expr| matches!(expr.kind(), SymExprKind::Hash { .. }))
    }
}

impl SymExpr {
    fn contains_var(&self) -> bool {
        self.visit_bool(|expr| {
            matches!(
                expr.kind(),
                SymExprKind::Var(_) | SymExprKind::Keccak { .. } | SymExprKind::Hash { .. }
            )
        })
    }
}

fn is_hard_arith_node(expr: &SymExpr) -> bool {
    match expr.kind() {
        SymExprKind::BinOp(SymBinOp::Mul, left, right) => {
            left.contains_var() && right.contains_var()
        }
        SymExprKind::BinOp(
            SymBinOp::UDiv | SymBinOp::URem | SymBinOp::SDiv | SymBinOp::SRem,
            left,
            right,
        ) => left.contains_var() || right.contains_var(),
        SymExprKind::TernOp(_, left, right, modulus) => {
            left.contains_var() || right.contains_var() || modulus.contains_var()
        }
        _ => false,
    }
}

/// Returns whether local hard-arithmetic search should run before asking the solver.
pub(crate) fn constraints_prefer_hard_arith_fallback_first(
    cx: &SymCx,
    constraints: &[SymBoolExpr],
) -> bool {
    if !constraints.iter().any(SymBoolExpr::contains_hard_arith)
        || constraints.iter().any(SymBoolExpr::contains_symbolic_hash)
    {
        return false;
    }

    let mut vars = SymbolicVars::default();
    for constraint in constraints {
        collect_bool_fallback_vars(constraint, &mut vars);
    }
    let vars = fallback_search_vars(cx, vars, constraints);
    !vars.is_empty() && vars.len() <= HARD_ARITH_FALLBACK_MAX_VARS
}

pub(crate) fn hard_arith_fallback_model(
    cx: &SymCx,
    constraints: &[SymBoolExpr],
) -> Option<SymbolicModel> {
    if !constraints.iter().any(SymBoolExpr::contains_hard_arith)
        || constraints.iter().any(SymBoolExpr::contains_symbolic_hash)
    {
        return None;
    }

    let mut vars = SymbolicVars::default();
    let mut constants = HashSet::<U256>::default();
    for constraint in constraints {
        collect_bool_fallback_vars(constraint, &mut vars);
        collect_bool_constants(constraint, &mut constants);
    }
    let mut constants = constants.into_iter().collect::<Vec<_>>();
    constants.sort_unstable();
    let vars = fallback_search_vars(cx, vars, constraints);
    if vars.is_empty() || vars.len() > HARD_ARITH_FALLBACK_MAX_VARS {
        return None;
    }

    let candidates = vars
        .iter()
        .map(|var| fallback_candidates_for_var(var, constraints, &constants))
        .collect::<Option<Vec<_>>>()?;
    let searched_vars = vars.iter().copied().collect::<SymbolicVars>();
    let constraint_vars = constraints
        .iter()
        .map(|constraint| {
            let mut vars = SymbolicVars::default();
            constraint.collect_vars(&mut vars);
            vars
        })
        .collect::<Vec<_>>();
    let mut model = SymbolicModel::default();
    let mut assignments = 0usize;
    let search = FallbackSearch {
        constraints,
        constraint_vars: &constraint_vars,
        searched_vars: &searched_vars,
        vars: &vars,
        candidates: &candidates,
        max_assignments: HARD_ARITH_FALLBACK_MAX_ASSIGNMENTS,
    };
    search.model(0, &mut model, &mut assignments)
}

// Constructive checked-multiply modeling is optional. Bound repeated support scans to keep a miss
// from consuming more work than the solver fallback it is intended to avoid.
const MAX_CHECKED_MUL_SUPPORT_VISITS: usize = 256;

/// Constructs and validates a concrete model for a checked-multiply guard branch.
///
/// Solidity's guard is `x == 0 || (x * y) / x == y`. The assignments below represent its
/// semantic cases directly: the zero disjunct, a nonzero exact product, and wrapping products in
/// either operand order. Simple support constraints are completed first so an exact operand value
/// from the path is preserved instead of being overwritten by the semantic default. This does not
/// perform the generic bounded candidate search, and a model is returned only when it satisfies
/// every original constraint.
pub(super) fn checked_mul_guard_branch_model(
    cx: &SymCx,
    constraints: &[SymBoolExpr],
    original_constraints: &[SymBoolExpr],
    replayable_storage: &SymbolicVars,
) -> Option<SymbolicModel> {
    let mut eval_vars = SymbolicVars::default();
    for constraint in original_constraints {
        constraint.collect_eval_vars(&mut eval_vars);
    }
    if eval_vars
        .iter()
        .any(|var| !cx.is_replayable_input(*var) && !replayable_storage.contains(var))
    {
        return None;
    }

    let mut remaining_support_visits = MAX_CHECKED_MUL_SUPPORT_VISITS;
    let mut candidates = Vec::new();
    let mut seen = HashSet::<&SymBoolExpr>::default();
    let mut pending = Vec::new();
    for constraint in constraints {
        if !seen.insert(constraint) {
            continue;
        }
        if remaining_support_visits == 0 {
            return None;
        }
        remaining_support_visits -= 1;
        if let Some(candidate) = checked_mul_guard_branch(constraint) {
            candidates.push(candidate);
        }
        match constraint.kind() {
            SymBoolExprKind::Not(inner) => pending.push(inner),
            SymBoolExprKind::And(values) => pending.extend(values.iter()),
            SymBoolExprKind::Const(_) | SymBoolExprKind::Cmp(_, _, _) => {}
        }
    }

    let mut nested = Vec::new();
    while let Some(constraint) = pending.pop() {
        if !seen.insert(constraint) {
            continue;
        }
        if remaining_support_visits == 0 {
            return None;
        }
        remaining_support_visits -= 1;
        nested.push(constraint);
        match constraint.kind() {
            SymBoolExprKind::Not(inner) => pending.push(inner),
            SymBoolExprKind::And(values) => pending.extend(values.iter()),
            SymBoolExprKind::Const(_) | SymBoolExprKind::Cmp(_, _, _) => {}
        }
    }
    for constraint in nested.into_iter().rev() {
        if let Some(candidate) = checked_mul_guard_branch(constraint) {
            candidates.push(candidate);
        }
    }

    for (zero_operand, expected, guard_is_true) in candidates {
        let assignments = if guard_is_true {
            [(U256::ZERO, U256::ZERO), (U256::ONE, U256::ONE)]
        } else {
            [(U256::MAX, U256::from(2)), (U256::from(2), U256::MAX)]
        };
        for (zero_default, expected_default) in assignments {
            let seed_orders = [
                [(&zero_operand, zero_default), (&expected, expected_default)],
                [(&expected, expected_default), (&zero_operand, zero_default)],
            ];
            for seeds in seed_orders {
                let mut model = SymbolicModel::default();
                if !propagate_fallback_support_constraints(
                    constraints,
                    &mut model,
                    &mut remaining_support_visits,
                ) {
                    if remaining_support_visits == 0 {
                        return None;
                    }
                    continue;
                }
                let mut valid = true;
                for (operand, default) in seeds {
                    let assigned = match operand.eval_model_if_complete(&model) {
                        Ok(Some(_)) => true,
                        Ok(None) => operand.assign_model_value(&mut model, default),
                        Err(_) => false,
                    };
                    if !assigned {
                        valid = false;
                        break;
                    }
                    if !propagate_fallback_support_constraints(
                        constraints,
                        &mut model,
                        &mut remaining_support_visits,
                    ) {
                        if remaining_support_visits == 0 {
                            return None;
                        }
                        valid = false;
                        break;
                    }
                }
                if valid {
                    if eval_vars.iter().all(|var| model.contains_name(*var)) {
                        let valid = original_constraints.iter().all(|constraint| {
                            charge_support_constraint(constraint, &mut remaining_support_visits)
                                && constraint.eval_model(&model).unwrap_or(false)
                        });
                        if valid {
                            return Some(model);
                        }
                        if remaining_support_visits == 0 {
                            return None;
                        }
                        continue;
                    }
                    if complete_fallback_support_model(
                        constraints,
                        &mut model,
                        &mut remaining_support_visits,
                    ) && complete_model_with_zeroes(
                        original_constraints,
                        &mut model,
                        &mut remaining_support_visits,
                    ) {
                        let valid = original_constraints.iter().all(|constraint| {
                            charge_support_constraint(constraint, &mut remaining_support_visits)
                                && constraint.eval_model(&model).unwrap_or(false)
                        });
                        if valid {
                            return Some(model);
                        }
                    }
                    if remaining_support_visits == 0 {
                        return None;
                    }
                }
            }
        }
    }
    None
}

fn checked_mul_guard_branch(constraint: &SymBoolExpr) -> Option<(SymExpr, SymExpr, bool)> {
    match constraint.kind() {
        SymBoolExprKind::Cmp(SymCmpOp::Eq, left, right) => {
            checked_mul_guard_word_comparison(left, right)
                .map(|(zero_operand, expected)| (zero_operand, expected, false))
        }
        SymBoolExprKind::Not(inner) => match inner.kind() {
            SymBoolExprKind::Cmp(SymCmpOp::Eq, left, right) => {
                checked_mul_guard_word_comparison(left, right)
                    .map(|(zero_operand, expected)| (zero_operand, expected, true))
            }
            SymBoolExprKind::And(values) => checked_mul_guard_conjunction(values)
                .map(|(zero_operand, expected)| (zero_operand, expected, true)),
            _ => None,
        },
        SymBoolExprKind::And(values) => checked_mul_guard_conjunction(values)
            .map(|(zero_operand, expected)| (zero_operand, expected, false)),
        _ => None,
    }
}

fn checked_mul_guard_word_comparison(
    left: &SymExpr,
    right: &SymExpr,
) -> Option<(SymExpr, SymExpr)> {
    let guard_word = if right.as_const().is_some_and(|value| value.is_zero()) {
        left
    } else if left.as_const().is_some_and(|value| value.is_zero()) {
        right
    } else {
        return None;
    };
    let SymExprKind::BinOp(SymBinOp::Or, left, right) = guard_word.kind() else {
        return None;
    };

    for (quotient_word, zero_word) in [(left, right), (right, left)] {
        let Some(quotient_matches) = quotient_word.bool_word_condition() else {
            continue;
        };
        let Some(zero_condition) = zero_word.bool_word_condition() else {
            continue;
        };
        let Some((zero_operand, expected, quotient_zero_condition)) =
            checked_mul_guard_operands(&quotient_matches)
        else {
            continue;
        };
        if zero_condition == quotient_zero_condition {
            return Some((zero_operand, expected));
        }
    }
    None
}

fn checked_mul_guard_conjunction(values: &[SymBoolExpr]) -> Option<(SymExpr, SymExpr)> {
    for value in values {
        let SymBoolExprKind::Not(quotient_matches) = value.kind() else {
            continue;
        };
        let Some((zero_operand, expected, zero_condition)) =
            checked_mul_guard_operands(quotient_matches)
        else {
            continue;
        };
        let contains_negated_zero_condition = values.iter().any(
            |value| matches!(value.kind(), SymBoolExprKind::Not(inner) if inner == &zero_condition),
        );
        if contains_negated_zero_condition {
            return Some((zero_operand, expected));
        }
    }
    None
}

fn checked_mul_guard_operands(condition: &SymBoolExpr) -> Option<(SymExpr, SymExpr, SymBoolExpr)> {
    let SymBoolExprKind::Cmp(SymCmpOp::Eq, left, right) = condition.kind() else {
        return None;
    };
    for (guarded_quotient, expected) in [(left, right), (right, left)] {
        let SymExprKind::Ite(zero_condition, zero, quotient) = guarded_quotient.kind() else {
            continue;
        };
        if !zero.as_const().is_some_and(|value| value.is_zero()) {
            continue;
        }
        let Some(zero_operand) = zero_condition.zero_check_operand() else {
            continue;
        };
        let Some((numerator, denominator)) = quotient.udiv_operands() else {
            continue;
        };
        if denominator != zero_operand {
            continue;
        }
        let SymExprKind::BinOp(SymBinOp::Mul, product_left, product_right) = numerator.kind()
        else {
            continue;
        };
        if (product_left == denominator && product_right == expected)
            || (product_right == denominator && product_left == expected)
        {
            return Some((zero_operand.clone(), expected.clone(), zero_condition.clone()));
        }
    }
    None
}

fn fallback_search_vars(
    cx: &SymCx,
    vars: SymbolicVars,
    constraints: &[SymBoolExpr],
) -> Vec<Symbol> {
    if vars.len() <= HARD_ARITH_FALLBACK_MAX_VARS {
        return vars.into_iter().collect();
    }

    let hard_arith_vars = hard_arith_fallback_vars(constraints);
    if !hard_arith_vars.is_empty() && hard_arith_vars.len() <= HARD_ARITH_FALLBACK_MAX_VARS {
        let mut vars = hard_arith_vars;
        add_zero_invalid_support_vars(&mut vars, constraints);
        return vars.into_iter().collect();
    }

    vars.into_iter()
        .filter(|var| {
            let var = cx.symbol_name(*var);
            var.starts_with("calldata")
                || var.starts_with("sequence")
                || var.starts_with("create_address")
                || var.starts_with("create2_address")
                || !var.contains('_')
        })
        .collect()
}

fn hard_arith_fallback_vars(constraints: &[SymBoolExpr]) -> SymbolicVars {
    let mut vars = SymbolicVars::default();
    for constraint in constraints {
        collect_bool_hard_arith_vars(constraint, &mut vars);
    }
    vars
}

fn add_zero_invalid_support_vars(vars: &mut SymbolicVars, constraints: &[SymBoolExpr]) {
    let zero_model = SymbolicModel::default();
    for constraint in constraints {
        if constraint.eval_model(&zero_model).unwrap_or(false) {
            continue;
        }
        // Scalar bounds can be completed constructively. Spending a search slot on them can
        // miss a valid endpoint (e.g. int256::MAX + 1) that is outside the candidate budget.
        let (inner, inverted) = match constraint.kind() {
            SymBoolExprKind::Not(inner) => (inner, true),
            _ => (constraint, false),
        };
        if let SymBoolExprKind::Cmp(op, left, right) = inner.kind()
            && support_cmp_op(*op, inverted)
                .is_some_and(|op| !matches!(op, SymCmpOp::Slt | SymCmpOp::Sgt))
            && [(left, right), (right, left)].into_iter().any(|(variable, bound)| {
                if !matches!(variable.kind(), SymExprKind::Var(_)) {
                    return false;
                }
                let mut dependencies = SymbolicVars::default();
                bound.collect_eval_vars(&mut dependencies);
                dependencies.is_subset(vars)
            })
        {
            continue;
        }

        let mut constraint_vars = SymbolicVars::default();
        constraint.collect_vars(&mut constraint_vars);
        let missing =
            constraint_vars.iter().filter(|var| !vars.contains(*var)).copied().collect::<Vec<_>>();
        if vars.len() + missing.len() > HARD_ARITH_FALLBACK_MAX_VARS {
            continue;
        }
        vars.extend(missing);
    }
}

fn fallback_candidates_for_var(
    var: &Symbol,
    constraints: &[SymBoolExpr],
    constants: &[U256],
) -> Option<Vec<U256>> {
    let hints = MaskHints::for_var(var, constraints);
    if (hints.one & hints.zero) != U256::ZERO {
        return None;
    }

    let mut candidates = HashSet::<U256>::default();
    for candidate in [
        U256::ZERO,
        U256::from(1),
        U256::from(2),
        U256::from(3),
        U256::MAX,
        U256::MAX - U256::from(1),
        U256::MAX - U256::from(2),
    ] {
        push_fallback_candidate(&mut candidates, candidate, hints);
    }

    for constant in constants.iter().copied() {
        push_fallback_candidate(&mut candidates, constant, hints);
        push_fallback_candidate(&mut candidates, constant.wrapping_add(U256::from(1)), hints);
        push_fallback_candidate(&mut candidates, constant.wrapping_sub(U256::from(1)), hints);
        if candidates.len() >= FALLBACK_MODEL_MAX_CANDIDATES_PER_VAR {
            break;
        }
    }

    for bit in 0..256 {
        let power = U256::from(1) << bit;
        push_fallback_candidate(&mut candidates, power, hints);
        if candidates.len() >= FALLBACK_MODEL_MAX_CANDIDATES_PER_VAR {
            break;
        }
    }

    let mut candidates = candidates.into_iter().collect::<Vec<_>>();
    candidates.sort_unstable();
    candidates.truncate(FALLBACK_MODEL_MAX_CANDIDATES_PER_VAR);
    Some(candidates)
}

struct FallbackSearch<'a> {
    constraints: &'a [SymBoolExpr],
    constraint_vars: &'a [SymbolicVars],
    searched_vars: &'a SymbolicVars,
    vars: &'a [Symbol],
    candidates: &'a [Vec<U256>],
    max_assignments: usize,
}

impl FallbackSearch<'_> {
    fn model(
        &self,
        index: usize,
        model: &mut SymbolicModel,
        assignments: &mut usize,
    ) -> Option<SymbolicModel> {
        if index == self.vars.len() {
            let mut completed = model.clone();
            let mut remaining_support_visits = usize::MAX;
            if complete_fallback_support_model(
                self.constraints,
                &mut completed,
                &mut remaining_support_visits,
            ) {
                return Some(completed);
            }
            // Greedy completion can choose one endpoint before seeing a tighter bound. Retry
            // from the search assignment after intersecting all currently evaluable bounds.
            let mut completed = model.clone();
            return (seed_bounded_support_vars(self.constraints, &mut completed)
                && complete_fallback_support_model(
                    self.constraints,
                    &mut completed,
                    &mut remaining_support_visits,
                ))
            .then_some(completed);
        }

        for candidate in &self.candidates[index] {
            if *assignments >= self.max_assignments {
                return None;
            }
            *assignments += 1;
            model.insert(self.vars[index], *candidate);
            if fallback_partial_model_satisfies_known_constraints(
                self.constraints,
                self.constraint_vars,
                self.searched_vars,
                model,
            ) && let Some(model) = self.model(index + 1, model, assignments)
            {
                return Some(model);
            }
        }
        model.remove(&self.vars[index]);
        None
    }
}

/// Seeds only unassigned scalar variables; every resulting witness is still fully validated.
fn seed_bounded_support_vars(constraints: &[SymBoolExpr], model: &mut SymbolicModel) -> bool {
    let mut bounds: HashMap<Symbol, (U256, U256)> = HashMap::default();
    for constraint in constraints {
        let (constraint, inverted) = match constraint.kind() {
            SymBoolExprKind::Not(inner) => (inner, true),
            _ => (constraint, false),
        };
        let SymBoolExprKind::Cmp(op, left, right) = constraint.kind() else { continue };
        let Some(mut op) = support_cmp_op(*op, inverted) else { continue };
        if matches!(op, SymCmpOp::Slt | SymCmpOp::Sgt) {
            continue;
        }
        let (var, known) = if let SymExprKind::Var(var) = left.kind()
            && !model.contains_name(*var)
            && let Ok(Some(value)) = right.eval_model_if_complete(model)
        {
            (*var, value)
        } else if let SymExprKind::Var(var) = right.kind()
            && !model.contains_name(*var)
            && let Ok(Some(value)) = left.eval_model_if_complete(model)
        {
            op = match op {
                SymCmpOp::Ult => SymCmpOp::Ugt,
                SymCmpOp::Ule => SymCmpOp::Uge,
                SymCmpOp::Ugt => SymCmpOp::Ult,
                SymCmpOp::Uge => SymCmpOp::Ule,
                other => other,
            };
            (*var, value)
        } else {
            continue;
        };
        let Some(value) = support_target_for_known_right(op, known) else { return false };
        let (lower, upper) = bounds.entry(var).or_insert((U256::ZERO, U256::MAX));
        match op {
            SymCmpOp::Eq => {
                *lower = (*lower).max(value);
                *upper = (*upper).min(value);
            }
            SymCmpOp::Ule | SymCmpOp::Ult => *upper = (*upper).min(value),
            SymCmpOp::Uge | SymCmpOp::Ugt => *lower = (*lower).max(value),
            SymCmpOp::Slt | SymCmpOp::Sgt => continue,
        }
        if lower > upper {
            return false;
        }
    }
    if bounds.is_empty() {
        return false;
    }
    for (var, (lower, _)) in bounds {
        model.insert(var, lower);
    }
    true
}

fn complete_fallback_support_model(
    constraints: &[SymBoolExpr],
    model: &mut SymbolicModel,
    remaining_support_visits: &mut usize,
) -> bool {
    for _ in 0..constraints.len() {
        let Some(mut changed) =
            complete_support_constraints_once(constraints, model, remaining_support_visits)
        else {
            return false;
        };
        if changed {
            continue;
        }
        // Default checked-add bases to zero only after exact/lower-bound completions had a chance
        // to assign a stronger value required by another constraint.
        for constraint in constraints {
            if !charge_support_constraint(constraint, remaining_support_visits) {
                return false;
            }
            match constraint.eval_model_if_complete(model) {
                Ok(Some(true)) => {}
                Ok(Some(false)) | Err(_) => return false,
                Ok(None) => {
                    changed |= complete_default_support_constraint(constraint, model);
                }
            }
        }
        if !changed {
            break;
        }
    }
    constraints.iter().all(|constraint| {
        charge_support_constraint(constraint, remaining_support_visits)
            && constraint.eval_model(model).unwrap_or(false)
    })
}

fn propagate_fallback_support_constraints(
    constraints: &[SymBoolExpr],
    model: &mut SymbolicModel,
    remaining_support_visits: &mut usize,
) -> bool {
    for _ in 0..constraints.len() {
        match complete_support_constraints_once(constraints, model, remaining_support_visits) {
            Some(true) => {}
            Some(false) => return true,
            None => return false,
        }
    }
    true
}

fn complete_support_constraints_once(
    constraints: &[SymBoolExpr],
    model: &mut SymbolicModel,
    remaining_support_visits: &mut usize,
) -> Option<bool> {
    let mut changed = false;
    for constraint in constraints {
        if !charge_support_constraint(constraint, remaining_support_visits) {
            return None;
        }
        match constraint.eval_model_if_complete(model) {
            Ok(Some(true)) => {}
            Ok(Some(false)) | Err(_) => return None,
            Ok(None) => changed |= complete_support_bool(constraint, model, false, false),
        }
    }
    Some(changed)
}

fn charge_support_constraint(
    constraint: &SymBoolExpr,
    remaining_support_visits: &mut usize,
) -> bool {
    if *remaining_support_visits == 0 {
        return false;
    }
    *remaining_support_visits -= 1;
    !constraint
        .visit_exprs(&mut |_| {
            if *remaining_support_visits == 0 {
                return ControlFlow::Break(());
            }
            *remaining_support_visits -= 1;
            ControlFlow::Continue(())
        })
        .is_break()
}

fn complete_model_with_zeroes(
    constraints: &[SymBoolExpr],
    model: &mut SymbolicModel,
    remaining_support_visits: &mut usize,
) -> bool {
    let mut vars = SymbolicVars::default();
    for constraint in constraints {
        if !charge_support_constraint(constraint, remaining_support_visits) {
            return false;
        }
        constraint.collect_eval_vars(&mut vars);
    }
    for var in vars {
        model.entry(var).or_default();
    }
    true
}

fn complete_default_support_constraint(
    constraint: &SymBoolExpr,
    model: &mut SymbolicModel,
) -> bool {
    complete_support_bool(constraint, model, false, true)
}

fn complete_support_bool(
    constraint: &SymBoolExpr,
    model: &mut SymbolicModel,
    inverted: bool,
    defaults_only: bool,
) -> bool {
    match constraint.kind() {
        SymBoolExprKind::Const(_) => false,
        SymBoolExprKind::Not(value) => {
            complete_support_bool(value, model, !inverted, defaults_only)
        }
        SymBoolExprKind::And(values) if !inverted => {
            let mut changed = false;
            for value in values.iter() {
                changed |= complete_support_bool(value, model, false, defaults_only);
            }
            changed
        }
        SymBoolExprKind::Cmp(op, left, right) => {
            let Some(op) = support_cmp_op(*op, inverted) else {
                return false;
            };
            if defaults_only {
                complete_default_support_comparison(op, left, right, model)
            } else {
                complete_support_comparison(op, left, right, model)
            }
        }
        SymBoolExprKind::And(_) => false,
    }
}

const fn support_cmp_op(op: SymCmpOp, inverted: bool) -> Option<SymCmpOp> {
    if !inverted {
        return Some(op);
    }

    match op {
        SymCmpOp::Ult => Some(SymCmpOp::Uge),
        SymCmpOp::Ugt => Some(SymCmpOp::Ule),
        SymCmpOp::Ule => Some(SymCmpOp::Ugt),
        SymCmpOp::Uge => Some(SymCmpOp::Ult),
        SymCmpOp::Eq | SymCmpOp::Slt | SymCmpOp::Sgt => None,
    }
}

fn complete_support_comparison(
    op: SymCmpOp,
    left: &SymExpr,
    right: &SymExpr,
    model: &mut SymbolicModel,
) -> bool {
    if complete_checked_sub_guard(op, left, right, model) {
        return true;
    }
    if let Ok(Some(value)) = left.eval_model_if_complete(model)
        && let Some(target) = support_target_for_known_left(op, value)
    {
        return right.assign_model_value(model, target);
    }
    if let Ok(Some(value)) = right.eval_model_if_complete(model)
        && let Some(target) = support_target_for_known_right(op, value)
    {
        return left.assign_model_value(model, target);
    }
    false
}

fn complete_default_support_comparison(
    op: SymCmpOp,
    left: &SymExpr,
    right: &SymExpr,
    model: &mut SymbolicModel,
) -> bool {
    complete_checked_add_guard(op, left, right, model)
}

fn complete_checked_sub_guard(
    op: SymCmpOp,
    left: &SymExpr,
    right: &SymExpr,
    model: &mut SymbolicModel,
) -> bool {
    match op {
        SymCmpOp::Uge => assign_checked_sub_minuend(left, right, model),
        SymCmpOp::Ule => assign_checked_sub_minuend(right, left, model),
        _ => false,
    }
}

fn assign_checked_sub_minuend(
    minuend: &SymExpr,
    sub_expr: &SymExpr,
    model: &mut SymbolicModel,
) -> bool {
    let SymExprKind::BinOp(SymBinOp::Sub, sub_minuend, amount) = sub_expr.kind() else {
        return false;
    };
    if sub_minuend != minuend {
        return false;
    }
    let Ok(Some(amount)) = amount.eval_model_if_complete(model) else {
        return false;
    };
    minuend.assign_model_value(model, amount)
}

fn complete_checked_add_guard(
    op: SymCmpOp,
    left: &SymExpr,
    right: &SymExpr,
    model: &mut SymbolicModel,
) -> bool {
    match op {
        SymCmpOp::Uge => assign_checked_add_base(left, right, model),
        SymCmpOp::Ule => assign_checked_add_base(right, left, model),
        _ => false,
    }
}

fn assign_checked_add_base(sum: &SymExpr, base: &SymExpr, model: &mut SymbolicModel) -> bool {
    let SymExprKind::BinOp(SymBinOp::Add, left, right) = sum.kind() else {
        return false;
    };
    if left == base && right.eval_model_if_complete(model).ok().flatten().is_some() {
        return base.assign_model_value(model, U256::ZERO);
    }
    if right == base && left.eval_model_if_complete(model).ok().flatten().is_some() {
        return base.assign_model_value(model, U256::ZERO);
    }
    false
}

fn support_target_for_known_left(op: SymCmpOp, value: U256) -> Option<U256> {
    match op {
        SymCmpOp::Eq | SymCmpOp::Ule | SymCmpOp::Uge => Some(value),
        SymCmpOp::Ult => value.checked_add(U256::from(1)),
        SymCmpOp::Ugt => value.checked_sub(U256::from(1)),
        SymCmpOp::Slt | SymCmpOp::Sgt => None,
    }
}

fn support_target_for_known_right(op: SymCmpOp, value: U256) -> Option<U256> {
    match op {
        SymCmpOp::Eq | SymCmpOp::Ule | SymCmpOp::Uge => Some(value),
        SymCmpOp::Ult => value.checked_sub(U256::from(1)),
        SymCmpOp::Ugt => value.checked_add(U256::from(1)),
        SymCmpOp::Slt | SymCmpOp::Sgt => None,
    }
}

fn fallback_partial_model_satisfies_known_constraints(
    constraints: &[SymBoolExpr],
    constraint_vars: &[SymbolicVars],
    searched_vars: &SymbolicVars,
    model: &SymbolicModel,
) -> bool {
    constraints.iter().zip(constraint_vars).all(|(constraint, vars)| {
        !vars.is_subset(searched_vars)
            || !vars.iter().all(|var| model.contains_name(*var))
            || constraint.eval_model(model).unwrap_or(false)
    })
}

fn collect_bool_fallback_vars(expr: &SymBoolExpr, vars: &mut SymbolicVars) {
    let _ = expr.visit_exprs(&mut |expr| {
        if let Some(var) = expr.kind().get_eval_var() {
            vars.insert(var);
        }
        ControlFlow::<()>::Continue(())
    });
}

fn collect_bool_hard_arith_vars(expr: &SymBoolExpr, vars: &mut SymbolicVars) {
    let _ = expr.visit_exprs(&mut |expr| {
        if is_hard_arith_node(expr) {
            expr.collect_eval_vars(vars);
        }
        ControlFlow::<()>::Continue(())
    });
}

pub(crate) fn fallback_single_var_model(constraints: &[SymBoolExpr]) -> Option<SymbolicModel> {
    let mut vars = SymbolicVars::default();
    let mut constants = HashSet::<U256>::default();
    for constraint in constraints {
        constraint.collect_vars(&mut vars);
        collect_bool_constants(constraint, &mut constants);
    }
    let mut constants = constants.into_iter().collect::<Vec<_>>();
    constants.sort_unstable();

    let var = if vars.len() == 1 { *vars.iter().next()? } else { return None };
    let hints = MaskHints::for_var(&var, constraints);
    if (hints.one & hints.zero) != U256::ZERO {
        return None;
    }

    let mut model = SymbolicModel::default();
    let mut remaining_support_visits = usize::MAX;
    if complete_fallback_support_model(constraints, &mut model, &mut remaining_support_visits)
        && model.len() == 1
        && model.contains_key(&var)
    {
        return Some(model);
    }

    for candidate in [
        U256::ZERO,
        U256::from(1),
        U256::from(2),
        U256::MAX,
        U256::MAX - U256::from(1),
        U256::MAX - U256::from(2),
    ] {
        let mut model = SymbolicModel::default();
        model.insert(var, (candidate | hints.one) & !hints.zero);
        if eval_model_constraints(constraints, &model) {
            return Some(model);
        }
    }

    let mut candidates = HashSet::<U256>::default();
    for constant in constants.iter().copied() {
        push_fallback_candidate(&mut candidates, constant, hints);
        push_fallback_candidate(&mut candidates, constant.wrapping_add(U256::from(1)), hints);
        push_fallback_candidate(&mut candidates, constant.wrapping_sub(U256::from(1)), hints);
    }

    for bit in 0..256 {
        let power = U256::from(1) << bit;
        push_fallback_candidate(&mut candidates, power, hints);
        for constant in constants.iter().copied().take(64) {
            push_fallback_candidate(&mut candidates, power | constant, hints);
            push_fallback_candidate(&mut candidates, power.wrapping_add(constant), hints);
        }
    }

    let mut candidates = candidates.into_iter().collect::<Vec<_>>();
    candidates.sort_unstable();
    for candidate in candidates {
        let mut model = SymbolicModel::default();
        model.insert(var, candidate);
        if eval_model_constraints(constraints, &model) {
            return Some(model);
        }
    }

    None
}

/// Searches a bounded Cartesian portfolio and returns only an evaluator-validated SAT witness.
///
/// Unsupported expressions and exhausted search return `None`, leaving the external solver as the
/// authoritative fallback.
pub(crate) fn fallback_bounded_model(constraints: &[SymBoolExpr]) -> Option<SymbolicModel> {
    if constraints.iter().any(SymBoolExpr::contains_hard_arith) {
        return None;
    }

    let mut vars = SymbolicVars::default();
    for constraint in constraints {
        collect_bool_fallback_vars(constraint, &mut vars);
        if vars.len() > FALLBACK_MODEL_MAX_VARS {
            return None;
        }
    }
    if vars.len() < 2 {
        return None;
    }
    if constraints.iter().any(SymBoolExpr::contains_symbolic_hash)
        || constraints.iter().any(SymBoolExpr::contains_gasleft)
    {
        return None;
    }
    let mut constants = HashSet::<U256>::default();
    for constraint in constraints {
        collect_bool_constants(constraint, &mut constants);
    }
    let mut constants = constants.into_iter().collect::<Vec<_>>();
    constants.sort_unstable();
    let vars = vars.into_iter().collect::<Vec<_>>();
    let candidates = vars
        .iter()
        .map(|var| fallback_candidates_for_var(var, constraints, &constants))
        .collect::<Option<Vec<_>>>()?;
    let searched_vars = vars.iter().copied().collect::<SymbolicVars>();
    let constraint_vars = constraints
        .iter()
        .map(|constraint| {
            let mut vars = SymbolicVars::default();
            constraint.collect_vars(&mut vars);
            vars
        })
        .collect::<Vec<_>>();
    let search = FallbackSearch {
        constraints,
        constraint_vars: &constraint_vars,
        searched_vars: &searched_vars,
        vars: &vars,
        candidates: &candidates,
        max_assignments: FALLBACK_MODEL_MAX_ASSIGNMENTS,
    };
    let mut model = SymbolicModel::default();
    let mut assignments = 0usize;
    search.model(0, &mut model, &mut assignments)
}

fn push_fallback_candidate(candidates: &mut HashSet<U256>, candidate: U256, hints: MaskHints) {
    candidates.insert((candidate | hints.one) & !hints.zero);
}

fn collect_bool_constants(expr: &SymBoolExpr, constants: &mut HashSet<U256>) {
    let _ = expr.visit_exprs(&mut |expr| {
        if let SymExprKind::Const(value) = expr.kind() {
            constants.insert(*value);
        }
        ControlFlow::<()>::Continue(())
    });
}

#[derive(Clone, Copy, Debug, Default)]
struct MaskHints {
    one: U256,
    zero: U256,
}

impl MaskHints {
    fn for_var(var: &Symbol, constraints: &[SymBoolExpr]) -> Self {
        let mut hints = Self::default();
        for constraint in constraints {
            hints.apply_bool(var, constraint, false);
        }
        hints
    }

    fn apply_bool(&mut self, var: &Symbol, expr: &SymBoolExpr, inverted: bool) {
        match expr.kind() {
            SymBoolExprKind::Const(_) => {}
            SymBoolExprKind::Not(value) => self.apply_bool(var, value, !inverted),
            SymBoolExprKind::And(values) if !inverted => {
                for value in values.iter() {
                    self.apply_bool(var, value, false);
                }
            }
            SymBoolExprKind::Cmp(SymCmpOp::Eq, left, right) => {
                self.apply_equality(var, left, right, inverted)
            }
            SymBoolExprKind::Cmp(_, _, _) | SymBoolExprKind::And(_) => {}
        }
    }

    fn apply_equality(&mut self, var: &Symbol, left: &SymExpr, right: &SymExpr, inverted: bool) {
        if let Some(mask) =
            zero_mask_equality(var, left, right).or_else(|| zero_mask_equality(var, right, left))
        {
            if inverted {
                if mask.is_power_of_two() {
                    self.one |= mask;
                }
            } else {
                self.zero |= mask;
            }
        }
    }
}

fn zero_mask_equality(var: &Symbol, masked: &SymExpr, zero: &SymExpr) -> Option<U256> {
    if !zero.as_const().is_some_and(|value| value.is_zero()) {
        return None;
    }
    match masked.kind() {
        SymExprKind::BinOp(SymBinOp::And, left, right)
            if left.kind().get_var().is_some_and(|name| &name == var) =>
        {
            right.as_const()
        }
        _ => None,
    }
}
