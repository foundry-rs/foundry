use super::*;

mod hard_arith_fallback;
mod monotonic_product;
mod normalize;

use hard_arith_fallback::constraints_prefer_hard_arith_fallback_first;
pub(crate) use hard_arith_fallback::hard_arith_fallback_model;
#[cfg(test)]
pub(crate) use hard_arith_fallback::{expr_contains_hard_arith, fallback_single_var_model};
#[cfg(test)]
pub(crate) use monotonic_product::product_monotonic_unsat;
use monotonic_product::product_monotonic_unsat_normalized;
pub(crate) use normalize::normalize_constraints_for_solver;
#[cfg(test)]
pub(crate) use normalize::{normalize_bool_for_solver, normalize_expr_for_solver};

/// Errors that arise when parsing or constructing solver commands from configuration.
#[derive(Debug, thiserror::Error)]
pub(crate) enum SolverConfigError {
    /// The command string parsed to an empty argv.
    #[error("symbolic solver command is empty")]
    EmptyCommand,
    /// The command string contains an unterminated quote character.
    #[error("unterminated {0} quote in symbolic solver command")]
    UnterminatedQuote(char),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum SolverOutcome {
    Cancelled,
    Error,
    NotStarted,
    SatAfterWinner,
    SatInvalid,
    SatValid,
    TimeoutOrUnknown,
    Unknown,
    UnknownAfterWinner,
    Unsat,
    UnsatAfterWinner,
    Unexpected,
}

impl SolverOutcome {
    /// Returns the diagnostic label for this solver outcome.
    const fn as_str(self) -> &'static str {
        match self {
            Self::Cancelled => "cancelled",
            Self::Error => "error",
            Self::NotStarted => "not-started",
            Self::SatAfterWinner => "sat-after-winner",
            Self::SatInvalid => "sat-invalid",
            Self::SatValid => "sat-valid",
            Self::TimeoutOrUnknown => "timeout-or-unknown",
            Self::Unknown => "unknown",
            Self::UnknownAfterWinner => "unknown-after-winner",
            Self::Unsat => "unsat",
            Self::UnsatAfterWinner => "unsat-after-winner",
            Self::Unexpected => "unexpected",
        }
    }
}

impl fmt::Display for SolverOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

pub(crate) type QueryObserver = Box<dyn Fn(usize) + Send + Sync + 'static>;

/// Minimal solver backend interface used by the symbolic executor.
///
/// Implementations are responsible for translating accumulated symbolic constraints
/// into solver queries, enforcing query budgets, and extracting concrete model values
/// for counterexample replay. The trait is intentionally small so alternate SMT
/// backends can be added without changing the executor entrypoints.
pub(crate) trait SymbolicSolver {
    /// Returns solver counters collected by this backend.
    fn stats(&self) -> SymbolicStats;

    /// Registers a callback invoked after each logical solver query is reserved.
    fn set_query_observer(&mut self, observer: Option<QueryObserver>);

    /// Returns aggregate staged-portfolio diagnostics collected by this backend.
    fn portfolio_diagnostics(&self) -> Option<&PortfolioDiagnostics>;

    /// Captures verbose diagnostics for later rendering instead of writing them live.
    fn capture_diagnostics(&mut self);

    /// Takes any captured verbose diagnostics collected by this backend.
    fn take_diagnostics(&mut self) -> Option<String>;

    /// Returns the number of satisfiable witnesses produced by local hard-arithmetic search.
    fn heuristic_witnesses(&self) -> usize {
        0
    }

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

    /// Returns branch satisfiability, allowing branch-only hard-arithmetic shortcuts.
    fn is_sat_branch(&mut self, constraints: &[BoolExpr]) -> Result<bool, SymbolicError> {
        self.is_sat(constraints)
    }

    /// Returns a concrete model for all symbolic variables constrained by the path.
    ///
    /// The executor uses the returned variable assignments to materialize ABI
    /// arguments, calldata, and invariant sequences for concrete replay.
    fn model(&mut self, constraints: &[BoolExpr]) -> Result<BTreeMap<String, U256>, SymbolicError>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SolverCommand {
    pub(crate) program: String,
    pub(crate) args: Vec<String>,
    pub(crate) display: String,
    pub(crate) smt_timeout: bool,
}

impl SolverCommand {
    /// Constructs a solver command from a program plus arguments.
    pub(crate) fn new(parts: Vec<String>, smt_timeout: bool) -> Result<Self, SolverConfigError> {
        let mut parts = parts.into_iter();
        let Some(program) = parts.next().filter(|part| !part.is_empty()) else {
            return Err(SolverConfigError::EmptyCommand);
        };
        let args = parts.collect::<Vec<_>>();
        let display = std::iter::once(program.as_str())
            .chain(args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ");
        Ok(Self { program, args, display, smt_timeout })
    }
}

pub(crate) struct SmtLibSubprocessSolver {
    pub(crate) commands: Result<Vec<SolverCommand>, SolverConfigError>,
    pub(crate) timeout: Option<u32>,
    pub(crate) max_queries: usize,
    pub(crate) queries: usize,
    query_observer: Option<QueryObserver>,
    pub(crate) dump_smt: bool,
    portfolio_scheduler: PortfolioScheduler,
    portfolio_diagnostics: PortfolioDiagnostics,
    captured_diagnostics: Option<String>,
    heuristic_witnesses: usize,
    sat_cache: HashMap<Vec<BoolExpr>, bool>,
    model_cache: HashMap<Vec<BoolExpr>, BTreeMap<String, U256>>,
    sat_queries: usize,
    model_queries: usize,
    sat_cache_hits: usize,
    model_cache_hits: usize,
    smt_queries: usize,
    solver_time: Duration,
}

impl SmtLibSubprocessSolver {
    /// Constructs a new instance.
    pub(crate) fn new(
        commands: Result<Vec<SolverCommand>, SolverConfigError>,
        timeout: Option<u32>,
        max_queries: usize,
        dump_smt: bool,
    ) -> Self {
        Self {
            commands,
            timeout,
            max_queries,
            queries: 0,
            query_observer: None,
            dump_smt,
            portfolio_scheduler: PortfolioScheduler::default(),
            portfolio_diagnostics: PortfolioDiagnostics::default(),
            captured_diagnostics: None,
            heuristic_witnesses: 0,
            sat_cache: HashMap::default(),
            model_cache: HashMap::default(),
            sat_queries: 0,
            model_queries: 0,
            sat_cache_hits: 0,
            model_cache_hits: 0,
            smt_queries: 0,
            solver_time: Duration::ZERO,
        }
    }

    /// Constructs a subprocess solver from Foundry symbolic config.
    pub(crate) fn from_config(config: &SymbolicConfig) -> Self {
        Self::new(
            solver_commands_for_config(config),
            config.timeout,
            config.max_solver_queries as usize,
            config.dump_smt,
        )
    }
}

impl SymbolicSolver for SmtLibSubprocessSolver {
    /// Implements the `stats` solver helper.
    fn stats(&self) -> SymbolicStats {
        SymbolicStats {
            paths: 0,
            solver_queries: self.queries,
            smt_queries: self.smt_queries,
            sat_queries: self.sat_queries,
            model_queries: self.model_queries,
            sat_cache_hits: self.sat_cache_hits,
            model_cache_hits: self.model_cache_hits,
            heuristic_witnesses: self.heuristic_witnesses,
            solver_time_ms: self.solver_time.as_millis().try_into().unwrap_or(u64::MAX),
        }
    }

    /// Registers a live query observer for progress rendering.
    fn set_query_observer(&mut self, observer: Option<QueryObserver>) {
        self.query_observer = observer;
    }

    /// Returns staged-portfolio diagnostics collected by this solver.
    fn portfolio_diagnostics(&self) -> Option<&PortfolioDiagnostics> {
        (!self.portfolio_diagnostics.is_empty()).then_some(&self.portfolio_diagnostics)
    }

    /// Enables deferred diagnostic rendering for verbose symbolic solver output.
    fn capture_diagnostics(&mut self) {
        self.captured_diagnostics.get_or_insert_with(String::new);
    }

    /// Returns and clears deferred diagnostic rendering output.
    fn take_diagnostics(&mut self) -> Option<String> {
        self.captured_diagnostics.take().filter(|diagnostics| !diagnostics.is_empty())
    }

    /// Returns how many validated local hard-arithmetic witnesses this solver used.
    fn heuristic_witnesses(&self) -> usize {
        self.heuristic_witnesses
    }

    /// Validates the `check_available` solver helper.
    fn check_available(&self) -> Result<(), SymbolicError> {
        let commands = self.commands()?;
        let mut errors = Vec::new();
        for command in commands {
            let output = match Command::new(&command.program).arg("--version").output() {
                Ok(output) => output,
                Err(err) => {
                    errors.push(format!("failed to execute `{}`: {err}", command.program));
                    continue;
                }
            };
            if output.status.success() {
                return Ok(());
            }
            errors.push(format!("`{}` is not a usable SMT solver executable", command.program));
        }
        Err(SymbolicError::Solver(errors.join("; ")))
    }

    /// Returns whether `is_sat` holds.
    fn is_sat(&mut self, constraints: &[BoolExpr]) -> Result<bool, SymbolicError> {
        self.is_sat_inner(constraints, false)
    }

    /// Returns whether a branch is feasible.
    fn is_sat_branch(&mut self, constraints: &[BoolExpr]) -> Result<bool, SymbolicError> {
        self.is_sat_inner(constraints, true)
    }

    /// Implements the `model` solver helper.
    fn model(&mut self, constraints: &[BoolExpr]) -> Result<BTreeMap<String, U256>, SymbolicError> {
        self.model_queries += 1;
        if constraints.iter().any(bool_contains_gasleft) {
            return Err(SymbolicError::Unsupported("GAS/gasleft() not modeled"));
        }
        let smt_constraints = normalize_constraints_for_solver(constraints);
        let cache_key = constraint_cache_key(&smt_constraints);

        if self.sat_cache.get(&cache_key) == Some(&false) {
            self.model_cache.remove(&cache_key);
            trace!("model: normalized sat cache says unsat");
            return Err(SymbolicError::Solver("counterexample path became unsat".to_string()));
        }
        if self.has_cached_unsat_subset(&cache_key) {
            self.cache_sat_result(cache_key.clone(), false);
            self.model_cache.remove(&cache_key);
            trace!("model: normalized unsat subset cache hit");
            return Err(SymbolicError::Solver("counterexample path became unsat".to_string()));
        }

        if let Some(model) = self.model_cache.get(&cache_key) {
            if model_satisfies_constraints(model, constraints) {
                let model = model.clone();
                self.model_cache_hits += 1;
                trace!("model: normalized cache hit");
                self.cache_sat_result(cache_key.clone(), true);
                return Ok(model);
            }
            trace!("model: normalized cache hit failed validation");
        }
        if self.model_cache.remove(&cache_key).is_some() {
            self.sat_cache.remove(&cache_key);
        }

        self.reserve_query()?;
        self.record_query();
        let _span = trace_span!(
            "solver_query",
            query_id = self.queries,
            constraint_count = constraints.len(),
            kind = "model"
        )
        .entered();
        trace!(query_id = self.queries, constraint_count = constraints.len(), "solver model");
        if constraints_prefer_hard_arith_fallback_first(&smt_constraints)
            && let Some(model) = validated_hard_arith_fallback_model(&smt_constraints, constraints)
        {
            self.heuristic_witnesses += 1;
            trace!("model: validated hard arithmetic fallback model before solver");
            self.cache_sat_result(cache_key.clone(), true);
            self.cache_model_result(cache_key, model.clone());
            return Ok(model);
        }
        let output = match self.query_normalized(&smt_constraints, true, constraints) {
            Ok(output) => output,
            Err(SymbolicError::SolverUnknown) => {
                if let Some(model) =
                    validated_hard_arith_fallback_model(&smt_constraints, constraints)
                {
                    self.heuristic_witnesses += 1;
                    trace!("model: validated hard arithmetic fallback model after solver unknown");
                    self.cache_sat_result(cache_key.clone(), true);
                    self.cache_model_result(cache_key, model.clone());
                    return Ok(model);
                }
                return Err(SymbolicError::SolverUnknown);
            }
            Err(err) => return Err(err),
        };
        let mut lines = output.lines();
        match lines.next().unwrap_or_default().trim() {
            "sat" => {
                let model = parse_and_validate_model(&output, constraints)?;
                self.cache_sat_result(cache_key.clone(), true);
                self.cache_model_result(cache_key, model.clone());
                Ok(model)
            }
            "unsat" => {
                self.model_cache.remove(&cache_key);
                self.cache_sat_result(cache_key, false);
                Err(SymbolicError::Solver("counterexample path became unsat".to_string()))
            }
            "unknown" => {
                if let Some(model) =
                    validated_hard_arith_fallback_model(&smt_constraints, constraints)
                {
                    self.heuristic_witnesses += 1;
                    self.cache_sat_result(cache_key.clone(), true);
                    self.cache_model_result(cache_key, model.clone());
                    Ok(model)
                } else {
                    Err(SymbolicError::SolverUnknown)
                }
            }
            other => Err(SymbolicError::Solver(format!("unexpected solver response `{other}`"))),
        }
    }
}

impl SmtLibSubprocessSolver {
    /// Implements the `is_sat` solver helper.
    fn is_sat_inner(
        &mut self,
        constraints: &[BoolExpr],
        defer_hard_arith_without_witness: bool,
    ) -> Result<bool, SymbolicError> {
        self.sat_queries += 1;
        if constraints.iter().any(bool_contains_gasleft) {
            return Err(SymbolicError::Unsupported("GAS/gasleft() not modeled"));
        }
        let smt_constraints = normalize_constraints_for_solver(constraints);
        let cache_key = constraint_cache_key(&smt_constraints);
        if let Some(result) = self.sat_cache.get(&cache_key) {
            self.sat_cache_hits += 1;
            trace!(result, "is_sat: normalized cache hit");
            return Ok(*result);
        }
        if self.has_cached_unsat_subset(&cache_key) {
            self.sat_cache_hits += 1;
            trace!("is_sat: normalized unsat subset cache hit");
            self.cache_sat_result(cache_key, false);
            return Ok(false);
        }

        self.reserve_query()?;
        self.record_query();
        let _span = trace_span!(
            "solver_query",
            query_id = self.queries,
            constraint_count = constraints.len(),
            kind = "is_sat"
        )
        .entered();
        trace!(query_id = self.queries, constraint_count = constraints.len(), "solver is_sat");
        if constraints_are_directly_unsat(&smt_constraints) {
            trace!("is_sat: direct contradiction");
            self.cache_sat_result(cache_key, false);
            return Ok(false);
        }
        if product_monotonic_unsat_normalized(&smt_constraints) {
            trace!("is_sat: monotonic product contradiction");
            self.cache_sat_result(cache_key, false);
            return Ok(false);
        }
        if constraints_prefer_hard_arith_fallback_first(&smt_constraints) {
            if validated_hard_arith_fallback_model(&smt_constraints, constraints).is_some() {
                self.heuristic_witnesses += 1;
                trace!("is_sat: validated hard arithmetic fallback model before solver");
                self.cache_sat_result(cache_key, true);
                return Ok(true);
            }
            if defer_hard_arith_without_witness {
                trace!("is_sat: deferring hard arithmetic branch without local witness");
                return Err(SymbolicError::SolverUnknown);
            }
        }
        let output = match self.query_normalized(&smt_constraints, false, constraints) {
            Ok(output) => output,
            Err(SymbolicError::SolverUnknown) => {
                if validated_hard_arith_fallback_model(&smt_constraints, constraints).is_some() {
                    self.heuristic_witnesses += 1;
                    trace!("is_sat: validated hard arithmetic fallback model after solver unknown");
                    self.cache_sat_result(cache_key, true);
                    return Ok(true);
                }
                return Err(SymbolicError::SolverUnknown);
            }
            Err(err) => return Err(err),
        };
        match output.lines().next().unwrap_or_default().trim() {
            "sat" => {
                self.cache_sat_result(cache_key, true);
                Ok(true)
            }
            "unsat" => {
                self.cache_sat_result(cache_key, false);
                Ok(false)
            }
            "unknown" => {
                if validated_hard_arith_fallback_model(&smt_constraints, constraints).is_some() {
                    self.heuristic_witnesses += 1;
                    self.cache_sat_result(cache_key, true);
                    Ok(true)
                } else {
                    Err(SymbolicError::SolverUnknown)
                }
            }
            other => Err(SymbolicError::Solver(format!("unexpected solver response `{other}`"))),
        }
    }
    /// Returns the resolved commands or the stored config error.
    pub(crate) fn commands(&self) -> Result<&[SolverCommand], SymbolicError> {
        self.commands
            .as_ref()
            .map(Vec::as_slice)
            .map_err(|err| SymbolicError::Solver(err.to_string()))
    }

    /// Emits one verbose solver diagnostic either live or into the deferred buffer.
    fn emit_diagnostic(&mut self, diagnostic: fmt::Arguments<'_>) {
        if let Some(captured_diagnostics) = &mut self.captured_diagnostics {
            let _ = captured_diagnostics.write_fmt(diagnostic);
        } else {
            let mut stderr = std::io::stderr().lock();
            let _ = stderr.write_fmt(diagnostic);
        }
    }

    /// Validates the `reserve_query` solver helper.
    pub(crate) const fn reserve_query(&self) -> Result<(), SymbolicError> {
        if self.queries >= self.max_queries {
            return Err(SymbolicError::SolverQueryLimit(self.max_queries));
        }
        Ok(())
    }

    /// Records one logical solver query and notifies the live observer, if any.
    fn record_query(&mut self) {
        self.queries += 1;
        if let Some(observer) = &self.query_observer {
            observer(self.queries);
        }
    }

    /// Caches a definitive normalized satisfiability result if the cache has room.
    fn cache_sat_result(&mut self, key: Vec<BoolExpr>, result: bool) {
        if self.sat_cache.contains_key(&key)
            || self.sat_cache.len() < SYMBOLIC_SOLVER_SAT_CACHE_MAX_ENTRIES
        {
            self.sat_cache.insert(key, result);
        }
    }

    /// Caches a validated normalized model result if the cache has room.
    fn cache_model_result(&mut self, key: Vec<BoolExpr>, model: BTreeMap<String, U256>) {
        if self.model_cache.contains_key(&key)
            || self.model_cache.len() < SYMBOLIC_SOLVER_MODEL_CACHE_MAX_ENTRIES
        {
            self.model_cache.insert(key, model);
        }
    }

    /// Returns whether an already-proved unsat constraint set is a subset of `key`.
    fn has_cached_unsat_subset(&self, key: &[BoolExpr]) -> bool {
        self.sat_cache
            .iter()
            .any(|(cached_key, result)| !*result && sorted_bool_exprs_are_subset(cached_key, key))
    }

    /// Sends already-normalized constraints to the configured solver portfolio.
    pub(crate) fn query_normalized(
        &mut self,
        smt_constraints: &[BoolExpr],
        model: bool,
        model_constraints: &[BoolExpr],
    ) -> Result<String, SymbolicError> {
        self.smt_queries += 1;
        let mut vars = BTreeSet::new();
        for constraint in smt_constraints {
            constraint.collect_vars(&mut vars);
        }

        let configured_commands = self.commands()?.to_vec();
        let ordered_commands = self.portfolio_scheduler.ordered_commands(&configured_commands);
        let commands =
            ordered_commands.iter().map(|(_, command)| command.clone()).collect::<Vec<_>>();

        let mut smt = String::with_capacity(256 + smt_constraints.len().saturating_mul(192));
        smt.push_str("(set-logic QF_BV)\n");
        if commands.iter().all(|command| command.smt_timeout)
            && let Some(timeout) = self.timeout.filter(|timeout| *timeout > 0)
        {
            let _ = writeln!(smt, "(set-option :timeout {})", timeout.saturating_mul(1000));
        }
        for var in vars {
            let _ = writeln!(smt, "(declare-fun {var} () (_ BitVec 256))");
        }
        for constraint in smt_constraints {
            let _ = writeln!(smt, "(assert {})", constraint.smt());
        }
        smt.push_str("(check-sat)\n");
        if model {
            smt.push_str("(get-model)\n");
        }
        if self.dump_smt {
            let query = self.queries;
            self.emit_diagnostic(format_args!("--- symbolic SMT query {query} ---\n{smt}\n"));
        }

        let started = Instant::now();
        let result =
            run_solver_commands(&commands, &smt, self.timeout, model.then_some(model_constraints));
        self.solver_time += started.elapsed();
        self.portfolio_scheduler.record(&ordered_commands, &result.summaries);
        if self.dump_smt {
            self.portfolio_diagnostics.record(&result.summaries);
            if !result.summaries.is_empty() {
                self.emit_diagnostic(format_args!(
                    "{}",
                    format_solver_portfolio_summaries(&result.summaries)
                ));
            }
        }
        result.output
    }
}

/// Returns a structural key for normalized solver cache lookups.
fn constraint_cache_key(constraints: &[BoolExpr]) -> Vec<BoolExpr> {
    let mut key = Vec::with_capacity(constraints.len());
    for constraint in constraints.iter().cloned().map(cache_key_bool) {
        collect_cache_key_conjunct(constraint, &mut key);
    }
    key.sort();
    key.dedup();
    key
}

/// Returns whether normalized conjunctive constraints contain a direct contradiction.
fn constraints_are_directly_unsat(constraints: &[BoolExpr]) -> bool {
    constraints.iter().any(|constraint| match constraint {
        BoolExpr::Const(false) => true,
        BoolExpr::Not(inner) => constraints.binary_search(inner.as_ref()).is_ok(),
        constraint => constraints.binary_search(&constraint.clone().not()).is_ok(),
    })
}

/// Returns whether every expression in sorted `subset` appears in sorted `superset`.
fn sorted_bool_exprs_are_subset(subset: &[BoolExpr], superset: &[BoolExpr]) -> bool {
    if subset.len() > superset.len() {
        return false;
    }

    let mut superset = superset.iter();
    for expected in subset {
        loop {
            match superset.next() {
                Some(candidate) if candidate < expected => {}
                Some(candidate) if candidate == expected => break,
                _ => return false,
            }
        }
    }
    true
}

/// Returns a conservative canonical boolean expression for cache-key equality.
fn cache_key_bool(expr: BoolExpr) -> BoolExpr {
    match expr {
        BoolExpr::Const(_) => expr,
        BoolExpr::Not(value) => cache_key_bool(*value).not(),
        BoolExpr::And(values) => {
            let mut conjuncts = Vec::new();
            for value in values.into_iter().map(cache_key_bool) {
                collect_cache_key_conjunct(value, &mut conjuncts);
            }
            conjuncts.sort();
            conjuncts.dedup();
            BoolExpr::and(conjuncts)
        }
        BoolExpr::Eq(left, right) => {
            let left = cache_key_expr(*left);
            let right = cache_key_expr(*right);
            if left <= right { BoolExpr::eq(left, right) } else { BoolExpr::eq(right, left) }
        }
        BoolExpr::Cmp(op, left, right) => {
            cache_key_cmp(op, cache_key_expr(*left), cache_key_expr(*right))
        }
    }
}

/// Collects cache-key conjuncts, flattening conjunctions because path constraints are conjunctive.
fn collect_cache_key_conjunct(expr: BoolExpr, out: &mut Vec<BoolExpr>) {
    match expr {
        BoolExpr::Const(true) => {}
        BoolExpr::And(values) => {
            for value in values {
                collect_cache_key_conjunct(value, out);
            }
        }
        value => out.push(value),
    }
}

/// Returns a conservative canonical comparison for cache-key equality.
fn cache_key_cmp(op: BoolExprOp, left: Expr, right: Expr) -> BoolExpr {
    match op {
        BoolExprOp::Ugt => BoolExpr::cmp(BoolExprOp::Ult, right, left),
        BoolExprOp::Uge => BoolExpr::cmp(BoolExprOp::Ule, right, left),
        BoolExprOp::Sgt => BoolExpr::cmp(BoolExprOp::Slt, right, left),
        BoolExprOp::Ult | BoolExprOp::Ule | BoolExprOp::Slt => BoolExpr::cmp(op, left, right),
    }
}

/// Returns a conservative canonical word expression for cache-key equality.
fn cache_key_expr(expr: Expr) -> Expr {
    match expr {
        Expr::Const(_) | Expr::Var(_) | Expr::GasLeft(_) => expr,
        Expr::Keccak(hash) => {
            let (name, len, bytes) = (*hash).into_parts();
            Expr::keccak(
                name,
                cache_key_expr(len),
                bytes.iter().cloned().map(cache_key_expr).collect(),
            )
        }
        Expr::Hash(hash) => {
            let (name, algorithm, bytes) = (*hash).into_parts();
            Expr::hash(name, algorithm, bytes.iter().cloned().map(cache_key_expr).collect())
        }
        Expr::Not(value) => Expr::not(cache_key_expr(*value)),
        Expr::Op(op, left, right) => {
            let left = cache_key_expr(*left);
            let right = cache_key_expr(*right);
            if expr_op_is_commutative(op) && right < left {
                Expr::op(op, right, left)
            } else {
                Expr::op(op, left, right)
            }
        }
        Expr::AddMod { left, right, modulus } => {
            let left = cache_key_expr(*left);
            let right = cache_key_expr(*right);
            let modulus = cache_key_expr(*modulus);
            if right < left {
                Expr::addmod(right, left, modulus)
            } else {
                Expr::addmod(left, right, modulus)
            }
        }
        Expr::MulMod { left, right, modulus } => {
            let left = cache_key_expr(*left);
            let right = cache_key_expr(*right);
            let modulus = cache_key_expr(*modulus);
            if right < left {
                Expr::mulmod(right, left, modulus)
            } else {
                Expr::mulmod(left, right, modulus)
            }
        }
        Expr::Ite(cond, left, right) => {
            Expr::ite(cache_key_bool(*cond), cache_key_expr(*left), cache_key_expr(*right))
        }
    }
}

/// Returns whether a word operation is safe to reorder for cache-key equality.
const fn expr_op_is_commutative(op: ExprOp) -> bool {
    matches!(op, ExprOp::Add | ExprOp::Mul | ExprOp::And | ExprOp::Or | ExprOp::Xor)
}

/// Returns a hard-arithmetic fallback model only after validating it against original constraints.
fn validated_hard_arith_fallback_model(
    normalized_constraints: &[BoolExpr],
    original_constraints: &[BoolExpr],
) -> Option<BTreeMap<String, U256>> {
    let model = hard_arith_fallback_model(normalized_constraints)?;
    model_satisfies_constraints(&model, original_constraints).then_some(model)
}

/// Returns whether a parsed model satisfies the current original constraints.
fn model_satisfies_constraints(model: &BTreeMap<String, U256>, constraints: &[BoolExpr]) -> bool {
    constraints.iter().all(|constraint| eval_bool_expr(constraint, model).unwrap_or(false))
}

#[derive(Clone, Debug, Default)]
struct PortfolioScheduler {
    history: Vec<VecDeque<PortfolioSchedulerSignal>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PortfolioSchedulerSignal {
    Winner { speed_bonus: i64 },
    InvalidModel,
    Error,
    Unknown,
    Neutral,
}

impl PortfolioSchedulerSignal {
    /// Returns the scheduler signal represented by one solver run summary.
    fn from_summary(summary: &SolverRunSummary) -> Self {
        let speed_bonus = PORTFOLIO_SCHEDULER_MAX_SPEED_BONUS.saturating_sub(
            summary.elapsed.as_millis().min(PORTFOLIO_SCHEDULER_SPEED_BONUS_CAP_MS) as i64,
        );
        match (summary.winner, summary.outcome) {
            (true, SolverOutcome::SatValid | SolverOutcome::Unsat) => Self::Winner { speed_bonus },
            (_, SolverOutcome::SatInvalid) => Self::InvalidModel,
            (_, SolverOutcome::Error | SolverOutcome::Unexpected) => Self::Error,
            (_, SolverOutcome::Unknown | SolverOutcome::TimeoutOrUnknown) => Self::Unknown,
            _ => Self::Neutral,
        }
    }

    /// Returns whether this signal should affect later portfolio scheduling.
    const fn is_neutral(self) -> bool {
        matches!(self, Self::Neutral)
    }

    /// Returns the numeric score contribution for adaptive portfolio ordering.
    const fn score(self) -> i64 {
        match self {
            Self::Winner { speed_bonus } => 1_000 + speed_bonus,
            Self::InvalidModel => -1_000,
            Self::Error => -750,
            Self::Unknown => -250,
            Self::Neutral => 0,
        }
    }
}

impl PortfolioScheduler {
    /// Returns configured commands ordered by recent portfolio performance.
    fn ordered_commands(&mut self, commands: &[SolverCommand]) -> Vec<(usize, SolverCommand)> {
        self.ensure_len(commands.len());
        let mut ordered = commands.iter().cloned().enumerate().collect::<Vec<_>>();
        ordered.sort_by(|(left_index, _), (right_index, _)| {
            self.score(*right_index)
                .cmp(&self.score(*left_index))
                .then_with(|| left_index.cmp(right_index))
        });
        ordered
    }

    /// Records one query's portfolio summaries against original configured solver indexes.
    fn record(
        &mut self,
        ordered_commands: &[(usize, SolverCommand)],
        summaries: &[SolverRunSummary],
    ) {
        for summary in summaries {
            let Some(run_index) = summary.index else { continue };
            let Some((configured_index, _)) = ordered_commands.get(run_index) else { continue };
            let Some(history) = self.history.get_mut(*configured_index) else { continue };
            let signal = PortfolioSchedulerSignal::from_summary(summary);
            if signal.is_neutral() {
                continue;
            }
            history.push_back(signal);
            if history.len() > PORTFOLIO_SCHEDULER_HISTORY {
                history.pop_front();
            }
        }
    }

    /// Ensures the scheduler has one history slot per configured solver.
    fn ensure_len(&mut self, len: usize) {
        self.history.resize_with(len, VecDeque::new);
    }

    /// Returns the recent-performance score for one configured solver index.
    fn score(&self, index: usize) -> i64 {
        self.history
            .get(index)
            .into_iter()
            .flatten()
            .rev()
            .enumerate()
            .map(|(age, signal)| {
                let recency = PORTFOLIO_SCHEDULER_HISTORY
                    .saturating_sub(age)
                    .max(PORTFOLIO_SCHEDULER_MIN_RECENCY_WEIGHT as usize)
                    as i64;
                recency * signal.score()
            })
            .sum()
    }
}

/// Returns the subprocess commands for the configured SMT solver setup.
pub(crate) fn solver_commands_for_config(
    config: &SymbolicConfig,
) -> Result<Vec<SolverCommand>, SolverConfigError> {
    if let Some(command) = config.solver_command.as_deref().filter(|command| !command.is_empty()) {
        return Ok(vec![SolverCommand::new(split_solver_command(command)?, false)?]);
    }

    let portfolio = config
        .solver_portfolio
        .iter()
        .map(|entry| entry.trim())
        .filter(|entry| !entry.is_empty())
        .collect::<Vec<_>>();
    if !portfolio.is_empty() {
        return portfolio.into_iter().map(solver_command_for_portfolio_entry).collect();
    }

    Ok(vec![named_solver_command(&config.solver)?])
}

/// Returns a warning when a configured portfolio will run with unavailable solver entries.
pub(crate) fn solver_portfolio_availability_warning(config: &SymbolicConfig) -> Option<String> {
    if config.solver_command.as_deref().is_some_and(|command| !command.trim().is_empty())
        || config.solver_portfolio.iter().all(|entry| entry.trim().is_empty())
    {
        return None;
    }

    let commands = solver_commands_for_config(config).ok()?;
    let unavailable = commands
        .iter()
        .filter_map(|command| {
            solver_command_availability_error(command)
                .map(|err| format!("`{}` ({err})", command.display))
        })
        .collect::<Vec<_>>();
    if unavailable.is_empty() {
        return None;
    }

    let suffix = if unavailable.len() == commands.len() {
        "No configured portfolio entries are currently available."
    } else {
        "Available portfolio entries will still be used."
    };
    Some(format!(
        "Symbolic solver portfolio is degraded; unavailable entries: {}. {suffix}",
        unavailable.join("; ")
    ))
}

/// Returns the default command for a known solver name.
pub(crate) fn named_solver_command(solver: &str) -> Result<SolverCommand, SolverConfigError> {
    let (parts, smt_timeout) = match solver {
        "z3" => (vec!["z3", "-in", "-smt2"], true),
        "yices" => (vec!["yices-smt2", "--bvconst-in-decimal"], false),
        "cvc5" => (
            vec![
                "cvc5",
                "--produce-models",
                "--lang",
                "smt2",
                "--bv-print-consts-as-indexed-symbols",
            ],
            false,
        ),
        "cvc5-int" => (
            vec![
                "cvc5",
                "--produce-models",
                "--lang",
                "smt2",
                "--bv-print-consts-as-indexed-symbols",
                "--solve-bv-as-int=iand",
                "--iand-mode=bitwise",
            ],
            false,
        ),
        "bitwuzla" => (vec!["bitwuzla", "--produce-models"], false),
        "bitwuzla-abs" => (vec!["bitwuzla", "--produce-models", "--abstraction"], false),
        // Preserve existing behavior for custom z3-compatible executable names/paths.
        custom => (vec![custom, "-in", "-smt2"], true),
    };
    let parts = parts.into_iter().map(str::to_string).collect::<Vec<_>>();
    SolverCommand::new(parts, smt_timeout)
}

/// Returns the command for one configured portfolio entry.
pub(crate) fn solver_command_for_portfolio_entry(
    entry: &str,
) -> Result<SolverCommand, SolverConfigError> {
    if entry.chars().any(|ch| ch.is_whitespace() || matches!(ch, '"' | '\'' | '\\')) {
        SolverCommand::new(split_solver_command(entry)?, false)
    } else {
        named_solver_command(entry)
    }
}

/// Splits a shell-like solver command into argv parts.
pub(crate) fn split_solver_command(command: &str) -> Result<Vec<String>, SolverConfigError> {
    let parts = split_quoted_args(command).map_err(SolverConfigError::UnterminatedQuote)?;
    if parts.is_empty() {
        return Err(SolverConfigError::EmptyCommand);
    }

    Ok(parts)
}

/// Returns why `command` is not currently executable as an SMT solver.
fn solver_command_availability_error(command: &SolverCommand) -> Option<String> {
    let output = match Command::new(&command.program).arg("--version").output() {
        Ok(output) => output,
        Err(err) => return Some(format!("failed to execute `{}`: {err}", command.program)),
    };
    (!output.status.success())
        .then(|| format!("`{}` is not a usable SMT solver executable", command.program))
}

#[derive(Debug)]
enum SolverProcessOutcome {
    Output(String),
    Unknown,
    Cancelled,
    Error(String),
}

#[derive(Debug)]
struct SolverProcessResult {
    index: usize,
    display: String,
    scheduled_after: Duration,
    started_after: Duration,
    elapsed: Duration,
    outcome: SolverProcessOutcome,
}

#[derive(Debug)]
struct ScheduledSolver {
    index: usize,
    command: SolverCommand,
    launch_after: Duration,
}

#[derive(Debug)]
struct SolverCommandRun {
    output: Result<String, SymbolicError>,
    summaries: Vec<SolverRunSummary>,
}

#[derive(Debug)]
pub(crate) struct SolverRunSummary {
    index: Option<usize>,
    display: String,
    scheduled_after: Option<Duration>,
    started_after: Option<Duration>,
    elapsed: Duration,
    outcome: SolverOutcome,
    detail: Option<String>,
    winner: bool,
}

impl SolverRunSummary {
    /// Builds a portfolio run summary with no detail or winner marker.
    pub(crate) const fn new(display: String, elapsed: Duration, outcome: SolverOutcome) -> Self {
        Self {
            index: None,
            display,
            scheduled_after: None,
            started_after: None,
            elapsed,
            outcome,
            detail: None,
            winner: false,
        }
    }

    /// Attaches the configured portfolio order and launch delay to this summary.
    pub(crate) const fn with_schedule(
        mut self,
        index: usize,
        scheduled_after: Duration,
        started_after: Option<Duration>,
    ) -> Self {
        self.index = Some(index);
        self.scheduled_after = Some(scheduled_after);
        self.started_after = started_after;
        self
    }

    /// Attaches an additional diagnostic detail string to this summary.
    fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Marks this solver run as the portfolio result winner.
    pub(crate) const fn winner(mut self) -> Self {
        self.winner = true;
        self
    }
}

#[derive(Clone, Debug, Default)]
pub struct PortfolioDiagnostics {
    pub(crate) queries: usize,
    pub(crate) solver_runs: usize,
    pub(crate) rescue_runs: usize,
    pub(crate) non_primary_wins: usize,
    pub(crate) rescue_wins: usize,
    pub(crate) not_started: usize,
    pub(crate) cancelled_after_winner: usize,
    pub(crate) invalid_models: usize,
    pub(crate) solver_errors: usize,
    pub(crate) winner_counts: BTreeMap<String, usize>,
    pub(crate) launch_counts: BTreeMap<String, usize>,
    pub(crate) outcome_counts: BTreeMap<SolverOutcome, usize>,
}

impl PortfolioDiagnostics {
    /// Returns whether this diagnostic set is empty.
    pub const fn is_empty(&self) -> bool {
        self.queries == 0
    }

    /// Records one portfolio query's per-solver summaries.
    pub(crate) fn record(&mut self, summaries: &[SolverRunSummary]) {
        if summaries.len() <= 1 {
            return;
        }

        self.queries += 1;
        for summary in summaries {
            *self.outcome_counts.entry(summary.outcome).or_default() += 1;
            if summary.started_after.is_some() {
                self.solver_runs += 1;
                *self.launch_counts.entry(summary.display.clone()).or_default() += 1;
                if summary.index.is_some_and(|index| index >= 2) {
                    self.rescue_runs += 1;
                }
            }

            match summary.outcome {
                SolverOutcome::NotStarted => self.not_started += 1,
                SolverOutcome::Cancelled
                | SolverOutcome::SatAfterWinner
                | SolverOutcome::UnsatAfterWinner
                | SolverOutcome::UnknownAfterWinner => self.cancelled_after_winner += 1,
                SolverOutcome::SatInvalid => self.invalid_models += 1,
                SolverOutcome::Error => self.solver_errors += 1,
                _ => {}
            }

            if summary.winner {
                *self.winner_counts.entry(summary.display.clone()).or_default() += 1;
                if summary.index.is_some_and(|index| index > 0) {
                    self.non_primary_wins += 1;
                }
                if summary.index.is_some_and(|index| index >= 2) {
                    self.rescue_wins += 1;
                }
            }
        }
    }

    /// Merges another aggregate portfolio summary into this one.
    pub fn merge(&mut self, other: &Self) {
        self.queries += other.queries;
        self.solver_runs += other.solver_runs;
        self.rescue_runs += other.rescue_runs;
        self.non_primary_wins += other.non_primary_wins;
        self.rescue_wins += other.rescue_wins;
        self.not_started += other.not_started;
        self.cancelled_after_winner += other.cancelled_after_winner;
        self.invalid_models += other.invalid_models;
        self.solver_errors += other.solver_errors;
        merge_counts(&mut self.winner_counts, &other.winner_counts);
        merge_counts(&mut self.launch_counts, &other.launch_counts);
        merge_counts(&mut self.outcome_counts, &other.outcome_counts);
    }
}

impl fmt::Display for PortfolioDiagnostics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_empty() {
            return Ok(());
        }

        writeln!(f, "--- symbolic solver portfolio summary ---")?;
        writeln!(f, "queries: {}", self.queries)?;
        writeln!(f, "solver runs: {}", self.solver_runs)?;
        writeln!(f, "rescue solver runs: {}", self.rescue_runs)?;
        writeln!(f, "not-started solver runs: {}", self.not_started)?;
        writeln!(f, "non-primary wins: {}", self.non_primary_wins)?;
        writeln!(f, "rescue wins: {}", self.rescue_wins)?;
        writeln!(f, "cancelled after winner: {}", self.cancelled_after_winner)?;
        writeln!(f, "invalid models: {}", self.invalid_models)?;
        writeln!(f, "solver errors: {}", self.solver_errors)?;
        if !self.winner_counts.is_empty() {
            writeln!(f, "winner counts:")?;
            for (solver, count) in &self.winner_counts {
                writeln!(f, "  {solver}: {count}")?;
            }
        }
        if !self.launch_counts.is_empty() {
            writeln!(f, "launch counts:")?;
            for (solver, count) in &self.launch_counts {
                writeln!(f, "  {solver}: {count}")?;
            }
        }
        writeln!(f, "outcome counts:")?;
        for (outcome, count) in &self.outcome_counts {
            writeln!(f, "  {outcome}: {count}")?;
        }
        Ok(())
    }
}

fn merge_counts<K: Ord + Clone>(base: &mut BTreeMap<K, usize>, other: &BTreeMap<K, usize>) {
    for (key, count) in other {
        *base.entry(key.clone()).or_default() += count;
    }
}

/// Runs one or more solver commands and returns the first decisive SMT-LIB response.
fn run_solver_commands(
    commands: &[SolverCommand],
    smt: &str,
    timeout: Option<u32>,
    model_constraints: Option<&[BoolExpr]>,
) -> SolverCommandRun {
    if commands.is_empty() {
        return SolverCommandRun {
            output: Err(SymbolicError::Solver("symbolic solver portfolio is empty".to_string())),
            summaries: Vec::new(),
        };
    }
    if commands.len() == 1 {
        let output = match run_solver_process(&commands[0], smt, timeout, &AtomicBool::new(false)) {
            SolverProcessOutcome::Output(output) => Ok(output),
            SolverProcessOutcome::Unknown => Err(SymbolicError::SolverUnknown),
            SolverProcessOutcome::Cancelled => {
                warn!("solver query was cancelled");
                Err(SymbolicError::Solver("solver query was cancelled".to_string()))
            }
            SolverProcessOutcome::Error(err) => Err(SymbolicError::Solver(err)),
        };
        return SolverCommandRun { output, summaries: Vec::new() };
    }

    let cancel = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();
    thread::scope(|scope| {
        let started_at = Instant::now();
        let mut pending = scheduled_portfolio(commands);
        let mut running = 0usize;

        let mut saw_unknown = false;
        let mut saw_unsat = false;
        let mut saw_invalid_sat_model = false;
        let mut errors = Vec::new();
        let mut decisive = None;
        let mut summaries = Vec::new();

        while running > 0 || !pending.is_empty() {
            if decisive.is_none() {
                let now = started_at.elapsed();
                let mut launched = false;
                while pending
                    .front()
                    .is_some_and(|solver| solver.launch_after <= now || (running == 0 && !launched))
                {
                    let solver = pending.pop_front().expect("pending solver exists");
                    let tx = tx.clone();
                    let cancel = Arc::clone(&cancel);
                    let started_after = started_at.elapsed();
                    running += 1;
                    launched = true;
                    scope.spawn(move || {
                        let start = Instant::now();
                        let outcome = run_solver_process(&solver.command, smt, timeout, &cancel);
                        let _ = tx.send(SolverProcessResult {
                            index: solver.index,
                            display: solver.command.display,
                            scheduled_after: solver.launch_after,
                            started_after,
                            elapsed: start.elapsed(),
                            outcome,
                        });
                    });
                }
            }

            if running == 0 {
                continue;
            }

            let result = if decisive.is_none() {
                next_portfolio_launch_wait(started_at, &pending)
                    .map_or_else(|| rx.recv().ok(), |wait| rx.recv_timeout(wait).ok())
            } else {
                rx.recv().ok()
            };
            let Some(result) = result else {
                continue;
            };
            running = running.saturating_sub(1);
            let SolverProcessResult {
                index,
                display,
                scheduled_after,
                started_after,
                elapsed,
                outcome,
            } = result;
            if decisive.is_some() {
                summaries.push(summary_for_cancelled_solver_result(
                    index,
                    display,
                    scheduled_after,
                    started_after,
                    elapsed,
                    outcome,
                ));
                continue;
            }
            match outcome {
                SolverProcessOutcome::Output(output) if solver_output_is_sat(&output) => {
                    if let Some(constraints) = model_constraints
                        && let Err(err) = validate_solver_model_output(&output, constraints)
                    {
                        summaries.push(
                            SolverRunSummary::new(
                                display.clone(),
                                elapsed,
                                SolverOutcome::SatInvalid,
                            )
                            .with_schedule(index, scheduled_after, Some(started_after))
                            .with_detail(err.to_string()),
                        );
                        saw_invalid_sat_model = true;
                        errors.push(format!("{display}: {err}"));
                        continue;
                    }
                    summaries.push(
                        SolverRunSummary::new(display, elapsed, SolverOutcome::SatValid)
                            .with_schedule(index, scheduled_after, Some(started_after))
                            .winner(),
                    );
                    decisive = Some(output);
                    cancel.store(true, Ordering::SeqCst);
                    while let Some(solver) = pending.pop_front() {
                        summaries.push(summary_for_unstarted_solver(solver));
                    }
                }
                SolverProcessOutcome::Output(output) if solver_output_is_unsat(&output) => {
                    summaries.push(
                        SolverRunSummary::new(display, elapsed, SolverOutcome::Unsat)
                            .with_schedule(index, scheduled_after, Some(started_after)),
                    );
                    saw_unsat = true;
                }
                SolverProcessOutcome::Output(output) if solver_output_is_unknown(&output) => {
                    summaries.push(
                        SolverRunSummary::new(display, elapsed, SolverOutcome::Unknown)
                            .with_schedule(index, scheduled_after, Some(started_after)),
                    );
                    saw_unknown = true;
                }
                SolverProcessOutcome::Output(output) => {
                    let first_line = first_solver_line(&output).to_string();
                    summaries.push(
                        SolverRunSummary::new(display.clone(), elapsed, SolverOutcome::Unexpected)
                            .with_schedule(index, scheduled_after, Some(started_after))
                            .with_detail(first_line.clone()),
                    );
                    errors.push(format!("{display}: unexpected solver response `{first_line}`"));
                }
                SolverProcessOutcome::Unknown => {
                    summaries.push(
                        SolverRunSummary::new(display, elapsed, SolverOutcome::TimeoutOrUnknown)
                            .with_schedule(index, scheduled_after, Some(started_after)),
                    );
                    saw_unknown = true;
                }
                SolverProcessOutcome::Cancelled => {
                    summaries.push(
                        SolverRunSummary::new(display, elapsed, SolverOutcome::Cancelled)
                            .with_schedule(index, scheduled_after, Some(started_after)),
                    );
                }
                SolverProcessOutcome::Error(err) => {
                    summaries.push(
                        SolverRunSummary::new(display.clone(), elapsed, SolverOutcome::Error)
                            .with_schedule(index, scheduled_after, Some(started_after))
                            .with_detail(err.clone()),
                    );
                    errors.push(format!("{display}: {err}"));
                }
            }
        }

        if decisive.is_none()
            && saw_unsat
            && let Some(summary) =
                summaries.iter_mut().find(|summary| summary.outcome == SolverOutcome::Unsat)
        {
            summary.winner = true;
        }

        let output = if let Some(output) = decisive {
            Ok(output)
        } else if saw_invalid_sat_model {
            Err(SymbolicError::Solver(errors.join("; ")))
        } else if saw_unsat {
            Ok("unsat\n".to_string())
        } else if saw_unknown {
            Err(SymbolicError::SolverUnknown)
        } else {
            Err(SymbolicError::Solver(errors.join("; ")))
        };

        SolverCommandRun { output, summaries }
    })
}

/// Returns the staged launch plan for a configured portfolio.
fn scheduled_portfolio(commands: &[SolverCommand]) -> VecDeque<ScheduledSolver> {
    commands
        .iter()
        .cloned()
        .enumerate()
        .map(|(index, command)| ScheduledSolver {
            index,
            command,
            launch_after: portfolio_launch_delay(index),
        })
        .collect()
}

/// Returns when the solver at `index` should be started, relative to query start.
const fn portfolio_launch_delay(index: usize) -> Duration {
    match index {
        0 => Duration::ZERO,
        1 => SECOND_PORTFOLIO_SOLVER_DELAY,
        index => RESCUE_PORTFOLIO_SOLVER_DELAY.saturating_mul(index.saturating_sub(1) as u32),
    }
}

/// Returns how long the supervisor can wait before the next pending solver is due.
fn next_portfolio_launch_wait(
    started_at: Instant,
    pending: &VecDeque<ScheduledSolver>,
) -> Option<Duration> {
    pending.front().map(|solver| {
        solver.launch_after.checked_sub(started_at.elapsed()).unwrap_or(Duration::ZERO)
    })
}

/// Summarizes a solver that was never launched because the portfolio already won.
fn summary_for_unstarted_solver(solver: ScheduledSolver) -> SolverRunSummary {
    SolverRunSummary::new(solver.command.display, Duration::ZERO, SolverOutcome::NotStarted)
        .with_schedule(solver.index, solver.launch_after, None)
}

/// Summarizes a solver result received after a portfolio winner was chosen.
fn summary_for_cancelled_solver_result(
    index: usize,
    display: String,
    scheduled_after: Duration,
    started_after: Duration,
    elapsed: Duration,
    outcome: SolverProcessOutcome,
) -> SolverRunSummary {
    let summary = match outcome {
        SolverProcessOutcome::Output(output) if solver_output_is_sat(&output) => {
            SolverRunSummary::new(display, elapsed, SolverOutcome::SatAfterWinner)
        }
        SolverProcessOutcome::Output(output) if solver_output_is_unsat(&output) => {
            SolverRunSummary::new(display, elapsed, SolverOutcome::UnsatAfterWinner)
        }
        SolverProcessOutcome::Output(output) if solver_output_is_unknown(&output) => {
            SolverRunSummary::new(display, elapsed, SolverOutcome::UnknownAfterWinner)
        }
        SolverProcessOutcome::Output(output) => {
            SolverRunSummary::new(display, elapsed, SolverOutcome::Unexpected)
                .with_detail(first_solver_line(&output).to_string())
        }
        SolverProcessOutcome::Unknown => {
            SolverRunSummary::new(display, elapsed, SolverOutcome::TimeoutOrUnknown)
        }
        SolverProcessOutcome::Cancelled => {
            SolverRunSummary::new(display, elapsed, SolverOutcome::Cancelled)
        }
        SolverProcessOutcome::Error(err) => {
            SolverRunSummary::new(display, elapsed, SolverOutcome::Error).with_detail(err)
        }
    };
    summary.with_schedule(index, scheduled_after, Some(started_after))
}

/// Formats solver portfolio outcome diagnostics.
fn format_solver_portfolio_summaries(summaries: &[SolverRunSummary]) -> String {
    let mut output = String::new();
    let _ = writeln!(output, "--- symbolic solver portfolio outcomes ---");
    for summary in summaries {
        let marker = if summary.winner { " winner" } else { "" };
        let schedule = summary.index.zip(summary.scheduled_after).map(|(index, delay)| {
            let started = summary
                .started_after
                .map(|started| format!(" started +{started:.3?}"))
                .unwrap_or_default();
            format!("#{} scheduled +{delay:.3?}{started} ", index + 1)
        });
        let _ = write!(
            output,
            "{}{}: {} in {:.3?}{}",
            schedule.as_deref().unwrap_or_default(),
            summary.display,
            summary.outcome,
            summary.elapsed,
            marker
        );
        if let Some(detail) = summary.detail.as_deref().filter(|detail| !detail.is_empty()) {
            let _ = write!(output, " ({detail})");
        }
        let _ = writeln!(output);
    }
    output
}

/// Runs one solver process to completion, timeout, or cooperative cancellation.
fn run_solver_process(
    command: &SolverCommand,
    smt: &str,
    timeout: Option<u32>,
    cancel: &AtomicBool,
) -> SolverProcessOutcome {
    let mut child = match Command::new(&command.program)
        .args(&command.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(err) => {
            return SolverProcessOutcome::Error(format!(
                "failed to spawn `{}`: {err}",
                command.display
            ));
        }
    };
    let stdout_reader = child.stdout.take().map(read_pipe_to_string);
    let stderr_reader = child.stderr.take().map(read_pipe_to_string);

    if let Some(mut stdin) = child.stdin.take()
        && let Err(err) = stdin.write_all(smt.as_bytes())
    {
        kill_and_reap_solver_process(&mut child, stdout_reader, stderr_reader);
        return SolverProcessOutcome::Error(format!("failed to write solver query: {err}"));
    }

    let deadline = timeout
        .filter(|seconds| *seconds > 0)
        .map(|seconds| Instant::now() + Duration::from_secs(seconds.into()));
    let mut backoff = INITIAL_SOLVER_POLL_BACKOFF;
    let status = loop {
        if cancel.load(Ordering::SeqCst) {
            kill_and_reap_solver_process(&mut child, stdout_reader, stderr_reader);
            return SolverProcessOutcome::Cancelled;
        }
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            kill_and_reap_solver_process(&mut child, stdout_reader, stderr_reader);
            return SolverProcessOutcome::Unknown;
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                thread::sleep(backoff);
                backoff = (backoff * 2).min(MAX_SOLVER_POLL_BACKOFF);
            }
            Err(err) => {
                kill_and_reap_solver_process(&mut child, stdout_reader, stderr_reader);
                return SolverProcessOutcome::Error(format!("failed to read solver output: {err}"));
            }
        }
    };

    let stdout = match join_pipe_output(stdout_reader, "stdout") {
        Ok(stdout) => stdout,
        Err(err) => return SolverProcessOutcome::Error(err),
    };
    let stderr = match join_pipe_output(stderr_reader, "stderr") {
        Ok(stderr) => stderr,
        Err(err) => return SolverProcessOutcome::Error(err),
    };
    if !status.success() {
        return SolverProcessOutcome::Error(solver_exit_error(command, status, &stdout, &stderr));
    }
    SolverProcessOutcome::Output(stdout)
}

fn read_pipe_to_string<R>(mut pipe: R) -> thread::JoinHandle<Result<String, String>>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut output = Vec::new();
        pipe.read_to_end(&mut output)
            .map_err(|err| format!("failed to read solver output: {err}"))?;
        Ok(String::from_utf8_lossy(&output).to_string())
    })
}

fn join_pipe_output(
    reader: Option<thread::JoinHandle<Result<String, String>>>,
    stream: &str,
) -> Result<String, String> {
    match reader {
        Some(reader) => reader.join().map_err(|_| format!("solver {stream} reader panicked"))?,
        None => Ok(String::new()),
    }
}

fn kill_and_reap_solver_process(
    child: &mut std::process::Child,
    stdout_reader: Option<thread::JoinHandle<Result<String, String>>>,
    stderr_reader: Option<thread::JoinHandle<Result<String, String>>>,
) {
    // This only terminates the direct child. Wrapper commands should forward termination and close
    // inherited pipes so descendant solver processes do not outlive cancelled queries.
    let _ = child.kill();
    let _ = child.wait();
    let _ = join_pipe_output(stdout_reader, "stdout");
    let _ = join_pipe_output(stderr_reader, "stderr");
}

fn solver_exit_error(
    command: &SolverCommand,
    status: std::process::ExitStatus,
    stdout: &str,
    stderr: &str,
) -> String {
    let mut message = format!("`{}` exited with {status}", command.display);
    if !stderr.trim().is_empty() {
        message.push_str(": ");
        message.push_str(stderr.trim());
    }
    if !stdout.trim().is_empty() {
        message.push_str("; stdout: ");
        message.push_str(stdout.trim());
    }
    message
}

fn solver_output_is_sat(output: &str) -> bool {
    first_solver_line(output) == "sat"
}

fn solver_output_is_unsat(output: &str) -> bool {
    first_solver_line(output) == "unsat"
}

fn solver_output_is_unknown(output: &str) -> bool {
    first_solver_line(output) == "unknown"
}

fn first_solver_line(output: &str) -> &str {
    output.lines().next().unwrap_or_default().trim()
}

pub(crate) fn parse_and_validate_model(
    output: &str,
    constraints: &[BoolExpr],
) -> Result<BTreeMap<String, U256>, SymbolicError> {
    let model = parse_model(output)?;
    if constraints.iter().all(|constraint| eval_bool_expr(constraint, &model).unwrap_or(false)) {
        Ok(model)
    } else {
        let reason = if constraints.iter().any(bool_contains_keccak) {
            "solver model does not satisfy path constraints involving symbolic Keccak heuristic"
        } else {
            "solver model does not satisfy path constraints"
        };
        debug!(
            constraint_count = constraints.len(),
            reason, "solver model does not satisfy path constraints"
        );
        Err(SymbolicError::Solver(reason.to_string()))
    }
}

pub(crate) fn validate_solver_model_output(
    output: &str,
    constraints: &[BoolExpr],
) -> Result<(), SymbolicError> {
    parse_and_validate_model(output, constraints).map(|_| ())
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
                    if hex.len() > 64 {
                        return Err(SymbolicError::Solver(
                            "solver hex model value exceeds 256 bits".to_string(),
                        ));
                    }
                    let mut bytes = [0u8; 32];
                    let decoded = alloy_primitives::hex::decode(hex).map_err(|err| {
                        SymbolicError::Solver(format!("invalid solver hex model value: {err}"))
                    })?;
                    let start = 32usize.saturating_sub(decoded.len());
                    bytes[start..start + decoded.len()].copy_from_slice(&decoded);
                    values.insert(name.to_string(), U256::from_be_bytes(bytes));
                    break;
                }
                if let Some(binary) = value.strip_prefix("#b") {
                    if binary.len() > 256 {
                        return Err(SymbolicError::Solver(
                            "solver binary model value exceeds 256 bits".to_string(),
                        ));
                    }
                    let parsed = U256::from_str_radix(binary, 2).map_err(|err| {
                        SymbolicError::Solver(format!("invalid solver binary model value: {err}"))
                    })?;
                    values.insert(name.to_string(), parsed);
                    break;
                }
                if value == "_"
                    && let Some(bv) = tokens.next().and_then(|v| v.strip_prefix("bv"))
                {
                    let parsed = U256::from_str_radix(bv, 10).map_err(|err| {
                        SymbolicError::Solver(format!("invalid solver decimal model value: {err}"))
                    })?;
                    values.insert(name.to_string(), parsed);
                    break;
                }
            }
        }
    }
    Ok(values)
}
