use super::*;

/// Minimal solver backend interface used by the symbolic executor.
///
/// Implementations are responsible for translating accumulated symbolic constraints
/// into solver queries, enforcing query budgets, and extracting concrete model values
/// for counterexample replay. The trait is intentionally small so alternate SMT
/// backends can be added without changing the executor entrypoints.
pub(crate) trait SymbolicSolver {
    /// Returns solver counters collected by this backend.
    fn stats(&self) -> SymbolicStats;

    /// Verifies that the configured solver can be invoked before exploration starts.
    ///
    /// Backends should keep this check lightweight and return a [`SymbolicError`] with
    /// a stable stop reason when the solver executable or service is unavailable.
    fn check_available(&self) -> Result<(), SymbolicError>;

    /// Returns whether the supplied path constraints are satisfiable.
    ///
    /// Implementations should count this as one solver query and map solver `unknown`
    /// or timeout responses into [`SymbolicError::SolverUnknown`] or
    /// [`SymbolicError::Solver`], as appropriate.
    fn is_sat(&mut self, constraints: &[BoolExpr]) -> Result<bool, SymbolicError>;

    /// Returns a concrete model for all symbolic variables constrained by the path.
    ///
    /// The executor uses the returned variable assignments to materialize ABI
    /// arguments, calldata, and invariant sequences for concrete replay.
    fn model(&mut self, constraints: &[BoolExpr]) -> Result<BTreeMap<String, U256>, SymbolicError>;
}

pub(crate) struct Z3SubprocessSolver {
    pub(crate) command: String,
    pub(crate) timeout: Option<u32>,
    pub(crate) max_queries: usize,
    pub(crate) queries: usize,
    pub(crate) dump_smt: bool,
}

impl Z3SubprocessSolver {
    /// Constructs a new instance.
    pub(crate) const fn new(
        command: String,
        timeout: Option<u32>,
        max_queries: usize,
        dump_smt: bool,
    ) -> Self {
        Self { command, timeout, max_queries, queries: 0, dump_smt }
    }
}

impl SymbolicSolver for Z3SubprocessSolver {
    /// Implements the `stats` solver helper.
    fn stats(&self) -> SymbolicStats {
        SymbolicStats { paths: 0, solver_queries: self.queries }
    }

    /// Validates the `check_available` solver helper.
    fn check_available(&self) -> Result<(), SymbolicError> {
        let output = Command::new(&self.command).arg("--version").output().map_err(|err| {
            SymbolicError::Solver(format!("failed to execute `{}`: {err}", self.command))
        })?;
        if output.status.success() {
            Ok(())
        } else {
            Err(SymbolicError::Solver(format!("`{}` is not a usable z3 executable", self.command)))
        }
    }

    /// Returns whether `is_sat` holds.
    fn is_sat(&mut self, constraints: &[BoolExpr]) -> Result<bool, SymbolicError> {
        self.reserve_query()?;
        self.queries += 1;
        if constraints_prefer_fallback_first(constraints)
            && fallback_single_var_model(constraints).is_some()
        {
            return Ok(true);
        }
        let output = self.query(constraints, false)?;
        match output.lines().next().unwrap_or_default().trim() {
            "sat" => Ok(true),
            "unsat" => Ok(false),
            "unknown" => fallback_single_var_model(constraints)
                .map(|_| true)
                .ok_or(SymbolicError::SolverUnknown),
            other => Err(SymbolicError::Solver(format!("unexpected z3 response `{other}`"))),
        }
    }

    /// Implements the `model` solver helper.
    fn model(&mut self, constraints: &[BoolExpr]) -> Result<BTreeMap<String, U256>, SymbolicError> {
        self.reserve_query()?;
        self.queries += 1;
        if constraints_prefer_fallback_first(constraints)
            && let Some(model) = fallback_single_var_model(constraints)
        {
            return Ok(model);
        }
        let output = self.query(constraints, true)?;
        let mut lines = output.lines();
        match lines.next().unwrap_or_default().trim() {
            "sat" => parse_model(&output),
            "unsat" => Err(SymbolicError::Solver("counterexample path became unsat".to_string())),
            "unknown" => fallback_single_var_model(constraints).ok_or(SymbolicError::SolverUnknown),
            other => Err(SymbolicError::Solver(format!("unexpected z3 response `{other}`"))),
        }
    }
}

impl Z3SubprocessSolver {
    /// Validates the `reserve_query` solver helper.
    pub(crate) const fn reserve_query(&self) -> Result<(), SymbolicError> {
        if self.queries >= self.max_queries {
            return Err(SymbolicError::SolverQueryLimit(self.max_queries));
        }
        Ok(())
    }

    /// Implements the `query` solver helper.
    pub(crate) fn query(
        &self,
        constraints: &[BoolExpr],
        model: bool,
    ) -> Result<String, SymbolicError> {
        let mut vars = BTreeSet::new();
        for constraint in constraints {
            constraint.collect_vars(&mut vars);
        }

        let mut smt = String::new();
        smt.push_str("(set-logic QF_BV)\n");
        if let Some(timeout) = self.timeout {
            let _ = writeln!(smt, "(set-option :timeout {})", timeout.saturating_mul(1000));
        }
        for var in vars {
            let _ = writeln!(smt, "(declare-fun {var} () (_ BitVec 256))");
        }
        for constraint in constraints {
            let _ = writeln!(smt, "(assert {})", constraint.smt());
        }
        smt.push_str("(check-sat)\n");
        if model {
            smt.push_str("(get-model)\n");
        }
        if self.dump_smt {
            let mut stderr = std::io::stderr().lock();
            let _ = writeln!(stderr, "--- symbolic SMT query {} ---\n{smt}", self.queries);
        }

        let mut child = Command::new(&self.command)
            .args(["-in", "-smt2"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| {
                SymbolicError::Solver(format!("failed to spawn `{}`: {err}", self.command))
            })?;
        child
            .stdin
            .as_mut()
            .expect("stdin configured")
            .write_all(smt.as_bytes())
            .map_err(|err| SymbolicError::Solver(format!("failed to write z3 query: {err}")))?;
        let output = child
            .wait_with_output()
            .map_err(|err| SymbolicError::Solver(format!("failed to read z3 output: {err}")))?;
        if !output.status.success() {
            return Err(SymbolicError::Solver(String::from_utf8_lossy(&output.stderr).to_string()));
        }
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}

/// Implements the `constraints_prefer_fallback_first` solver helper.
pub(crate) fn constraints_prefer_fallback_first(constraints: &[BoolExpr]) -> bool {
    constraints.iter().any(bool_contains_symbolic_mul)
}

/// Returns the `bool_contains_symbolic_mul` solver helper result.
pub(crate) fn bool_contains_symbolic_mul(expr: &BoolExpr) -> bool {
    match expr {
        BoolExpr::Const(_) => false,
        BoolExpr::Not(value) => bool_contains_symbolic_mul(value),
        BoolExpr::And(values) => values.iter().any(bool_contains_symbolic_mul),
        BoolExpr::Eq(left, right) | BoolExpr::Cmp(_, left, right) => {
            expr_contains_symbolic_mul(left) || expr_contains_symbolic_mul(right)
        }
    }
}

/// Returns the `expr_contains_symbolic_mul` solver helper result.
pub(crate) fn expr_contains_symbolic_mul(expr: &Expr) -> bool {
    match expr {
        Expr::Const(_) | Expr::Var(_) | Expr::Keccak { .. } | Expr::Hash { .. } => false,
        Expr::Not(value) => expr_contains_symbolic_mul(value),
        Expr::Op(ExprOp::Mul, left, right) => expr_contains_var(left) && expr_contains_var(right),
        Expr::Op(_, left, right) => {
            expr_contains_symbolic_mul(left) || expr_contains_symbolic_mul(right)
        }
        Expr::Ite(cond, left, right) => {
            bool_contains_symbolic_mul(cond)
                || expr_contains_symbolic_mul(left)
                || expr_contains_symbolic_mul(right)
        }
    }
}

/// Returns the `expr_contains_var` solver helper result.
pub(crate) fn expr_contains_var(expr: &Expr) -> bool {
    match expr {
        Expr::Const(_) => false,
        Expr::Var(_) | Expr::Keccak { .. } | Expr::Hash { .. } => true,
        Expr::Not(value) => expr_contains_var(value),
        Expr::Op(_, left, right) => expr_contains_var(left) || expr_contains_var(right),
        Expr::Ite(cond, left, right) => {
            bool_contains_var(cond) || expr_contains_var(left) || expr_contains_var(right)
        }
    }
}

/// Returns the `bool_contains_var` solver helper result.
pub(crate) fn bool_contains_var(expr: &BoolExpr) -> bool {
    match expr {
        BoolExpr::Const(_) => false,
        BoolExpr::Not(value) => bool_contains_var(value),
        BoolExpr::And(values) => values.iter().any(bool_contains_var),
        BoolExpr::Eq(left, right) | BoolExpr::Cmp(_, left, right) => {
            expr_contains_var(left) || expr_contains_var(right)
        }
    }
}

/// Implements the `fallback_single_var_model` solver helper.
pub(crate) fn fallback_single_var_model(
    constraints: &[BoolExpr],
) -> Option<BTreeMap<String, U256>> {
    let mut vars = BTreeSet::new();
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
        let model = BTreeMap::from([(var.clone(), candidate)]);
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
            for value in values {
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
        Expr::Var(_) | Expr::Keccak { .. } | Expr::Hash { .. } => {}
        Expr::Not(value) => collect_expr_constants(value, constants),
        Expr::Op(_, left, right) => {
            collect_expr_constants(left, constants);
            collect_expr_constants(right, constants);
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
                for value in values {
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
                if name == var =>
            {
                Some(*mask)
            }
            _ => None,
        },
        _ => None,
    }
}

/// Returns the `parse_model` solver helper result.
pub(crate) fn parse_model(output: &str) -> Result<BTreeMap<String, U256>, SymbolicError> {
    let mut values = BTreeMap::new();
    let mut tokens = output
        .split(|c: char| c.is_whitespace() || matches!(c, '(' | ')'))
        .filter(|token| !token.is_empty());
    while let Some(token) = tokens.next() {
        if token == "define-fun" {
            let Some(name) = tokens.next() else { continue };
            while let Some(value) = tokens.next() {
                if let Some(hex) = value.strip_prefix("#x") {
                    let mut bytes = [0u8; 32];
                    let decoded = alloy_primitives::hex::decode(hex).map_err(|err| {
                        SymbolicError::Solver(format!("invalid z3 hex model value: {err}"))
                    })?;
                    let start = 32usize.saturating_sub(decoded.len());
                    bytes[start..start + decoded.len()].copy_from_slice(&decoded);
                    values.insert(name.to_string(), U256::from_be_bytes(bytes));
                    break;
                }
                if value == "_"
                    && let Some(bv) = tokens.next().and_then(|v| v.strip_prefix("bv"))
                {
                    let parsed = U256::from_str_radix(bv, 10).map_err(|err| {
                        SymbolicError::Solver(format!("invalid z3 decimal model value: {err}"))
                    })?;
                    values.insert(name.to_string(), parsed);
                    break;
                }
            }
        }
    }
    Ok(values)
}
