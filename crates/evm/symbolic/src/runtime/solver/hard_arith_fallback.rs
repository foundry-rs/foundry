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
        SymExprKind::BinOp(SymExprBinOp::Mul, left, right) => {
            left.contains_var() && right.contains_var()
        }
        SymExprKind::BinOp(
            SymExprBinOp::UDiv | SymExprBinOp::URem | SymExprBinOp::SDiv | SymExprBinOp::SRem,
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
pub(crate) fn constraints_prefer_hard_arith_fallback_first(constraints: &[SymBoolExpr]) -> bool {
    if !constraints.iter().any(SymBoolExpr::contains_hard_arith)
        || constraints.iter().any(SymBoolExpr::contains_symbolic_hash)
    {
        return false;
    }

    let mut vars = SymbolicVars::default();
    for constraint in constraints {
        collect_bool_fallback_vars(constraint, &mut vars);
    }
    let vars = fallback_search_vars(vars);
    !vars.is_empty() && vars.len() <= HARD_ARITH_FALLBACK_MAX_VARS
}

pub(crate) fn hard_arith_fallback_model(constraints: &[SymBoolExpr]) -> Option<SymbolicModel> {
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
    let vars = fallback_search_vars(vars);
    if vars.is_empty() || vars.len() > HARD_ARITH_FALLBACK_MAX_VARS {
        return None;
    }

    let candidates = vars
        .iter()
        .map(|var| fallback_candidates_for_var(var.as_str(), constraints, &constants))
        .collect::<Option<Vec<_>>>()?;
    let searched_vars = vars.iter().cloned().collect::<SymbolicVars>();
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

fn fallback_search_vars(vars: SymbolicVars) -> Vec<Symbol> {
    if vars.len() <= HARD_ARITH_FALLBACK_MAX_VARS {
        return vars.into_iter().collect();
    }

    vars.into_iter()
        .filter(|var| {
            let var = var.as_str();
            var.starts_with("calldata")
                || var.starts_with("sequence")
                || var.starts_with("create_address")
                || var.starts_with("create2_address")
                || !var.contains('_')
        })
        .collect()
}

fn fallback_candidates_for_var(
    var: &str,
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
            return fallback_model_satisfies_all_constraints(self.constraints, model)
                .then(|| model.clone());
        }

        for candidate in &self.candidates[index] {
            model.insert(self.vars[index].clone(), *candidate);
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

fn fallback_partial_model_satisfies_known_constraints(
    constraints: &[SymBoolExpr],
    constraint_vars: &[SymbolicVars],
    searched_vars: &SymbolicVars,
    model: &SymbolicModel,
) -> bool {
    constraints.iter().zip(constraint_vars).all(|(constraint, vars)| {
        !vars.is_subset(searched_vars)
            || !vars.iter().all(|var| model.contains_name(var.clone()))
            || constraint.eval_model(model).unwrap_or(false)
    })
}

fn collect_bool_fallback_vars(expr: &SymBoolExpr, vars: &mut SymbolicVars) {
    let _ = expr.visit_exprs(&mut |expr| {
        if let SymExprKind::Var(var) = expr.kind() {
            vars.insert(var.clone());
        }
        ControlFlow::<()>::Continue(())
    });
}

#[cfg(test)]
pub(crate) fn fallback_single_var_model(constraints: &[SymBoolExpr]) -> Option<SymbolicModel> {
    let mut vars = SymbolicVars::default();
    let mut constants = HashSet::<U256>::default();
    for constraint in constraints {
        constraint.collect_vars(&mut vars);
        collect_bool_constants(constraint, &mut constants);
    }
    let mut constants = constants.into_iter().collect::<Vec<_>>();
    constants.sort_unstable();

    let var = if vars.len() == 1 { vars.iter().next()?.clone() } else { return None };
    let hints = MaskHints::for_var(var.as_str(), constraints);
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
        model.insert(var.clone(), candidate);
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
    fn for_var(var: &str, constraints: &[SymBoolExpr]) -> Self {
        let mut hints = Self::default();
        for constraint in constraints {
            hints.apply_bool(var, constraint, false);
        }
        hints
    }

    fn apply_bool(&mut self, var: &str, expr: &SymBoolExpr, inverted: bool) {
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

    fn apply_equality(&mut self, var: &str, left: &SymExpr, right: &SymExpr, inverted: bool) {
        if let Some(mask) =
            zero_mask_equality(var, left, right).or_else(|| zero_mask_equality(var, right, left))
        {
            if inverted {
                self.one |= mask;
            } else {
                self.zero |= mask;
            }
        }
    }
}

fn zero_mask_equality(var: &str, masked: &SymExpr, zero: &SymExpr) -> Option<U256> {
    if !zero.as_const().is_some_and(|value| value.is_zero()) {
        return None;
    }
    match masked.kind() {
        SymExprKind::BinOp(SymExprBinOp::And, left, right) => match (left.kind(), right.kind()) {
            (SymExprKind::Var(name), SymExprKind::Const(mask))
            | (SymExprKind::Const(mask), SymExprKind::Var(name))
                if name.as_str() == var =>
            {
                Some(*mask)
            }
            _ => None,
        },
        _ => None,
    }
}
