use super::*;

/// Returns the `bool_contains_hard_arith` solver helper result.
pub(crate) fn bool_contains_hard_arith(expr: &BoolExpr) -> bool {
    match expr {
        BoolExpr::Const(_) => false,
        BoolExpr::Not(value) => bool_contains_hard_arith(value),
        BoolExpr::And(values) => values.iter().any(bool_contains_hard_arith),
        BoolExpr::Eq(left, right) | BoolExpr::Cmp(_, left, right) => {
            expr_contains_hard_arith(left) || expr_contains_hard_arith(right)
        }
    }
}

/// Returns the `expr_contains_hard_arith` solver helper result.
pub(crate) fn expr_contains_hard_arith(expr: &Expr) -> bool {
    match expr {
        Expr::Const(_) | Expr::Var(_) | Expr::GasLeft(_) | Expr::Keccak(_) | Expr::Hash(_) => false,
        Expr::Not(value) => expr_contains_hard_arith(value),
        Expr::Op(ExprOp::Mul, left, right) => expr_contains_var(left) && expr_contains_var(right),
        Expr::Op(ExprOp::UDiv | ExprOp::URem | ExprOp::SDiv | ExprOp::SRem, left, right) => {
            expr_contains_var(left) || expr_contains_var(right)
        }
        Expr::AddMod { left, right, modulus } | Expr::MulMod { left, right, modulus } => {
            expr_contains_var(left) || expr_contains_var(right) || expr_contains_var(modulus)
        }
        Expr::Op(_, left, right) => {
            expr_contains_hard_arith(left) || expr_contains_hard_arith(right)
        }
        Expr::Ite(cond, left, right) => {
            bool_contains_hard_arith(cond)
                || expr_contains_hard_arith(left)
                || expr_contains_hard_arith(right)
        }
    }
}

/// Returns whether the expression contains symbolic hash variables that local search should avoid.
pub(crate) fn expr_contains_symbolic_hash(expr: &Expr) -> bool {
    let mut contains = false;
    expr.visit(&mut |expr| contains |= matches!(expr, Expr::Hash(_)));
    contains
}

/// Returns whether the boolean expression contains symbolic hash variables.
pub(crate) fn bool_contains_symbolic_hash(expr: &BoolExpr) -> bool {
    let mut contains = false;
    expr.visit(&mut |expr| match expr {
        BoolExpr::Eq(left, right) | BoolExpr::Cmp(_, left, right) => {
            contains |= expr_contains_symbolic_hash(left) || expr_contains_symbolic_hash(right);
        }
        BoolExpr::Const(_) | BoolExpr::Not(_) | BoolExpr::And(_) => {}
    });
    contains
}

/// Returns the `expr_contains_var` solver helper result.
pub(crate) fn expr_contains_var(expr: &Expr) -> bool {
    let mut contains = false;
    expr.visit(&mut |expr| {
        contains |= matches!(expr, Expr::Var(_) | Expr::Keccak(_) | Expr::Hash(_))
    });
    contains
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

/// Implements the `hard_arith_fallback_model` solver helper.
pub(crate) fn hard_arith_fallback_model(
    constraints: &[BoolExpr],
) -> Option<BTreeMap<String, U256>> {
    if !constraints.iter().any(bool_contains_hard_arith)
        || constraints.iter().any(bool_contains_symbolic_hash)
    {
        return None;
    }

    let mut vars = SymbolicVars::default();
    let mut constants = BTreeSet::new();
    for constraint in constraints {
        collect_bool_fallback_vars(constraint, &mut vars);
        collect_bool_constants(constraint, &mut constants);
    }
    let vars = fallback_search_vars(vars);
    if vars.is_empty() || vars.len() > HARD_ARITH_FALLBACK_MAX_VARS {
        return None;
    }

    let candidates = vars
        .iter()
        .map(|var| fallback_candidates_for_var(var, constraints, &constants))
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
    let mut model = BTreeMap::new();
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
pub(crate) fn fallback_search_vars(vars: SymbolicVars) -> Vec<Arc<str>> {
    if vars.len() <= HARD_ARITH_FALLBACK_MAX_VARS {
        return vars.into_iter().collect();
    }

    vars.into_iter()
        .filter(|var| {
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
    constants: &BTreeSet<U256>,
) -> Option<Vec<U256>> {
    let hints = MaskHints::for_var(var, constraints);
    if (hints.one & hints.zero) != U256::ZERO {
        return None;
    }

    let mut candidates = BTreeSet::new();
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

    Some(candidates.into_iter().take(HARD_ARITH_FALLBACK_MAX_CANDIDATES_PER_VAR).collect())
}

/// Holds immutable state for recursive hard-arithmetic fallback search.
struct FallbackSearch<'a> {
    constraints: &'a [BoolExpr],
    constraint_vars: &'a [SymbolicVars],
    searched_vars: &'a SymbolicVars,
    vars: &'a [Arc<str>],
    candidates: &'a [Vec<U256>],
}

impl FallbackSearch<'_> {
    /// Searches the bounded candidate product for a satisfying assignment.
    fn model(
        &self,
        index: usize,
        model: &mut BTreeMap<String, U256>,
        assignments: &mut usize,
    ) -> Option<BTreeMap<String, U256>> {
        if index == self.vars.len() {
            *assignments += 1;
            if *assignments > HARD_ARITH_FALLBACK_MAX_ASSIGNMENTS {
                return None;
            }
            return fallback_model_satisfies_all_constraints(self.constraints, model)
                .then(|| model.clone());
        }

        for candidate in &self.candidates[index] {
            model.insert(self.vars[index].to_string(), *candidate);
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
        model.remove(self.vars[index].as_ref());
        None
    }
}

/// Checks all constraints before returning a hard-arithmetic fallback witness.
pub(crate) fn fallback_model_satisfies_all_constraints(
    constraints: &[BoolExpr],
    model: &BTreeMap<String, U256>,
) -> bool {
    constraints.iter().all(|constraint| eval_bool_expr(constraint, model).unwrap_or(false))
}

/// Checks constraints that depend only on already-assigned fallback variables.
pub(crate) fn fallback_partial_model_satisfies_known_constraints(
    constraints: &[BoolExpr],
    constraint_vars: &[SymbolicVars],
    searched_vars: &SymbolicVars,
    model: &BTreeMap<String, U256>,
) -> bool {
    constraints.iter().zip(constraint_vars).all(|(constraint, vars)| {
        !vars.is_subset(searched_vars)
            || !vars.iter().all(|var| model.contains_key(var.as_ref()))
            || eval_bool_expr(constraint, model).unwrap_or(false)
    })
}

/// Collects variables that local hard-arithmetic search can assign directly.
pub(crate) fn collect_bool_fallback_vars(expr: &BoolExpr, vars: &mut SymbolicVars) {
    match expr {
        BoolExpr::Const(_) => {}
        BoolExpr::Not(value) => collect_bool_fallback_vars(value, vars),
        BoolExpr::And(values) => {
            for value in values.iter() {
                collect_bool_fallback_vars(value, vars);
            }
        }
        BoolExpr::Eq(left, right) | BoolExpr::Cmp(_, left, right) => {
            collect_expr_fallback_vars(left, vars);
            collect_expr_fallback_vars(right, vars);
        }
    }
}

/// Collects assignable variables from an expression, recursing into recomputable hashes.
pub(crate) fn collect_expr_fallback_vars(expr: &Expr, vars: &mut SymbolicVars) {
    match expr {
        Expr::Const(_) | Expr::GasLeft(_) | Expr::Hash(_) => {}
        Expr::Var(var) => {
            vars.insert(var.clone());
        }
        Expr::Keccak(hash) => {
            collect_expr_fallback_vars(hash.len(), vars);
            for byte in hash.bytes() {
                collect_expr_fallback_vars(byte, vars);
            }
        }
        Expr::Not(value) => collect_expr_fallback_vars(value, vars),
        Expr::Op(_, left, right) => {
            collect_expr_fallback_vars(left, vars);
            collect_expr_fallback_vars(right, vars);
        }
        Expr::AddMod { left, right, modulus } | Expr::MulMod { left, right, modulus } => {
            collect_expr_fallback_vars(left, vars);
            collect_expr_fallback_vars(right, vars);
            collect_expr_fallback_vars(modulus, vars);
        }
        Expr::Ite(cond, left, right) => {
            collect_bool_fallback_vars(cond, vars);
            collect_expr_fallback_vars(left, vars);
            collect_expr_fallback_vars(right, vars);
        }
    }
}

/// Implements the `fallback_single_var_model` solver helper.
#[cfg(test)]
pub(crate) fn fallback_single_var_model(
    constraints: &[BoolExpr],
) -> Option<BTreeMap<String, U256>> {
    let mut vars = SymbolicVars::default();
    let mut constants = BTreeSet::new();
    for constraint in constraints {
        constraint.collect_vars(&mut vars);
        collect_bool_constants(constraint, &mut constants);
    }

    let var = if vars.len() == 1 { vars.iter().next()?.clone() } else { return None };
    let hints = MaskHints::for_var(&var, constraints);
    if (hints.one & hints.zero) != U256::ZERO {
        return None;
    }

    let mut candidates = BTreeSet::new();
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

    for candidate in candidates {
        let model = BTreeMap::from([(var.to_string(), candidate)]);
        if constraints.iter().all(|constraint| eval_bool_expr(constraint, &model).unwrap_or(false))
        {
            return Some(model);
        }
    }

    None
}

/// Applies the `push_fallback_candidate` solver helper.
pub(crate) fn push_fallback_candidate(
    candidates: &mut BTreeSet<U256>,
    candidate: U256,
    hints: MaskHints,
) {
    candidates.insert((candidate | hints.one) & !hints.zero);
}

/// Implements the `collect_bool_constants` solver helper.
pub(crate) fn collect_bool_constants(expr: &BoolExpr, constants: &mut BTreeSet<U256>) {
    match expr {
        BoolExpr::Const(_) => {}
        BoolExpr::Not(value) => collect_bool_constants(value, constants),
        BoolExpr::And(values) => {
            for value in values.iter() {
                collect_bool_constants(value, constants);
            }
        }
        BoolExpr::Eq(left, right) | BoolExpr::Cmp(_, left, right) => {
            collect_expr_constants(left, constants);
            collect_expr_constants(right, constants);
        }
    }
}

/// Implements the `collect_expr_constants` solver helper.
pub(crate) fn collect_expr_constants(expr: &Expr, constants: &mut BTreeSet<U256>) {
    match expr {
        Expr::Const(value) => {
            constants.insert(*value);
        }
        Expr::Var(_) | Expr::GasLeft(_) | Expr::Keccak(_) | Expr::Hash(_) => {}
        Expr::Not(value) => collect_expr_constants(value, constants),
        Expr::Op(_, left, right) => {
            collect_expr_constants(left, constants);
            collect_expr_constants(right, constants);
        }
        Expr::AddMod { left, right, modulus } | Expr::MulMod { left, right, modulus } => {
            collect_expr_constants(left, constants);
            collect_expr_constants(right, constants);
            collect_expr_constants(modulus, constants);
        }
        Expr::Ite(cond, left, right) => {
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
    /// Implements the `for_var` solver helper.
    pub(crate) fn for_var(var: &str, constraints: &[BoolExpr]) -> Self {
        let mut hints = Self::default();
        for constraint in constraints {
            hints.apply_bool(var, constraint, false);
        }
        hints
    }

    /// Applies the `apply_bool` solver helper.
    pub(crate) fn apply_bool(&mut self, var: &str, expr: &BoolExpr, inverted: bool) {
        match expr {
            BoolExpr::Const(_) => {}
            BoolExpr::Not(value) => self.apply_bool(var, value, !inverted),
            BoolExpr::And(values) if !inverted => {
                for value in values.iter() {
                    self.apply_bool(var, value, false);
                }
            }
            BoolExpr::Eq(left, right) => self.apply_equality(var, left, right, inverted),
            BoolExpr::Cmp(_, _, _) | BoolExpr::And(_) => {}
        }
    }

    /// Applies the `apply_equality` solver helper.
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

/// Implements the `zero_mask_equality` solver helper.
pub(crate) fn zero_mask_equality(var: &str, masked: &Expr, zero: &Expr) -> Option<U256> {
    if !matches!(zero, Expr::Const(value) if value.is_zero()) {
        return None;
    }
    match masked {
        Expr::Op(ExprOp::And, left, right) => match (left.as_ref(), right.as_ref()) {
            (Expr::Var(name), Expr::Const(mask)) | (Expr::Const(mask), Expr::Var(name))
                if name.as_ref() == var =>
            {
                Some(*mask)
            }
            _ => None,
        },
        _ => None,
    }
}
