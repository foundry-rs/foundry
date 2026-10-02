//! Solver orchestration, query scheduling, caching, and model validation.

use super::*;
use std::{
    io::{BufRead, BufReader, Read},
    process::{Child, ChildStdin, Output},
    sync::mpsc::{Receiver, RecvTimeoutError},
    thread::JoinHandle,
};
use wait_timeout::ChildExt;

mod fallback;
mod normalize;
mod reasoning;
mod smt;

use fallback::{checked_mul_guard_branch_model, constraints_prefer_hard_arith_fallback_first};
use normalize::{
    constraints_are_directly_unsat, normalize_constraints_for_solver_cached,
    sorted_bool_exprs_are_subset,
};
use reasoning::{product_monotonic_unsat_normalized, remove_implied_monotonic_constraints};
use smt::write_smt_assertions;

pub(crate) use fallback::{
    fallback_bounded_model, fallback_single_var_model, hard_arith_fallback_model,
};

const Z3_QUERY_END: &str = "foundry-query-complete";

/// Errors that arise when parsing or constructing solver commands from configuration.
#[derive(Debug, thiserror::Error)]
pub(crate) enum SolverConfigError {
    /// The command string parsed to an empty argv.
    #[error("symbolic solver command is empty")]
    EmptyCommand,
    /// The command string contains invalid shell quoting.
    #[error("invalid shell quoting in symbolic solver command")]
    InvalidShellQuoting,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
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

impl fmt::Display for SolverOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
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
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BranchFeasibility {
    Sat,
    Unsat,
    NeedsSolver,
}

impl BranchFeasibility {
    const fn into_result(self) -> Result<bool, SymbolicError> {
        match self {
            Self::Sat => Ok(true),
            Self::Unsat => Ok(false),
            Self::NeedsSolver => Err(SymbolicError::SolverUnknown),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SolverCommand {
    program: String,
    args: Vec<String>,
    display: String,
    smt_timeout: bool,
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
    commands: Result<Vec<SolverCommand>, SolverConfigError>,
    timeout: Option<u32>,
    max_queries: usize,
    queries: usize,
    dump_smt: bool,
    portfolio_scheduler: PortfolioScheduler,
    heuristic_witnesses: usize,
    replayable_storage: SymbolicVars,
    normalization_cache: HashMap<SymBoolExpr, SymBoolExpr>,
    sat_cache: HashMap<Vec<SymBoolExpr>, bool>,
    model_cache: HashMap<Vec<SymBoolExpr>, SymbolicModel>,
    sat_queries: usize,
    model_queries: usize,
    sat_cache_hits: usize,
    model_cache_hits: usize,
    smt_queries: usize,
    solver_time: Duration,
    smt_input_bytes: u64,
    smt_max_query_bytes: u64,
    smt_build_time: Duration,
    smt_max_query_time: Duration,
    z3_session: Option<Z3Session>,
}

impl SmtLibSubprocessSolver {
    /// Constructs a subprocess solver from Foundry symbolic config.
    pub(crate) fn from_config(config: &SymbolicConfig) -> Self {
        Self {
            commands: solver_commands_for_config(config),
            timeout: config.timeout,
            max_queries: config.max_solver_queries as usize,
            queries: 0,
            dump_smt: config.dump_smt,
            portfolio_scheduler: PortfolioScheduler::default(),
            heuristic_witnesses: 0,
            replayable_storage: SymbolicVars::default(),
            normalization_cache: HashMap::default(),
            sat_cache: HashMap::default(),
            model_cache: HashMap::default(),
            sat_queries: 0,
            model_queries: 0,
            sat_cache_hits: 0,
            model_cache_hits: 0,
            smt_queries: 0,
            solver_time: Duration::ZERO,
            smt_input_bytes: 0,
            smt_max_query_bytes: 0,
            smt_build_time: Duration::ZERO,
            smt_max_query_time: Duration::ZERO,
            z3_session: None,
        }
    }

    /// Returns solver counters collected by this backend.
    pub(crate) fn stats(&self) -> SymbolicStats {
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
            smt_input_bytes: self.smt_input_bytes,
            smt_max_query_bytes: self.smt_max_query_bytes,
            smt_build_time_ms: self.smt_build_time.as_millis().try_into().unwrap_or(u64::MAX),
            smt_max_query_time_ms: self
                .smt_max_query_time
                .as_millis()
                .try_into()
                .unwrap_or(u64::MAX),
        }
    }

    /// Clears cached expression keys tied to a previous symbolic context.
    pub(crate) fn clear_context_caches(&mut self) {
        self.normalization_cache.clear();
        self.sat_cache.clear();
        self.model_cache.clear();
    }

    /// Verifies that a configured solver can be invoked before exploration starts.
    pub(crate) fn check_available(&self) -> Result<(), SymbolicError> {
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

    /// Returns satisfiability with path-local storage symbols that concrete replay can set.
    pub(crate) fn is_sat_with_replayable_storage(
        &mut self,
        cx: &mut SymCx,
        constraints: &[SymBoolExpr],
        replayable_storage: &SymbolicVars,
    ) -> Result<bool, SymbolicError> {
        self.with_replayable_storage(replayable_storage, |solver| {
            solver.is_sat_inner(cx, constraints, false).and_then(BranchFeasibility::into_result)
        })
    }

    /// Returns branch feasibility with path-local storage symbols concrete replay can set.
    pub(crate) fn branch_feasibility_with_replayable_storage(
        &mut self,
        cx: &mut SymCx,
        constraints: &[SymBoolExpr],
        replayable_storage: &SymbolicVars,
    ) -> Result<BranchFeasibility, SymbolicError> {
        self.with_replayable_storage(replayable_storage, |solver| {
            solver.is_sat_inner(cx, constraints, true)
        })
    }

    /// Returns a model with path-local storage symbols that concrete replay can set.
    pub(crate) fn model_with_replayable_storage(
        &mut self,
        cx: &mut SymCx,
        constraints: &[SymBoolExpr],
        replayable_storage: &SymbolicVars,
    ) -> Result<SymbolicModel, SymbolicError> {
        self.with_replayable_storage(replayable_storage, |solver| solver.model(cx, constraints))
    }

    fn with_replayable_storage<T>(
        &mut self,
        replayable_storage: &SymbolicVars,
        operation: impl FnOnce(&mut Self) -> T,
    ) -> T {
        let previous = std::mem::replace(&mut self.replayable_storage, replayable_storage.clone());
        let result = operation(self);
        self.replayable_storage = previous;
        result
    }

    /// Returns variable assignments used to materialize inputs for concrete replay.
    pub(crate) fn model(
        &mut self,
        cx: &mut SymCx,
        constraints: &[SymBoolExpr],
    ) -> Result<SymbolicModel, SymbolicError> {
        // Local witnesses may decide the feasibility of gas-dependent branches, but a model assigns
        // `gasleft()` a concrete value the engine cannot replay faithfully, so fail closed here.
        if constraints.iter().any(SymBoolExpr::contains_gasleft) {
            return Err(SymbolicError::Unsupported("GAS/gasleft() not modeled"));
        }
        self.model_queries += 1;
        let smt_constraints =
            normalize_constraints_for_solver_cached(cx, constraints, &mut self.normalization_cache);
        let cache_key = smt_constraints.clone();

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
            if eval_model_constraints(constraints, model) {
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
        self.queries += 1;
        let _span = trace_span!(
            "solver_query",
            query_id = self.queries,
            constraint_count = constraints.len(),
            kind = "model"
        )
        .entered();
        trace!(query_id = self.queries, constraint_count = constraints.len(), "solver model");
        if let Some(model) = fallback_single_var_model(&smt_constraints)
            && eval_model_constraints(constraints, &model)
        {
            self.cache_sat_result(cache_key.clone(), true);
            self.cache_model_result(cache_key, model.clone());
            return Ok(model);
        }
        if let Some(model) = fallback_bounded_model(&smt_constraints)
            && eval_model_constraints(constraints, &model)
        {
            self.cache_sat_result(cache_key.clone(), true);
            self.cache_model_result(cache_key, model.clone());
            return Ok(model);
        }
        if let Some(model) = checked_mul_guard_branch_model(
            cx,
            &smt_constraints,
            constraints,
            &self.replayable_storage,
        ) {
            trace!("model: validated constructive checked-multiply guard model");
            self.cache_sat_result(cache_key.clone(), true);
            return Ok(model);
        }
        if constraints_prefer_hard_arith_fallback_first(cx, &smt_constraints)
            && let Some(model) =
                validated_hard_arith_fallback_model(cx, &smt_constraints, constraints)
        {
            self.heuristic_witnesses += 1;
            trace!("model: validated hard arithmetic fallback model before solver");
            self.cache_sat_result(cache_key.clone(), true);
            self.cache_model_result(cache_key, model.clone());
            return Ok(model);
        }
        let output = match self.query_normalized(cx, &smt_constraints, true, constraints) {
            Ok(output) => output,
            Err(SymbolicError::SolverUnknown) => {
                if let Some(model) =
                    validated_hard_arith_fallback_model(cx, &smt_constraints, constraints)
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
                let model = parse_and_validate_model(cx, &output, constraints)?;
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
                    validated_hard_arith_fallback_model(cx, &smt_constraints, constraints)
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

    fn is_sat_inner(
        &mut self,
        cx: &mut SymCx,
        constraints: &[SymBoolExpr],
        defer_hard_arith_without_witness: bool,
    ) -> Result<BranchFeasibility, SymbolicError> {
        self.sat_queries += 1;
        let smt_constraints =
            normalize_sat_constraints(cx, constraints, &mut self.normalization_cache);
        let cache_key = smt_constraints.clone();
        if let Some(result) = self.sat_cache.get(&cache_key) {
            self.sat_cache_hits += 1;
            trace!(result, "is_sat: normalized cache hit");
            return Ok(if *result { BranchFeasibility::Sat } else { BranchFeasibility::Unsat });
        }
        if self.has_cached_unsat_subset(&cache_key) {
            self.sat_cache_hits += 1;
            trace!("is_sat: normalized unsat subset cache hit");
            self.cache_sat_result(cache_key, false);
            return Ok(BranchFeasibility::Unsat);
        }
        if defer_hard_arith_without_witness
            && let Some((condition, base)) = constraints.split_last()
            && {
                let normalized_base =
                    normalize_sat_constraints(cx, base, &mut self.normalization_cache);
                self.sat_cache.get(&normalized_base) == Some(&true)
            }
            && {
                let mut complement = Vec::with_capacity(constraints.len());
                complement.extend(base.iter().cloned());
                complement.push(condition.clone().not(cx));
                let normalized_complement =
                    normalize_sat_constraints(cx, &complement, &mut self.normalization_cache);
                self.has_cached_unsat_subset(&normalized_complement)
            }
        {
            self.sat_cache_hits += 1;
            trace!("is_sat: branch complement unsat cache hit");
            self.cache_sat_result(cache_key, true);
            return Ok(BranchFeasibility::Sat);
        }

        self.reserve_query()?;
        self.queries += 1;
        let _span = trace_span!(
            "solver_query",
            query_id = self.queries,
            constraint_count = constraints.len(),
            kind = "is_sat"
        )
        .entered();
        trace!(query_id = self.queries, constraint_count = constraints.len(), "solver is_sat");
        if constraints_are_directly_unsat(cx, &smt_constraints) {
            trace!("is_sat: direct contradiction");
            self.cache_sat_result(cache_key, false);
            return Ok(BranchFeasibility::Unsat);
        }
        if product_monotonic_unsat_normalized(&smt_constraints) {
            trace!("is_sat: monotonic product contradiction");
            self.cache_sat_result(cache_key, false);
            return Ok(BranchFeasibility::Unsat);
        }
        if !constraints.is_empty()
            && eval_model_constraints(constraints, &SymbolicModel::default())
            && !constraints.iter().any(SymBoolExpr::contains_gasleft)
        {
            self.cache_sat_result(cache_key, true);
            return Ok(BranchFeasibility::Sat);
        }
        if let Some(model) = fallback_single_var_model(&smt_constraints)
            && eval_model_constraints(constraints, &model)
        {
            self.cache_sat_result(cache_key, true);
            return Ok(BranchFeasibility::Sat);
        }
        if let Some(model) = fallback_bounded_model(&smt_constraints)
            && eval_model_constraints(constraints, &model)
        {
            self.cache_sat_result(cache_key, true);
            return Ok(BranchFeasibility::Sat);
        }
        if checked_mul_guard_branch_model(
            cx,
            &smt_constraints,
            constraints,
            &self.replayable_storage,
        )
        .is_some()
        {
            trace!("is_sat: validated constructive checked-multiply guard model");
            self.cache_sat_result(cache_key, true);
            return Ok(BranchFeasibility::Sat);
        }
        if constraints_prefer_hard_arith_fallback_first(cx, &smt_constraints) {
            if validated_hard_arith_fallback_model(cx, &smt_constraints, constraints).is_some() {
                self.heuristic_witnesses += 1;
                trace!("is_sat: validated hard arithmetic fallback model before solver");
                self.cache_sat_result(cache_key, true);
                return Ok(BranchFeasibility::Sat);
            }
            if defer_hard_arith_without_witness {
                trace!("is_sat: deferring hard arithmetic branch without local witness");
                return Ok(BranchFeasibility::NeedsSolver);
            }
        }
        let output = match self.query_normalized(cx, &smt_constraints, false, constraints) {
            Ok(output) => output,
            Err(SymbolicError::SolverUnknown) => {
                if validated_hard_arith_fallback_model(cx, &smt_constraints, constraints).is_some()
                {
                    self.heuristic_witnesses += 1;
                    trace!("is_sat: validated hard arithmetic fallback model after solver unknown");
                    self.cache_sat_result(cache_key, true);
                    return Ok(BranchFeasibility::Sat);
                }
                return Err(SymbolicError::SolverUnknown);
            }
            Err(err) => return Err(err),
        };
        match output.lines().next().unwrap_or_default().trim() {
            "sat" => {
                self.cache_sat_result(cache_key, true);
                Ok(BranchFeasibility::Sat)
            }
            "unsat" => {
                self.cache_sat_result(cache_key, false);
                Ok(BranchFeasibility::Unsat)
            }
            "unknown" => {
                if validated_hard_arith_fallback_model(cx, &smt_constraints, constraints).is_some()
                {
                    self.heuristic_witnesses += 1;
                    self.cache_sat_result(cache_key, true);
                    Ok(BranchFeasibility::Sat)
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

    pub(crate) const fn reserve_query(&self) -> Result<(), SymbolicError> {
        if self.queries >= self.max_queries {
            return Err(SymbolicError::SolverQueryLimit(self.max_queries));
        }
        Ok(())
    }

    fn cache_sat_result(&mut self, key: Vec<SymBoolExpr>, result: bool) {
        cache_result(&mut self.sat_cache, key, result, SYMBOLIC_SOLVER_SAT_CACHE_MAX_ENTRIES);
    }

    fn cache_model_result(&mut self, key: Vec<SymBoolExpr>, model: SymbolicModel) {
        cache_result(&mut self.model_cache, key, model, SYMBOLIC_SOLVER_MODEL_CACHE_MAX_ENTRIES);
    }

    /// Returns whether an already-proved unsat constraint set is a subset of `key`.
    fn has_cached_unsat_subset(&self, key: &[SymBoolExpr]) -> bool {
        self.sat_cache
            .iter()
            .any(|(cached_key, result)| !*result && sorted_bool_exprs_are_subset(cached_key, key))
    }

    /// Sends already-normalized constraints to the configured solver portfolio.
    pub(crate) fn query_normalized(
        &mut self,
        cx: &SymCx,
        smt_constraints: &[SymBoolExpr],
        model: bool,
        model_constraints: &[SymBoolExpr],
    ) -> Result<String, SymbolicError> {
        self.smt_queries += 1;
        let build_started = Instant::now();
        let mut vars = SymbolicVars::default();
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
            let name = cx.symbol_name(var);
            let _ = writeln!(smt, "(declare-fun {name} () (_ BitVec 256))");
        }
        write_smt_assertions(cx, &mut smt, smt_constraints)?;
        smt.push_str("(check-sat)\n");
        if model {
            smt.push_str("(get-model)\n");
        }
        let smt_bytes = smt.len().try_into().unwrap_or(u64::MAX);
        self.smt_input_bytes = self.smt_input_bytes.saturating_add(smt_bytes);
        self.smt_max_query_bytes = self.smt_max_query_bytes.max(smt_bytes);
        self.smt_build_time += build_started.elapsed();
        if self.dump_smt {
            let query = self.queries;
            let _ = writeln!(std::io::stderr(), "--- symbolic SMT query {query} ---\n{smt}");
        }

        let started = Instant::now();
        let result = if let [command] = commands.as_slice()
            && command.smt_timeout
            && command.program == "z3"
            && command.args == ["-in", "-smt2"]
        {
            let output = self.query_z3(command, &smt).into_result();
            SolverCommandRun { output, summaries: Vec::new() }
        } else {
            run_solver_commands(
                cx,
                &commands,
                &smt,
                self.timeout,
                model.then_some(model_constraints),
            )
        };
        let query_time = started.elapsed();
        self.solver_time += query_time;
        self.smt_max_query_time = self.smt_max_query_time.max(query_time);
        self.portfolio_scheduler.record(&ordered_commands, &result.summaries);
        if self.dump_smt && !result.summaries.is_empty() {
            let _ = write!(
                std::io::stderr(),
                "{}",
                format_solver_portfolio_summaries(&result.summaries)
            );
        }
        result.output
    }

    fn query_z3(&mut self, command: &SolverCommand, smt: &str) -> SolverProcessOutcome {
        let mut session = match self.z3_session.take() {
            Some(session) => session,
            None => match Z3Session::spawn(command) {
                Ok(session) => session,
                Err(err) => return SolverProcessOutcome::Error(err),
            },
        };
        let outcome = session.query(command, smt, self.timeout);
        match outcome {
            output @ SolverProcessOutcome::Output(_) => {
                self.z3_session = Some(session);
                output
            }
            SolverProcessOutcome::Error(_) => {
                drop(session);
                run_solver_process(command, smt, self.timeout, &AtomicBool::new(false))
            }
            other => other,
        }
    }
}

fn cache_result<K, V>(cache: &mut HashMap<K, V>, key: K, value: V, max_entries: usize)
where
    K: Eq + std::hash::Hash,
{
    let has_capacity = cache.len() < max_entries;
    match cache.entry(key) {
        alloy_primitives::map::Entry::Occupied(mut entry) => {
            entry.insert(value);
        }
        alloy_primitives::map::Entry::Vacant(entry) if has_capacity => {
            entry.insert(value);
        }
        alloy_primitives::map::Entry::Vacant(_) => {}
    }
}

/// Normalizes satisfiability constraints and removes soundly redundant constraints.
fn normalize_sat_constraints(
    cx: &mut SymCx,
    constraints: &[SymBoolExpr],
    normalization_cache: &mut HashMap<SymBoolExpr, SymBoolExpr>,
) -> Vec<SymBoolExpr> {
    let constraints = remove_implied_monotonic_constraints(
        normalize_constraints_for_solver_cached(cx, constraints, normalization_cache),
    );
    remove_witnessed_isolated_hash_constraints(cx, constraints)
}

/// Removes independently satisfiable constraints over one opaque hash symbol.
///
/// SMT treats symbolic hashes as free bit-vector symbols. If a constraint's only SMT symbol is a
/// hash unused by other constraints, a concrete witness proves that it cannot affect conjunction
/// satisfiability. The witness is discarded because hash values are not replayable inputs.
fn remove_witnessed_isolated_hash_constraints(
    cx: &mut SymCx,
    constraints: Vec<SymBoolExpr>,
) -> Vec<SymBoolExpr> {
    if !constraints.iter().any(|constraint| {
        constraint.visit_bool(|expr| {
            matches!(expr.kind(), SymExprKind::Keccak { .. } | SymExprKind::Hash { .. })
        })
    }) {
        return constraints;
    }

    let mut symbol_constraint_counts = HashMap::<Symbol, usize>::default();
    let hash_candidates = constraints
        .iter()
        .map(|constraint| {
            let mut symbols = SymbolicVars::default();
            let contains_hash = collect_solver_vars(constraint, &mut symbols);
            for symbol in &symbols {
                *symbol_constraint_counts.entry(*symbol).or_default() += 1;
            }
            if contains_hash && symbols.len() == 1 { symbols.first().copied() } else { None }
        })
        .collect::<Vec<_>>();

    constraints
        .into_iter()
        .zip(hash_candidates)
        .filter_map(|(constraint, candidate)| {
            let Some(symbol) =
                candidate.filter(|symbol| symbol_constraint_counts.get(symbol) == Some(&1))
            else {
                return Some(constraint);
            };
            let abstracted = constraint.fold_exprs(cx, &mut |cx, expr| match expr.kind() {
                SymExprKind::Keccak { name, .. } | SymExprKind::Hash { name, .. }
                    if *name == symbol =>
                {
                    SymExpr::get_var(cx, symbol)
                }
                _ => expr,
            });
            let removable = fallback_single_var_model(std::slice::from_ref(&abstracted)).is_some();
            (!removable).then_some(constraint)
        })
        .collect()
}

/// Collects variables as the SMT writer sees them, stopping at opaque hash leaves.
fn collect_solver_vars(constraint: &SymBoolExpr, vars: &mut SymbolicVars) -> bool {
    fn visit_bool(expr: &SymBoolExpr, vars: &mut SymbolicVars) -> bool {
        match expr.kind() {
            SymBoolExprKind::Const(_) => false,
            SymBoolExprKind::Not(expr) => visit_bool(expr, vars),
            SymBoolExprKind::And(exprs) => {
                let mut contains_hash = false;
                for expr in exprs.iter() {
                    contains_hash |= visit_bool(expr, vars);
                }
                contains_hash
            }
            SymBoolExprKind::Cmp(_, left, right) => {
                visit_word(left, vars) | visit_word(right, vars)
            }
        }
    }

    fn visit_word(expr: &SymExpr, vars: &mut SymbolicVars) -> bool {
        match expr.kind() {
            SymExprKind::Const(_) => false,
            SymExprKind::Var(symbol) | SymExprKind::GasLeft(symbol) => {
                vars.insert(*symbol);
                false
            }
            SymExprKind::Keccak { name, .. } | SymExprKind::Hash { name, .. } => {
                vars.insert(*name);
                true
            }
            SymExprKind::Not(expr) => visit_word(expr, vars),
            SymExprKind::BinOp(_, left, right) => visit_word(left, vars) | visit_word(right, vars),
            SymExprKind::TernOp(_, left, right, modulus) => {
                visit_word(left, vars) | visit_word(right, vars) | visit_word(modulus, vars)
            }
            SymExprKind::Ite(condition, then_expr, else_expr) => {
                visit_bool(condition, vars)
                    | visit_word(then_expr, vars)
                    | visit_word(else_expr, vars)
            }
        }
    }

    visit_bool(constraint, vars)
}

/// Returns a hard-arithmetic fallback model only after validating it against original constraints.
fn validated_hard_arith_fallback_model(
    cx: &SymCx,
    normalized_constraints: &[SymBoolExpr],
    original_constraints: &[SymBoolExpr],
) -> Option<SymbolicModel> {
    let model = hard_arith_fallback_model(cx, normalized_constraints)?;
    eval_model_constraints(original_constraints, &model).then_some(model)
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
        self.history.resize_with(commands.len(), VecDeque::new);
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
            if matches!(signal, PortfolioSchedulerSignal::Neutral) {
                continue;
            }
            history.push_back(signal);
            if history.len() > PORTFOLIO_SCHEDULER_HISTORY {
                history.pop_front();
            }
        }
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
    let parts = shlex::split(command).ok_or(SolverConfigError::InvalidShellQuoting)?;
    if parts.is_empty() {
        return Err(SolverConfigError::EmptyCommand);
    }

    Ok(parts)
}

#[derive(Debug)]
enum SolverProcessOutcome {
    Output(String),
    Unknown,
    Cancelled,
    Error(String),
}

impl SolverProcessOutcome {
    fn into_result(self) -> Result<String, SymbolicError> {
        match self {
            Self::Output(output) => Ok(output),
            Self::Unknown => Err(SymbolicError::SolverUnknown),
            Self::Cancelled => {
                warn!("solver query was cancelled");
                Err(SymbolicError::Solver("solver query was cancelled".to_string()))
            }
            Self::Error(err) => Err(SymbolicError::Solver(err)),
        }
    }
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

    fn with_detail(mut self, detail: String) -> Self {
        self.detail = Some(detail);
        self
    }

    /// Marks this solver run as the portfolio result winner.
    pub(crate) const fn winner(mut self) -> Self {
        self.winner = true;
        self
    }
}

/// Runs one or more solver commands and returns the first decisive SMT-LIB response.
fn run_solver_commands(
    cx: &SymCx,
    commands: &[SolverCommand],
    smt: &str,
    timeout: Option<u32>,
    model_constraints: Option<&[SymBoolExpr]>,
) -> SolverCommandRun {
    if commands.is_empty() {
        return SolverCommandRun {
            output: Err(SymbolicError::Solver("symbolic solver portfolio is empty".to_string())),
            summaries: Vec::new(),
        };
    }
    if commands.len() == 1 {
        let output =
            run_solver_process(&commands[0], smt, timeout, &AtomicBool::new(false)).into_result();
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
                SolverProcessOutcome::Output(output)
                    if output.lines().next().unwrap_or_default().trim() == "sat" =>
                {
                    if let Some(constraints) = model_constraints
                        && let Err(err) = validate_solver_model_output(cx, &output, constraints)
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
                SolverProcessOutcome::Output(output)
                    if output.lines().next().unwrap_or_default().trim() == "unsat" =>
                {
                    summaries.push(
                        SolverRunSummary::new(display, elapsed, SolverOutcome::Unsat)
                            .with_schedule(index, scheduled_after, Some(started_after)),
                    );
                    saw_unsat = true;
                }
                SolverProcessOutcome::Output(output)
                    if output.lines().next().unwrap_or_default().trim() == "unknown" =>
                {
                    summaries.push(
                        SolverRunSummary::new(display, elapsed, SolverOutcome::Unknown)
                            .with_schedule(index, scheduled_after, Some(started_after)),
                    );
                    saw_unknown = true;
                }
                SolverProcessOutcome::Output(output) => {
                    let first_line = output.lines().next().unwrap_or_default().trim().to_string();
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
        SolverProcessOutcome::Output(output)
            if output.lines().next().unwrap_or_default().trim() == "sat" =>
        {
            SolverRunSummary::new(display, elapsed, SolverOutcome::SatAfterWinner)
        }
        SolverProcessOutcome::Output(output)
            if output.lines().next().unwrap_or_default().trim() == "unsat" =>
        {
            SolverRunSummary::new(display, elapsed, SolverOutcome::UnsatAfterWinner)
        }
        SolverProcessOutcome::Output(output)
            if output.lines().next().unwrap_or_default().trim() == "unknown" =>
        {
            SolverRunSummary::new(display, elapsed, SolverOutcome::UnknownAfterWinner)
        }
        SolverProcessOutcome::Output(output) => {
            SolverRunSummary::new(display, elapsed, SolverOutcome::Unexpected)
                .with_detail(output.lines().next().unwrap_or_default().trim().to_string())
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

struct Z3Session {
    child: SolverChild,
    stdin: ChildStdin,
    stdout: Receiver<Result<String, String>>,
    stderr: Receiver<String>,
    stdout_thread: Option<JoinHandle<()>>,
    stderr_thread: Option<JoinHandle<()>>,
}

impl Z3Session {
    fn spawn(command: &SolverCommand) -> Result<Self, String> {
        let mut child = spawn_solver_process(command)?;
        let stdin = child.child_mut().stdin.take().expect("piped solver stdin is available");
        let stdout = child.child_mut().stdout.take().expect("piped solver stdout is available");
        let stderr = child.child_mut().stderr.take().expect("piped solver stderr is available");

        let (stdout_tx, stdout_rx) = mpsc::channel();
        let stdout_thread = thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let line = line.map_err(|err| format!("failed to read solver output: {err}"));
                let failed = line.is_err();
                if stdout_tx.send(line).is_err() || failed {
                    break;
                }
            }
        });
        let (stderr_tx, stderr_rx) = mpsc::channel();
        let stderr_thread = thread::spawn(move || {
            let mut stderr = BufReader::new(stderr);
            let mut output = String::new();
            let _ = stderr.read_to_string(&mut output);
            let _ = stderr_tx.send(output);
        });

        Ok(Self {
            child,
            stdin,
            stdout: stdout_rx,
            stderr: stderr_rx,
            stdout_thread: Some(stdout_thread),
            stderr_thread: Some(stderr_thread),
        })
    }

    fn query(
        &mut self,
        command: &SolverCommand,
        smt: &str,
        timeout: Option<u32>,
    ) -> SolverProcessOutcome {
        if let Err(err) = self
            .stdin
            .write_all(b"(reset)\n")
            .and_then(|_| self.stdin.write_all(smt.as_bytes()))
            .and_then(|_| writeln!(self.stdin, "(echo \"{Z3_QUERY_END}\")"))
            .and_then(|_| self.stdin.flush())
        {
            return SolverProcessOutcome::Error(format!("failed to write solver query: {err}"));
        }

        let started_at = Instant::now();
        let timeout = timeout
            .filter(|seconds| *seconds > 0)
            .map(|seconds| Duration::from_secs(seconds.into()));
        let mut output = String::new();
        loop {
            let Some(wait) = solver_wait_duration(started_at.elapsed(), timeout) else {
                return SolverProcessOutcome::Unknown;
            };
            match self.stdout.recv_timeout(wait) {
                Ok(Ok(line)) if line == Z3_QUERY_END => {
                    return SolverProcessOutcome::Output(output);
                }
                Ok(Ok(line)) => {
                    output.push_str(&line);
                    output.push('\n');
                }
                Ok(Err(err)) => return SolverProcessOutcome::Error(err),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    let stderr =
                        self.stderr.recv_timeout(SOLVER_CANCEL_CHECK_INTERVAL).unwrap_or_default();
                    return match self.child.child_mut().try_wait() {
                        Ok(Some(status)) => SolverProcessOutcome::Error(solver_exit_error(
                            command, status, &output, &stderr,
                        )),
                        Ok(None) => SolverProcessOutcome::Error(
                            "solver stdout closed before the query completed".to_string(),
                        ),
                        Err(err) => SolverProcessOutcome::Error(format!(
                            "failed to query solver process status: {err}"
                        )),
                    };
                }
            }
        }
    }
}

impl Drop for Z3Session {
    fn drop(&mut self) {
        self.child.terminate();
        if let Some(thread) = self.stdout_thread.take() {
            let _ = thread.join();
        }
        if let Some(thread) = self.stderr_thread.take() {
            let _ = thread.join();
        }
    }
}

fn spawn_solver_process(command: &SolverCommand) -> Result<SolverChild, String> {
    Command::new(&command.program)
        .args(&command.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map(SolverChild::new)
        .map_err(|err| format!("failed to spawn `{}`: {err}", command.display))
}

/// Runs one solver process to completion, timeout, or cooperative cancellation.
fn run_solver_process(
    command: &SolverCommand,
    smt: &str,
    timeout: Option<u32>,
    cancel: &AtomicBool,
) -> SolverProcessOutcome {
    let mut child = match spawn_solver_process(command) {
        Ok(child) => child,
        Err(err) => return SolverProcessOutcome::Error(err),
    };

    if let Some(mut stdin) = child.child_mut().stdin.take()
        && let Err(err) = stdin.write_all(smt.as_bytes())
    {
        return SolverProcessOutcome::Error(format!("failed to write solver query: {err}"));
    }

    let started_at = Instant::now();
    let timeout =
        timeout.filter(|seconds| *seconds > 0).map(|seconds| Duration::from_secs(seconds.into()));
    loop {
        if cancel.load(Ordering::SeqCst) {
            return SolverProcessOutcome::Cancelled;
        }

        let Some(wait) = solver_wait_duration(started_at.elapsed(), timeout) else {
            return SolverProcessOutcome::Unknown;
        };

        match child.child_mut().wait_timeout(wait) {
            Ok(Some(_)) => break,
            Ok(None) => {}
            Err(err) => {
                return SolverProcessOutcome::Error(format!(
                    "failed to wait for solver process: {err}"
                ));
            }
        }
    }

    let output = match child.wait_with_output() {
        Ok(output) => output,
        Err(err) => {
            return SolverProcessOutcome::Error(format!("failed to read solver output: {err}"));
        }
    };
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        return SolverProcessOutcome::Error(solver_exit_error(
            command,
            output.status,
            &stdout,
            &stderr,
        ));
    }
    SolverProcessOutcome::Output(stdout)
}

fn solver_wait_duration(elapsed: Duration, timeout: Option<Duration>) -> Option<Duration> {
    let Some(timeout) = timeout else {
        return Some(SOLVER_CANCEL_CHECK_INTERVAL);
    };
    let remaining = timeout.checked_sub(elapsed)?;
    if remaining.is_zero() { None } else { Some(remaining.min(SOLVER_CANCEL_CHECK_INTERVAL)) }
}

struct SolverChild {
    child: Option<Child>,
}

impl SolverChild {
    const fn new(child: Child) -> Self {
        Self { child: Some(child) }
    }

    const fn child_mut(&mut self) -> &mut Child {
        self.child.as_mut().expect("solver child exists")
    }

    fn wait_with_output(mut self) -> std::io::Result<Output> {
        self.child.take().expect("solver child exists").wait_with_output()
    }

    fn terminate(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for SolverChild {
    fn drop(&mut self) {
        self.terminate();
    }
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

pub(crate) fn parse_and_validate_model(
    cx: &SymCx,
    output: &str,
    constraints: &[SymBoolExpr],
) -> Result<SymbolicModel, SymbolicError> {
    let symbols = model_symbols_for_constraints(cx, constraints);
    let model = parse_model_with_symbols(output, &symbols)?;
    if eval_model_constraints(constraints, &model) {
        Ok(model)
    } else {
        let reason = if constraints.iter().any(SymBoolExpr::contains_keccak) {
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
    cx: &SymCx,
    output: &str,
    constraints: &[SymBoolExpr],
) -> Result<(), SymbolicError> {
    parse_and_validate_model(cx, output, constraints).map(|_| ())
}

fn parse_model_with_symbols(
    output: &str,
    symbols: &HashMap<String, Symbol>,
) -> Result<SymbolicModel, SymbolicError> {
    parse_model_with_symbol(output, |name| symbols.get(name).copied())
}

fn parse_model_with_symbol(
    output: &str,
    mut symbol_for: impl FnMut(&str) -> Option<Symbol>,
) -> Result<SymbolicModel, SymbolicError> {
    let mut values = SymbolicModel::default();
    parse_model_values(output, |name, value| {
        if let Some(symbol) = symbol_for(name) {
            values.insert(symbol, value);
        }
    })?;
    Ok(values)
}

fn parse_model_values(
    output: &str,
    mut insert_value: impl FnMut(&str, U256),
) -> Result<(), SymbolicError> {
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
                    insert_value(name, U256::from_be_bytes(bytes));
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
                    insert_value(name, parsed);
                    break;
                }
                if value == "_"
                    && let Some(bv) = tokens.next().and_then(|v| v.strip_prefix("bv"))
                {
                    let parsed = U256::from_str_radix(bv, 10).map_err(|err| {
                        SymbolicError::Solver(format!("invalid solver decimal model value: {err}"))
                    })?;
                    insert_value(name, parsed);
                    break;
                }
            }
        }
    }
    Ok(())
}

fn model_symbols_for_constraints(
    cx: &SymCx,
    constraints: &[SymBoolExpr],
) -> HashMap<String, Symbol> {
    let mut vars = SymbolicVars::default();
    for constraint in constraints {
        constraint.collect_vars(&mut vars);
    }
    vars.into_iter().map(|symbol| (cx.symbol_name(symbol).to_owned(), symbol)).collect()
}
