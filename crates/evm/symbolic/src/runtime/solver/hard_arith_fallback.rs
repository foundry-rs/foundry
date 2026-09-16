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
    #[cfg(test)]
    pub(crate) fn contains_hard_arith(&self) -> bool {
        self.visit_bool(is_hard_arith_node)
    }

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
        if candidates.len() >= HARD_ARITH_FALLBACK_MAX_CANDIDATES_PER_VAR {
            break;
        }
    }

    for bit in 0..256 {
        let power = U256::from(1) << bit;
        push_fallback_candidate(&mut candidates, power, hints);
        if candidates.len() >= HARD_ARITH_FALLBACK_MAX_CANDIDATES_PER_VAR {
            break;
        }
    }

    let mut candidates = candidates.into_iter().collect::<Vec<_>>();
    candidates.sort_unstable();
    candidates.truncate(HARD_ARITH_FALLBACK_MAX_CANDIDATES_PER_VAR);
    Some(candidates)
}

struct FallbackSearch<'a> {
    constraints: &'a [SymBoolExpr],
    constraint_vars: &'a [SymbolicVars],
    searched_vars: &'a SymbolicVars,
    vars: &'a [Symbol],
    candidates: &'a [Vec<U256>],
}

impl FallbackSearch<'_> {
    fn model(
        &self,
        index: usize,
        model: &mut SymbolicModel,
        assignments: &mut usize,
    ) -> Option<SymbolicModel> {
        if index == self.vars.len() {
            *assignments += 1;
            if *assignments > HARD_ARITH_FALLBACK_MAX_ASSIGNMENTS {
                return None;
            }
            let mut completed = model.clone();
            let mut remaining_support_visits = usize::MAX;
            return complete_fallback_support_model(
                self.constraints,
                &mut completed,
                &mut remaining_support_visits,
            )
            .then_some(completed);
        }

        for candidate in &self.candidates[index] {
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
            if *assignments > HARD_ARITH_FALLBACK_MAX_ASSIGNMENTS {
                return None;
            }
        }
        model.remove(&self.vars[index]);
        None
    }
}

#[cfg(test)]
fn fallback_model_satisfies_all_constraints(
    constraints: &[SymBoolExpr],
    model: &(impl SymbolicModelLookup + ?Sized),
) -> bool {
    eval_model_constraints(constraints, model)
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
            Ok(None) => changed |= complete_support_constraint(constraint, model),
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

fn complete_support_constraint(constraint: &SymBoolExpr, model: &mut SymbolicModel) -> bool {
    complete_support_bool(constraint, model, false, false)
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

pub(crate) fn fallback_two_var_model(constraints: &[SymBoolExpr]) -> Option<SymbolicModel> {
    if constraints.iter().any(SymBoolExpr::contains_hard_arith) {
        return None;
    }

    let mut vars = SymbolicVars::default();
    for constraint in constraints {
        collect_bool_fallback_vars(constraint, &mut vars);
        if vars.len() > 2 {
            return None;
        }
    }
    if vars.len() != 2 {
        return None;
    }
    if constraints.iter().any(SymBoolExpr::contains_symbolic_hash)
        || constraints.iter().any(SymBoolExpr::contains_gasleft)
    {
        return None;
    }
    if !constraints_have_two_var_relation(constraints, &vars)
        || !constraints_bind_each_search_var(constraints, &vars)
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
    };
    let mut model = SymbolicModel::default();
    let mut assignments = 0usize;
    search.model(0, &mut model, &mut assignments)
}

fn constraints_have_two_var_relation(
    constraints: &[SymBoolExpr],
    searched_vars: &SymbolicVars,
) -> bool {
    constraints
        .iter()
        .any(|constraint| bool_expr_has_two_var_relation(constraint, searched_vars, false))
}

fn bool_expr_has_two_var_relation(
    expr: &SymBoolExpr,
    searched_vars: &SymbolicVars,
    inverted: bool,
) -> bool {
    match expr.kind() {
        SymBoolExprKind::Const(_) => false,
        SymBoolExprKind::Not(expr) => {
            bool_expr_has_two_var_relation(expr, searched_vars, !inverted)
        }
        SymBoolExprKind::And(exprs) if !inverted => {
            exprs.iter().any(|expr| bool_expr_has_two_var_relation(expr, searched_vars, false))
        }
        SymBoolExprKind::And(_) => false,
        SymBoolExprKind::Cmp(_, left, right) => {
            let mut vars = SymbolicVars::default();
            collect_expr_fallback_vars(left, &mut vars);
            collect_expr_fallback_vars(right, &mut vars);
            vars.len() == 2 && vars.is_subset(searched_vars)
        }
    }
}

fn constraints_bind_each_search_var(
    constraints: &[SymBoolExpr],
    searched_vars: &SymbolicVars,
) -> bool {
    searched_vars.iter().all(|var| {
        constraints.iter().any(|constraint| bool_expr_binds_single_var(constraint, *var, false))
    })
}

fn bool_expr_binds_single_var(expr: &SymBoolExpr, bound_var: Symbol, inverted: bool) -> bool {
    match expr.kind() {
        SymBoolExprKind::Const(_) => false,
        SymBoolExprKind::Not(expr) => bool_expr_binds_single_var(expr, bound_var, !inverted),
        SymBoolExprKind::And(exprs) if !inverted => {
            exprs.iter().any(|expr| bool_expr_binds_single_var(expr, bound_var, false))
        }
        SymBoolExprKind::And(_) => false,
        SymBoolExprKind::Cmp(_, left, right) => {
            let mut vars = SymbolicVars::default();
            collect_expr_fallback_vars(left, &mut vars);
            collect_expr_fallback_vars(right, &mut vars);
            vars.len() == 1
                && vars.contains(&bound_var)
                && (expr_contains_const(left) || expr_contains_const(right))
        }
    }
}

fn collect_expr_fallback_vars(expr: &SymExpr, vars: &mut SymbolicVars) {
    let _ = expr.visit(&mut |expr| {
        if let Some(var) = expr.kind().get_eval_var() {
            vars.insert(var);
        }
        ControlFlow::<()>::Continue(())
    });
}

fn expr_contains_const(expr: &SymExpr) -> bool {
    expr.visit_bool(|expr| matches!(expr.kind(), SymExprKind::Const(_)))
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
                if is_single_bit(mask) {
                    self.one |= mask;
                }
            } else {
                self.zero |= mask;
            }
        }
    }
}

fn is_single_bit(value: U256) -> bool {
    !value.is_zero() && (value & (value - U256::from(1))).is_zero()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn replayable_input(cx: &mut SymCx, name: &str) -> SymExpr {
        let symbol = cx.intern(name);
        cx.mark_replayable_input(symbol);
        SymExpr::get_var(cx, symbol)
    }

    fn checked_mul_guard_word(
        cx: &mut SymCx,
        zero_operand: &SymExpr,
        expected: &SymExpr,
    ) -> SymExpr {
        let zero = SymExpr::zero(cx);
        let operand_is_zero = SymBoolExpr::eq(cx, zero_operand.clone(), zero.clone());
        let product = SymExpr::binop(cx, SymBinOp::Mul, zero_operand.clone(), expected.clone());
        let quotient = SymExpr::binop(cx, SymBinOp::UDiv, product, zero_operand.clone());
        let checked_product = SymExpr::ite(cx, operand_is_zero.clone(), zero, quotient);
        let operand_is_zero_word = SymExpr::bool_word(cx, operand_is_zero);
        let product_matches_expected = SymBoolExpr::eq(cx, checked_product, expected.clone());
        let product_matches_expected_word = SymExpr::bool_word(cx, product_matches_expected);
        SymExpr::binop(cx, SymBinOp::Or, operand_is_zero_word, product_matches_expected_word)
    }

    #[test]
    fn checked_mul_guard_branch_model_preserves_exact_operand_constraints() {
        let mut cx = SymCx::new();
        let x = replayable_input(&mut cx, "x");
        let y = replayable_input(&mut cx, "y");
        let guard = checked_mul_guard_word(&mut cx, &x, &y);
        let zero = SymExpr::zero(&mut cx);
        let guard_is_false = SymBoolExpr::eq(&mut cx, guard, zero);
        let guard_is_true = guard_is_false.clone().not(&mut cx);

        let seven = SymExpr::constant(&mut cx, U256::from(7));
        let y_is_seven = SymBoolExpr::eq(&mut cx, y.clone(), seven);
        let true_constraints = [guard_is_true, y_is_seven];
        let true_model = checked_mul_guard_branch_model(
            &cx,
            &true_constraints,
            &true_constraints,
            &SymbolicVars::default(),
        )
        .expect("true guard branch model");
        assert_eq!(x.eval_model(&true_model).unwrap(), U256::ZERO);
        assert_eq!(y.eval_model(&true_model).unwrap(), U256::from(7));
        assert!(fallback_model_satisfies_all_constraints(&true_constraints, &true_model));

        let three = SymExpr::constant(&mut cx, U256::from(3));
        let y_is_three = SymBoolExpr::eq(&mut cx, y.clone(), three);
        let false_constraints = [guard_is_false, y_is_three];
        let false_model = checked_mul_guard_branch_model(
            &cx,
            &false_constraints,
            &false_constraints,
            &SymbolicVars::default(),
        )
        .expect("false guard branch model");
        assert_eq!(x.eval_model(&false_model).unwrap(), U256::MAX);
        assert_eq!(y.eval_model(&false_model).unwrap(), U256::from(3));
        assert!(fallback_model_satisfies_all_constraints(&false_constraints, &false_model));
    }

    #[test]
    fn checked_mul_guard_branch_model_matches_nested_boolean_guard() {
        let mut cx = SymCx::new();
        let x = replayable_input(&mut cx, "x");
        let y = replayable_input(&mut cx, "y");
        let zero = SymExpr::zero(&mut cx);
        let x_is_zero = SymBoolExpr::eq(&mut cx, x.clone(), zero.clone());
        let product = SymExpr::binop(&mut cx, SymBinOp::Mul, x.clone(), y.clone());
        let quotient = SymExpr::binop(&mut cx, SymBinOp::UDiv, product.clone(), x.clone());
        let guarded_quotient = SymExpr::ite(&mut cx, x_is_zero.clone(), zero, quotient);
        let quotient_matches = SymBoolExpr::eq(&mut cx, guarded_quotient, y.clone());
        let quotient_mismatches = quotient_matches.not(&mut cx);
        let x_is_nonzero = x_is_zero.not(&mut cx);
        let guard_is_false = SymBoolExpr::and(&mut cx, vec![quotient_mismatches, x_is_nonzero]);
        let max = SymExpr::constant(&mut cx, U256::MAX);
        let product_is_not_max = SymBoolExpr::eq(&mut cx, product, max).not(&mut cx);

        let guard_is_true = guard_is_false.clone().not(&mut cx);
        let nested_false_branch =
            SymBoolExpr::and(&mut cx, vec![guard_is_true, product_is_not_max.clone()]).not(&mut cx);
        let false_constraints = [nested_false_branch.clone()];
        let false_model = checked_mul_guard_branch_model(
            &cx,
            &false_constraints,
            &false_constraints,
            &SymbolicVars::default(),
        )
        .expect("nested false guard branch model");
        assert_eq!(x.eval_model(&false_model).unwrap(), U256::MAX);
        assert_eq!(y.eval_model(&false_model).unwrap(), U256::from(2));
        assert!(fallback_model_satisfies_all_constraints(&false_constraints, &false_model));

        let guard_word = checked_mul_guard_word(&mut cx, &x, &y);
        let zero = SymExpr::zero(&mut cx);
        let word_guard_is_false = SymBoolExpr::eq(&mut cx, guard_word, zero);
        let normalized = [nested_false_branch, word_guard_is_false.clone()];
        let original = [word_guard_is_false, normalized[0].clone()];
        let combined_model =
            checked_mul_guard_branch_model(&cx, &normalized, &original, &SymbolicVars::default())
                .expect("combined word and nested guard model");
        assert!(fallback_model_satisfies_all_constraints(&original, &combined_model));

        let guarded_nonmax_product =
            SymBoolExpr::and(&mut cx, vec![guard_is_false, product_is_not_max]).not(&mut cx);
        let true_constraints = [guarded_nonmax_product];
        let true_model = checked_mul_guard_branch_model(
            &cx,
            &true_constraints,
            &true_constraints,
            &SymbolicVars::default(),
        )
        .expect("nested true guard branch model");
        assert_eq!(x.eval_model(&true_model).unwrap(), U256::ZERO);
        assert_eq!(y.eval_model(&true_model).unwrap(), U256::ZERO);
        assert!(fallback_model_satisfies_all_constraints(&true_constraints, &true_model));
    }

    #[test]
    fn checked_mul_guard_branch_model_completes_original_model_symbols() {
        let mut cx = SymCx::new();
        let x = replayable_input(&mut cx, "x");
        let y = replayable_input(&mut cx, "y");
        let guard = checked_mul_guard_word(&mut cx, &x, &y);
        let zero = SymExpr::zero(&mut cx);
        let guard_is_true = SymBoolExpr::eq(&mut cx, guard, zero).not(&mut cx);
        let slot_symbol = cx.intern("slot");
        let slot = SymExpr::get_var(&mut cx, slot_symbol);
        let one = SymExpr::one(&mut cx);
        let slot_is_not_one = SymBoolExpr::eq(&mut cx, slot, one).not(&mut cx);
        let normalized = [guard_is_true.clone()];
        let original = [guard_is_true, slot_is_not_one];
        let replayable_storage = [slot_symbol].into_iter().collect();

        let model =
            checked_mul_guard_branch_model(&cx, &normalized, &original, &replayable_storage)
                .expect("completed guard branch model");

        assert_eq!(model.get(&slot_symbol), Some(&U256::ZERO));
        assert!(fallback_model_satisfies_all_constraints(&original, &model));
    }

    #[test]
    fn checked_mul_guard_branch_model_rejects_symbolic_hash_assignments() {
        let mut cx = SymCx::new();
        let x = replayable_input(&mut cx, "x");
        let y = replayable_input(&mut cx, "y");
        let guard = checked_mul_guard_word(&mut cx, &x, &y);
        let zero = SymExpr::zero(&mut cx);
        let guard_is_true = SymBoolExpr::eq(&mut cx, guard, zero.clone()).not(&mut cx);
        let y_is_zero = SymBoolExpr::eq(&mut cx, y.clone(), zero.clone());
        let hash_symbol = cx.intern("sha256_y");
        let hash = SymExpr::hash_symbol(&mut cx, hash_symbol, "sha256", vec![y]);
        let hash_is_zero = SymBoolExpr::eq(&mut cx, hash, zero);
        let constraints = [guard_is_true, y_is_zero, hash_is_zero];

        assert!(
            checked_mul_guard_branch_model(
                &cx,
                &constraints,
                &constraints,
                &SymbolicVars::default(),
            )
            .is_none()
        );
    }

    #[test]
    fn checked_mul_guard_branch_model_rejects_gasleft_assignments() {
        let mut cx = SymCx::new();
        let x = replayable_input(&mut cx, "x");
        let y = replayable_input(&mut cx, "y");
        let guard = checked_mul_guard_word(&mut cx, &x, &y);
        let zero = SymExpr::zero(&mut cx);
        let guard_is_true = SymBoolExpr::eq(&mut cx, guard, zero.clone()).not(&mut cx);
        let gas_left = SymExpr::gas_left(&mut cx, 0);
        let gas_is_zero = SymBoolExpr::eq(&mut cx, gas_left, zero);
        let constraints = [guard_is_true, gas_is_zero];

        assert!(
            checked_mul_guard_branch_model(
                &cx,
                &constraints,
                &constraints,
                &SymbolicVars::default(),
            )
            .is_none()
        );
    }

    #[test]
    fn checked_mul_guard_branch_model_rejects_opaque_var_assignments() {
        for name in ["create_address_opaque", "vmRandomUint_0", "svm_0"] {
            let mut cx = SymCx::new();
            let x = replayable_input(&mut cx, "x");
            let y = replayable_input(&mut cx, "y");
            let guard = checked_mul_guard_word(&mut cx, &x, &y);
            let zero = SymExpr::zero(&mut cx);
            let guard_is_true = SymBoolExpr::eq(&mut cx, guard, zero.clone()).not(&mut cx);
            let opaque = SymExpr::var(&mut cx, name);
            let opaque_is_zero = SymBoolExpr::eq(&mut cx, opaque, zero);
            let constraints = [guard_is_true, opaque_is_zero];

            assert!(
                checked_mul_guard_branch_model(
                    &cx,
                    &constraints,
                    &constraints,
                    &SymbolicVars::default(),
                )
                .is_none(),
                "accepted opaque model symbol {name}"
            );
        }
    }

    #[test]
    fn checked_mul_guard_branch_model_propagates_relational_operand_constraints() {
        let mut cx = SymCx::new();
        let x = replayable_input(&mut cx, "x");
        let y = replayable_input(&mut cx, "y");
        let guard = checked_mul_guard_word(&mut cx, &x, &y);
        let zero = SymExpr::zero(&mut cx);
        let guard_is_false = SymBoolExpr::eq(&mut cx, guard, zero);
        let guard_is_true = guard_is_false.clone().not(&mut cx);

        let seven = SymExpr::constant(&mut cx, U256::from(7));
        let x_plus_seven = SymExpr::binop(&mut cx, SymBinOp::Add, x.clone(), seven);
        let y_is_x_plus_seven = SymBoolExpr::eq(&mut cx, y.clone(), x_plus_seven);
        let true_constraints = [guard_is_true, y_is_x_plus_seven];
        let true_model = checked_mul_guard_branch_model(
            &cx,
            &true_constraints,
            &true_constraints,
            &SymbolicVars::default(),
        )
        .expect("true relational model");
        assert_eq!(x.eval_model(&true_model).unwrap(), U256::ZERO);
        assert_eq!(y.eval_model(&true_model).unwrap(), U256::from(7));
        assert!(fallback_model_satisfies_all_constraints(&true_constraints, &true_model));

        let operands_are_equal = SymBoolExpr::eq(&mut cx, x.clone(), y.clone());
        let false_constraints = [guard_is_false, operands_are_equal];
        let false_model = checked_mul_guard_branch_model(
            &cx,
            &false_constraints,
            &false_constraints,
            &SymbolicVars::default(),
        )
        .expect("false relational model");
        assert_eq!(x.eval_model(&false_model).unwrap(), U256::MAX);
        assert_eq!(y.eval_model(&false_model).unwrap(), U256::MAX);
        assert!(fallback_model_satisfies_all_constraints(&false_constraints, &false_model));
    }

    #[test]
    fn checked_mul_guard_branch_model_stops_at_shared_support_budget() {
        let mut cx = SymCx::new();
        let first_x = replayable_input(&mut cx, "first_x");
        let first_y = replayable_input(&mut cx, "first_y");
        let first_guard = checked_mul_guard_word(&mut cx, &first_x, &first_y);
        let zero = SymExpr::zero(&mut cx);
        let first_guard_is_true = SymBoolExpr::eq(&mut cx, first_guard, zero.clone()).not(&mut cx);

        let second_x = replayable_input(&mut cx, "second_x");
        let second_y = replayable_input(&mut cx, "second_y");
        let second_guard = checked_mul_guard_word(&mut cx, &second_x, &second_y);
        let second_guard_is_false = SymBoolExpr::eq(&mut cx, second_guard, zero);

        let one = SymExpr::one(&mut cx);
        let second_x_plus_one = SymExpr::binop(&mut cx, SymBinOp::Add, second_x.clone(), one);
        let x_relation = SymBoolExpr::eq(&mut cx, first_x.clone(), second_x_plus_one);
        let y_relation = SymBoolExpr::eq(&mut cx, first_y.clone(), second_y.clone());
        let mut constraints =
            vec![first_guard_is_true, second_guard_is_false, x_relation, y_relation];
        for _ in 0..8 {
            constraints.push(SymBoolExpr::constant(&mut cx, true));
        }

        let expected = [
            (cx.intern("first_x"), U256::ZERO),
            (cx.intern("first_y"), U256::from(2)),
            (cx.intern("second_x"), U256::MAX),
            (cx.intern("second_y"), U256::from(2)),
        ]
        .into_iter()
        .collect::<SymbolicModel>();
        assert!(fallback_model_satisfies_all_constraints(&constraints, &expected));

        assert!(
            checked_mul_guard_branch_model(
                &cx,
                &constraints,
                &constraints,
                &SymbolicVars::default(),
            )
            .is_none()
        );
    }

    #[test]
    fn checked_mul_guard_branch_model_stops_at_shared_expression_budget() {
        let mut cx = SymCx::new();
        let x = replayable_input(&mut cx, "x");
        let y = replayable_input(&mut cx, "y");
        let guard = checked_mul_guard_word(&mut cx, &x, &y);
        let zero = SymExpr::zero(&mut cx);
        let guard_is_true = SymBoolExpr::eq(&mut cx, guard, zero.clone()).not(&mut cx);

        let source = replayable_input(&mut cx, "source");
        let mut shared = source;
        for _ in 0..9 {
            shared = SymExpr::binop(&mut cx, SymBinOp::Add, shared.clone(), shared);
        }
        let support = SymBoolExpr::eq(&mut cx, shared, zero);
        let original = [guard_is_true, support.clone()];
        let normalized = normalize_constraints_for_solver(&mut cx, &original);

        assert!(normalized.contains(&support));
        assert!(
            checked_mul_guard_branch_model(&cx, &normalized, &original, &SymbolicVars::default(),)
                .is_none()
        );
    }

    #[test]
    fn hard_arith_fallback_ignores_unrelated_abi_vars() {
        let mut cx = SymCx::new();
        let amount = SymExpr::var(&mut cx, "sequence_0_0_0_1");
        let zero = SymExpr::zero(&mut cx);
        let scale = SymExpr::constant(&mut cx, U256::from(1_000_000));
        let product = SymExpr::binop(&mut cx, SymBinOp::Mul, scale.clone(), amount.clone());
        let div = SymExpr::binop(&mut cx, SymBinOp::UDiv, product, amount.clone());
        let amount_is_zero = SymBoolExpr::eq(&mut cx, amount, zero);
        let guarded_zero = SymExpr::zero(&mut cx);
        let guarded_div = SymExpr::ite(&mut cx, amount_is_zero.clone(), guarded_zero, div);
        let overflow_branch = SymBoolExpr::eq(&mut cx, guarded_div, scale).not(&mut cx);

        let address_bound = U256::from(1) << 160;
        let mut constraints = vec![amount_is_zero.not(&mut cx), overflow_branch];
        for idx in 0..6 {
            let abi_word = SymExpr::var(&mut cx, &format!("sequence_0_0_0_addr_{idx}"));
            constraints.push(SymBoolExpr::cmp_word_const(
                &mut cx,
                SymCmpOp::Ult,
                &abi_word,
                address_bound,
            ));
        }

        assert!(constraints_prefer_hard_arith_fallback_first(&cx, &constraints));
        let model = hard_arith_fallback_model(&cx, &constraints).expect("fallback model");
        assert!(model.contains_name(cx.symbol("sequence_0_0_0_1")));
        assert!(constraints.iter().all(|constraint| constraint.eval_model(&model).unwrap()));
    }

    #[test]
    fn hard_arith_fallback_keeps_prior_path_vars_needed_by_zero_model() {
        let mut cx = SymCx::new();
        let setup_amount = SymExpr::var(&mut cx, "sequence_0_0_0_1");
        let borrow_amount = SymExpr::var(&mut cx, "sequence_2_2_0_1");
        let zero = SymExpr::zero(&mut cx);
        let scale = SymExpr::constant(&mut cx, U256::from(1_000_000));
        let product = SymExpr::binop(&mut cx, SymBinOp::Mul, scale.clone(), borrow_amount.clone());
        let quotient = SymExpr::binop(&mut cx, SymBinOp::UDiv, product, borrow_amount.clone());

        let constraints = vec![
            SymBoolExpr::eq(&mut cx, setup_amount, zero.clone()).not(&mut cx),
            SymBoolExpr::eq(&mut cx, borrow_amount, zero).not(&mut cx),
            SymBoolExpr::eq(&mut cx, quotient, scale),
        ];

        assert!(constraints_prefer_hard_arith_fallback_first(&cx, &constraints));
        let model = hard_arith_fallback_model(&cx, &constraints).expect("fallback model");
        assert!(model.contains_name(cx.symbol("sequence_0_0_0_1")));
        assert!(model.contains_name(cx.symbol("sequence_2_2_0_1")));
        assert!(constraints.iter().all(|constraint| constraint.eval_model(&model).unwrap()));
    }

    #[test]
    fn hard_arith_fallback_completes_checked_storage_guards() {
        let mut cx = SymCx::new();
        let amount = SymExpr::var(&mut cx, "sequence_0_0_0_1");
        let from_balance = SymExpr::var(&mut cx, "storage_from_balance");
        let to_balance = SymExpr::var(&mut cx, "storage_to_balance");
        let zero = SymExpr::zero(&mut cx);
        let scale = SymExpr::constant(&mut cx, U256::from(1_000_000));
        let product = SymExpr::binop(&mut cx, SymBinOp::Mul, scale.clone(), amount.clone());
        let quotient = SymExpr::binop(&mut cx, SymBinOp::UDiv, product, amount.clone());

        let debited = SymExpr::binop(&mut cx, SymBinOp::Sub, from_balance.clone(), amount.clone());
        let credited = SymExpr::binop(&mut cx, SymBinOp::Add, to_balance.clone(), amount.clone());
        let mut constraints = vec![
            SymBoolExpr::eq(&mut cx, amount, zero).not(&mut cx),
            SymBoolExpr::eq(&mut cx, quotient, scale),
            SymBoolExpr::cmp(&mut cx, SymCmpOp::Ult, from_balance, debited).not(&mut cx),
            SymBoolExpr::cmp(&mut cx, SymCmpOp::Ult, credited, to_balance).not(&mut cx),
        ];

        let address_bound = U256::from(1) << 160;
        for idx in 0..6 {
            let abi_word = SymExpr::var(&mut cx, &format!("sequence_0_0_0_addr_{idx}"));
            constraints.push(SymBoolExpr::cmp_word_const(
                &mut cx,
                SymCmpOp::Ult,
                &abi_word,
                address_bound,
            ));
        }

        assert!(constraints_prefer_hard_arith_fallback_first(&cx, &constraints));
        let model = hard_arith_fallback_model(&cx, &constraints).expect("fallback model");
        assert!(model.contains_name(cx.symbol("sequence_0_0_0_1")));
        assert!(model.contains_name(cx.symbol("storage_from_balance")));
        assert!(model.contains_name(cx.symbol("storage_to_balance")));
        assert!(constraints.iter().all(|constraint| constraint.eval_model(&model).unwrap()));
    }
}
