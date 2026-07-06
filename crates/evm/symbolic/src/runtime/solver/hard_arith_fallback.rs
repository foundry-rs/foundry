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
            return complete_fallback_support_model(self.constraints, &mut completed)
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

fn fallback_model_satisfies_all_constraints(
    constraints: &[SymBoolExpr],
    model: &(impl SymbolicModelLookup + ?Sized),
) -> bool {
    constraints.iter().all(|constraint| constraint.eval_model(model).unwrap_or(false))
}

fn complete_fallback_support_model(constraints: &[SymBoolExpr], model: &mut SymbolicModel) -> bool {
    for _ in 0..constraints.len() {
        let mut changed = false;
        for constraint in constraints {
            match constraint.eval_model_if_complete(model) {
                Ok(Some(true)) => {}
                Ok(Some(false)) | Err(_) => return false,
                Ok(None) => {
                    changed |= complete_support_constraint(constraint, model);
                }
            }
        }
        if changed {
            continue;
        }
        // Default checked-add bases to zero only after exact/lower-bound completions had a chance
        // to assign a stronger value required by another constraint.
        for constraint in constraints {
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
    fallback_model_satisfies_all_constraints(constraints, model)
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

    let mut candidates = HashSet::<U256>::default();
    for candidate in [
        U256::ZERO,
        U256::from(1),
        U256::from(2),
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
        if constraints.iter().all(|constraint| constraint.eval_model(&model).unwrap_or(false)) {
            return Some(model);
        }
    }

    None
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
