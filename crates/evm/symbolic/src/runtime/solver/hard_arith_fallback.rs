use super::*;

pub(crate) fn bool_contains_hard_arith(expr: &BoolExpr) -> bool {
    match expr.as_inner() {
        BoolExprInner::Const(_) => false,
        BoolExprInner::Not(value) => bool_contains_hard_arith(value),
        BoolExprInner::And(values) => values.iter().any(bool_contains_hard_arith),
        BoolExprInner::Eq(left, right) | BoolExprInner::Cmp(_, left, right) => {
            expr_contains_hard_arith(left) || expr_contains_hard_arith(right)
        }
    }
}

pub(crate) fn expr_contains_hard_arith(expr: &Expr) -> bool {
    match expr.as_inner() {
        ExprInner::Const(_)
        | ExprInner::Var(_)
        | ExprInner::GasLeft(_)
        | ExprInner::Keccak { .. }
        | ExprInner::Hash { .. } => false,
        ExprInner::Not(value) => expr_contains_hard_arith(value),
        ExprInner::Op(ExprOp::Mul, left, right) => {
            expr_contains_var(left) && expr_contains_var(right)
        }
        ExprInner::Op(ExprOp::UDiv | ExprOp::URem | ExprOp::SDiv | ExprOp::SRem, left, right) => {
            expr_contains_var(left) || expr_contains_var(right)
        }
        ExprInner::AddMod { left, right, modulus } | ExprInner::MulMod { left, right, modulus } => {
            expr_contains_var(left) || expr_contains_var(right) || expr_contains_var(modulus)
        }
        ExprInner::Op(_, left, right) => {
            expr_contains_hard_arith(left) || expr_contains_hard_arith(right)
        }
        ExprInner::Ite(cond, left, right) => {
            bool_contains_hard_arith(cond)
                || expr_contains_hard_arith(left)
                || expr_contains_hard_arith(right)
        }
    }
}

/// Returns whether the expression contains symbolic hash variables that local search should avoid.
pub(crate) fn expr_contains_symbolic_hash(expr: &Expr) -> bool {
    expr.visit(&mut |expr| {
        if matches!(expr.as_inner(), ExprInner::Hash { .. }) {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    })
    .is_break()
}

/// Returns whether the boolean expression contains symbolic hash variables.
pub(crate) fn bool_contains_symbolic_hash(expr: &BoolExpr) -> bool {
    expr.visit(&mut |expr| match expr.as_inner() {
        BoolExprInner::Eq(left, right) | BoolExprInner::Cmp(_, left, right)
            if expr_contains_symbolic_hash(left) || expr_contains_symbolic_hash(right) =>
        {
            ControlFlow::Break(())
        }
        _ => ControlFlow::Continue(()),
    })
    .is_break()
}

pub(crate) fn expr_contains_var(expr: &Expr) -> bool {
    expr.visit(&mut |expr| {
        if matches!(
            expr.as_inner(),
            ExprInner::Var(_) | ExprInner::Keccak { .. } | ExprInner::Hash { .. }
        ) {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    })
    .is_break()
}

/// Returns whether local hard-arithmetic search should run before asking the solver.
pub(crate) fn constraints_prefer_hard_arith_fallback_first(constraints: &[BoolExpr]) -> bool {
    if !constraints.iter().any(bool_contains_hard_arith)
        || constraints.iter().any(bool_contains_symbolic_hash)
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

pub(crate) fn hard_arith_fallback_model(constraints: &[BoolExpr]) -> Option<SymbolicModel> {
    if !constraints.iter().any(bool_contains_hard_arith)
        || constraints.iter().any(bool_contains_symbolic_hash)
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

/// Selects direct symbolic inputs for bounded fallback search.
pub(crate) fn fallback_search_vars(vars: SymbolicVars) -> Vec<Symbol> {
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

/// Returns deterministic local-search candidates for one symbolic variable.
pub(crate) fn fallback_candidates_for_var(
    var: &str,
    constraints: &[BoolExpr],
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

/// Holds immutable state for recursive hard-arithmetic fallback search.
struct FallbackSearch<'a> {
    constraints: &'a [BoolExpr],
    constraint_vars: &'a [SymbolicVars],
    searched_vars: &'a SymbolicVars,
    vars: &'a [Symbol],
    candidates: &'a [Vec<U256>],
}

impl FallbackSearch<'_> {
    /// Searches the bounded candidate product for a satisfying assignment.
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

/// Checks all constraints before returning a hard-arithmetic fallback witness.
pub(crate) fn fallback_model_satisfies_all_constraints(
    constraints: &[BoolExpr],
    model: &(impl SymbolicModelLookup + ?Sized),
) -> bool {
    constraints.iter().all(|constraint| constraint.eval(model).unwrap_or(false))
}

/// Checks constraints that depend only on already-assigned fallback variables.
pub(crate) fn fallback_partial_model_satisfies_known_constraints(
    constraints: &[BoolExpr],
    constraint_vars: &[SymbolicVars],
    searched_vars: &SymbolicVars,
    model: &SymbolicModel,
) -> bool {
    constraints.iter().zip(constraint_vars).all(|(constraint, vars)| {
        !vars.is_subset(searched_vars)
            || !vars.iter().all(|var| model.contains_name(*var))
            || constraint.eval(model).unwrap_or(false)
    })
}

/// Collects variables that local hard-arithmetic search can assign directly.
pub(crate) fn collect_bool_fallback_vars(expr: &BoolExpr, vars: &mut SymbolicVars) {
    match expr.as_inner() {
        BoolExprInner::Const(_) => {}
        BoolExprInner::Not(value) => collect_bool_fallback_vars(value, vars),
        BoolExprInner::And(values) => {
            for value in values.iter() {
                collect_bool_fallback_vars(value, vars);
            }
        }
        BoolExprInner::Eq(left, right) | BoolExprInner::Cmp(_, left, right) => {
            collect_expr_fallback_vars(left, vars);
            collect_expr_fallback_vars(right, vars);
        }
    }
}

/// Collects assignable variables from an expression, recursing into recomputable hashes.
pub(crate) fn collect_expr_fallback_vars(expr: &Expr, vars: &mut SymbolicVars) {
    match expr.as_inner() {
        ExprInner::Const(_) | ExprInner::GasLeft(_) | ExprInner::Hash { .. } => {}
        ExprInner::Var(var) => {
            vars.insert(*var);
        }
        ExprInner::Keccak { len, bytes, .. } => {
            collect_expr_fallback_vars(len, vars);
            for byte in bytes.iter() {
                collect_expr_fallback_vars(byte, vars);
            }
        }
        ExprInner::Not(value) => collect_expr_fallback_vars(value, vars),
        ExprInner::Op(_, left, right) => {
            collect_expr_fallback_vars(left, vars);
            collect_expr_fallback_vars(right, vars);
        }
        ExprInner::AddMod { left, right, modulus } | ExprInner::MulMod { left, right, modulus } => {
            collect_expr_fallback_vars(left, vars);
            collect_expr_fallback_vars(right, vars);
            collect_expr_fallback_vars(modulus, vars);
        }
        ExprInner::Ite(cond, left, right) => {
            collect_bool_fallback_vars(cond, vars);
            collect_expr_fallback_vars(left, vars);
            collect_expr_fallback_vars(right, vars);
        }
    }
}

#[cfg(test)]
pub(crate) fn fallback_single_var_model(constraints: &[BoolExpr]) -> Option<SymbolicModel> {
    let mut vars = SymbolicVars::default();
    let mut constants = HashSet::<U256>::default();
    for constraint in constraints {
        constraint.collect_vars(&mut vars);
        collect_bool_constants(constraint, &mut constants);
    }
    let mut constants = constants.into_iter().collect::<Vec<_>>();
    constants.sort_unstable();

    let var = if vars.len() == 1 { *vars.iter().next()? } else { return None };
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
        model.insert(var, candidate);
        if constraints.iter().all(|constraint| constraint.eval(&model).unwrap_or(false)) {
            return Some(model);
        }
    }

    None
}

pub(crate) fn push_fallback_candidate(
    candidates: &mut HashSet<U256>,
    candidate: U256,
    hints: MaskHints,
) {
    candidates.insert((candidate | hints.one) & !hints.zero);
}

pub(crate) fn collect_bool_constants(expr: &BoolExpr, constants: &mut HashSet<U256>) {
    match expr.as_inner() {
        BoolExprInner::Const(_) => {}
        BoolExprInner::Not(value) => collect_bool_constants(value, constants),
        BoolExprInner::And(values) => {
            for value in values.iter() {
                collect_bool_constants(value, constants);
            }
        }
        BoolExprInner::Eq(left, right) | BoolExprInner::Cmp(_, left, right) => {
            collect_expr_constants(left, constants);
            collect_expr_constants(right, constants);
        }
    }
}

pub(crate) fn collect_expr_constants(expr: &Expr, constants: &mut HashSet<U256>) {
    match expr.as_inner() {
        ExprInner::Const(value) => {
            constants.insert(*value);
        }
        ExprInner::Var(_)
        | ExprInner::GasLeft(_)
        | ExprInner::Keccak { .. }
        | ExprInner::Hash { .. } => {}
        ExprInner::Not(value) => collect_expr_constants(value, constants),
        ExprInner::Op(_, left, right) => {
            collect_expr_constants(left, constants);
            collect_expr_constants(right, constants);
        }
        ExprInner::AddMod { left, right, modulus } | ExprInner::MulMod { left, right, modulus } => {
            collect_expr_constants(left, constants);
            collect_expr_constants(right, constants);
            collect_expr_constants(modulus, constants);
        }
        ExprInner::Ite(cond, left, right) => {
            collect_bool_constants(cond, constants);
            collect_expr_constants(left, constants);
            collect_expr_constants(right, constants);
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct MaskHints {
    pub(crate) one: U256,
    pub(crate) zero: U256,
}

impl MaskHints {
    pub(crate) fn for_var(var: &str, constraints: &[BoolExpr]) -> Self {
        let mut hints = Self::default();
        for constraint in constraints {
            hints.apply_bool(var, constraint, false);
        }
        hints
    }

    pub(crate) fn apply_bool(&mut self, var: &str, expr: &BoolExpr, inverted: bool) {
        match expr.as_inner() {
            BoolExprInner::Const(_) => {}
            BoolExprInner::Not(value) => self.apply_bool(var, value, !inverted),
            BoolExprInner::And(values) if !inverted => {
                for value in values.iter() {
                    self.apply_bool(var, value, false);
                }
            }
            BoolExprInner::Eq(left, right) => self.apply_equality(var, left, right, inverted),
            BoolExprInner::Cmp(_, _, _) | BoolExprInner::And(_) => {}
        }
    }

    pub(crate) fn apply_equality(&mut self, var: &str, left: &Expr, right: &Expr, inverted: bool) {
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

pub(crate) fn zero_mask_equality(var: &str, masked: &Expr, zero: &Expr) -> Option<U256> {
    if !zero.as_const().is_some_and(|value| value.is_zero()) {
        return None;
    }
    match masked.as_inner() {
        ExprInner::Op(ExprOp::And, left, right) => match (left.as_inner(), right.as_inner()) {
            (ExprInner::Var(name), ExprInner::Const(mask))
            | (ExprInner::Const(mask), ExprInner::Var(name))
                if name.as_str() == var =>
            {
                Some(*mask)
            }
            _ => None,
        },
        _ => None,
    }
}
