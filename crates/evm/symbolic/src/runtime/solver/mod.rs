//! Solver orchestration, query scheduling, caching, and model validation.

use super::*;
use std::{
    io::{BufRead, BufReader, Read},
    process::{Child, Output},
    sync::mpsc::{Receiver, RecvTimeoutError},
    thread::JoinHandle,
};
use wait_timeout::ChildExt;

mod fallback;
mod normalize;
mod reasoning;
mod routing;
mod session;
mod smt;

use fallback::{checked_mul_guard_branch_model, constraints_prefer_hard_arith_fallback_first};
use normalize::{
    NormalizationCache, constraints_are_directly_unsat, normalize_constraints_for_solver_cached,
    sorted_bool_exprs_are_subset,
};
use reasoning::{
    CheckedXorResult, checked_xor_result, product_monotonic_unsat_normalized,
    remove_implied_monotonic_constraints,
};
use routing::{QueryBucket, query_bucket};
use session::{SolverSession, run_solver_query};
use smt::write_smt_assertions;

pub(crate) use fallback::{
    fallback_bounded_model, fallback_single_var_model, hard_arith_fallback_model,
};

/// Extra wall-clock time a session solver gets past its own per-query limit before it is killed.
const SESSION_TIMEOUT_GRACE: Duration = Duration::from_secs(1);

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
    /// Known solver name, used to route queries to the solver suited to them.
    name: Option<&'static str>,
    /// Extra arguments that let the solver answer queries in one long-lived process, or `None`
    /// when it runs one process per query.
    session_args: Option<&'static [&'static str]>,
    /// How the solver takes a per-query time limit in milliseconds in session mode.
    session_timeout_flag: Option<SessionTimeoutFlag>,
}

/// Command-line spelling of a solver's per-query time limit in milliseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SessionTimeoutFlag {
    /// The flag, then the limit as its own argument.
    Separate(&'static str),
    /// The flag with the limit appended.
    Joined(&'static str),
    /// No flag: each query sets `(set-option :timeout ms)`.
    SmtOption,
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
        Ok(Self {
            program,
            args,
            display,
            smt_timeout,
            name: None,
            session_args: None,
            session_timeout_flag: None,
        })
    }

    /// Marks this command as the known solver `name`, able to run as a session with
    /// `session_args`.
    const fn known(
        mut self,
        name: &'static str,
        session_args: &'static [&'static str],
        session_timeout_flag: Option<SessionTimeoutFlag>,
    ) -> Self {
        self.name = Some(name);
        self.session_args = Some(session_args);
        self.session_timeout_flag = session_timeout_flag;
        self
    }

    /// Whether the solver can answer queries in one long-lived process.
    pub(crate) const fn session_capable(&self) -> bool {
        self.session_args.is_some()
    }

    /// Whether a session separates queries with `(reset)` instead of `(push 1)`/`(pop 1)`.
    ///
    /// Inside a pushed scope z3 switches to its incremental solver, which skips the bit-blasting
    /// preprocessing that makes hard bit-vector queries tractable (one query here took 5.8 s
    /// plainly and over 20 s pushed). `(reset)` keeps the fast one-shot solver. Bitwuzla 0.9.1
    /// crashes on `(get-model)` after `(reset)`, so it and cvc5 keep using scopes. The incremental
    /// solver still wins on some queries (one ite-heavy query took 3.4 s scoped and over 25 s
    /// one-shot), so `z3-inc` names z3 kept in scoped mode, to race against `z3`.
    pub(crate) fn session_resets(&self) -> bool {
        self.name == Some("z3")
    }

    /// Returns the command line that starts this solver as a session.
    pub(crate) fn session_command(&self, timeout: Option<u32>) -> Self {
        let mut command = self.clone();
        command
            .args
            .extend(self.session_args.unwrap_or_default().iter().map(|arg| arg.to_string()));
        if let Some(flag) = self.session_timeout_flag
            && let Some(seconds) = timeout.filter(|seconds| *seconds > 0)
        {
            let ms = u64::from(seconds) * 1000;
            match flag {
                SessionTimeoutFlag::Separate(flag) => {
                    command.args.push(flag.to_string());
                    command.args.push(ms.to_string());
                }
                SessionTimeoutFlag::Joined(flag) => command.args.push(format!("{flag}{ms}")),
                SessionTimeoutFlag::SmtOption => {}
            }
        }
        command
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
    normalization_cache: NormalizationCache,
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
    /// Long-lived solver processes, per parallel worker and configured solver.
    sessions: Vec<Vec<Option<SolverSession>>>,
    /// Number of queries a batch may solve at once.
    jobs: usize,
    /// Wall-clock deadline of the whole run; queries never run past it.
    deadline: Option<Instant>,
    /// Models of recent satisfiable queries, tried on new queries before the solver, when
    /// [`Self::set_reuse_models`] turned reuse on.
    recent_models: Option<VecDeque<SymbolicModel>>,
    model_reuse_hits: usize,
}

/// Result of a batch query stopped because another query of the batch was satisfiable.
pub(crate) const BATCH_STOPPED: &str = "solver query stopped: another batch query was satisfiable";

/// How often a racing query checks whether its batch stopped.
const BATCH_STOP_POLL_INTERVAL: Duration = Duration::from_millis(20);

/// SMT-LIB text of a query, its routing bucket, and the solvers to race in order with their
/// portfolio indexes.
type BuiltQuery = (String, QueryBucket, Vec<(usize, SolverCommand)>);

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
            normalization_cache: NormalizationCache::default(),
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
            sessions: Vec::new(),
            jobs: 1,
            deadline: None,
            recent_models: None,
            model_reuse_hits: 0,
        }
    }

    /// Turns on trying the models of recent satisfiable queries on each new query before the
    /// solver, which settles most feasibility checks along a path, as KLEE's counterexample cache
    /// does. Satisfiable queries then also ask the solver for a model.
    pub(crate) fn set_reuse_models(&mut self, reuse: bool) {
        self.recent_models = reuse.then(VecDeque::new);
    }

    /// Returns whether a recent model, with unassigned symbols set to zero, satisfies every
    /// constraint, which proves them satisfiable.
    fn recent_model_satisfies(&self, constraints: &[SymBoolExpr]) -> bool {
        let Some(models) = &self.recent_models else { return false };
        if constraints.is_empty() || constraints.iter().any(SymBoolExpr::contains_gasleft) {
            return false;
        }
        models.iter().any(|model| eval_model_constraints(constraints, &ZeroDefaultModel(model)))
    }

    /// Remembers a model for [`Self::recent_model_satisfies`].
    fn remember_model(&mut self, model: &SymbolicModel) {
        let Some(models) = &mut self.recent_models else { return };
        if models.len() >= RECENT_MODELS {
            models.pop_back();
        }
        models.push_front(model.clone());
    }

    /// Sets how many queries a batch may solve in parallel.
    pub(crate) fn set_jobs(&mut self, jobs: usize) {
        self.jobs = jobs.max(1);
    }

    /// Sets the wall-clock deadline queries must finish by.
    pub(crate) const fn set_deadline(&mut self, deadline: Option<Instant>) {
        self.deadline = deadline;
    }

    /// Returns the time limit, in whole seconds, the next query may use: the configured limit,
    /// capped by what is left before the deadline.
    fn query_timeout(&self) -> Result<Option<u32>, SymbolicError> {
        let Some(deadline) = self.deadline else { return Ok(self.timeout) };
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(SymbolicError::Timeout(self.timeout.unwrap_or_default()));
        }
        let remaining = u32::try_from(remaining.as_secs().max(1)).unwrap_or(u32::MAX);
        Ok(Some(
            self.timeout.filter(|timeout| *timeout > 0).map_or(remaining, |t| t.min(remaining)),
        ))
    }

    /// Returns worker `worker`'s sessions, one slot per configured solver.
    fn worker_sessions(
        &mut self,
        worker: usize,
        commands: usize,
    ) -> &mut Vec<Option<SolverSession>> {
        if self.sessions.len() <= worker {
            self.sessions.resize_with(worker + 1, Vec::new);
        }
        let sessions = &mut self.sessions[worker];
        if sessions.len() < commands {
            sessions.resize_with(commands, || None);
        }
        sessions
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
            model_reuse_hits: self.model_reuse_hits,
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
        let model = self.model_refining(cx, constraints, KECCAK_REFINEMENT_ROUNDS)?;
        self.remember_model(&model);
        Ok(model)
    }

    /// Returns a model, refining up to `rounds` times a model whose Keccak values are not the
    /// real hashes of its preimages.
    ///
    /// Each Keccak node is an unconstrained symbol to the solver, so its model may pick any hash.
    /// A refinement adds, for each wrong hash, the fact that this preimage hashes to its real
    /// value, which is sound because Keccak is a function, and solves again.
    fn model_refining(
        &mut self,
        cx: &mut SymCx,
        constraints: &[SymBoolExpr],
        rounds: usize,
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
        if let Some(result) = checked_xor_result(constraints) {
            match result {
                CheckedXorResult::Unsat => {
                    self.cache_sat_result(cache_key, false);
                    return Err(SymbolicError::Solver(
                        "counterexample path became unsat".to_string(),
                    ));
                }
                CheckedXorResult::Model(model) => {
                    self.cache_sat_result(cache_key.clone(), true);
                    self.cache_model_result(cache_key, model.clone());
                    return Ok(model);
                }
            }
        }
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
        // A model with wrong Keccak values is refined below rather than rejected in the race.
        let has_keccak = constraints.iter().any(SymBoolExpr::contains_keccak);
        let validated = if has_keccak { &[][..] } else { constraints };
        let output = match self.query_normalized(cx, &smt_constraints, true, validated) {
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
            "sat" if has_keccak => {
                let symbols = model_symbols_for_constraints(cx, constraints);
                let model = parse_model_with_symbols(&output, &symbols)?;
                if eval_model_constraints(constraints, &model) {
                    self.cache_sat_result(cache_key.clone(), true);
                    self.cache_model_result(cache_key, model.clone());
                    return Ok(model);
                }
                let lemmas = keccak_lemmas(cx, constraints, &model);
                if lemmas.is_empty() || rounds == 0 {
                    return parse_and_validate_model(cx, &output, constraints);
                }
                trace!(lemmas = lemmas.len(), "model: refining Keccak values");
                let mut refined = constraints.to_vec();
                refined.extend(lemmas);
                // Pinning the hashes of this model's preimages alone lets the next model move the
                // collision to other preimages: also tie hash equality to preimage equality.
                for lemma in hash_congruence_lemmas(cx, constraints) {
                    if !refined.contains(&lemma) {
                        refined.push(lemma);
                    }
                }
                self.model_refining(cx, &refined, rounds - 1)
            }
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
        match self.sat_prepare(cx, constraints, defer_hard_arith_without_witness)? {
            SatStep::Done(result) => Ok(result),
            SatStep::Query { cache_key, smt_constraints } => {
                let want_model = self.recent_models.is_some();
                let output = self.query_normalized(cx, &smt_constraints, want_model, &[]);
                self.sat_finish(cx, constraints, cache_key, &smt_constraints, output)
            }
        }
    }

    /// Settles a satisfiability check without the SMT solver when caches or local reasoning can,
    /// and otherwise returns the normalized query to send.
    fn sat_prepare(
        &mut self,
        cx: &mut SymCx,
        constraints: &[SymBoolExpr],
        defer_hard_arith_without_witness: bool,
    ) -> Result<SatStep, SymbolicError> {
        self.sat_queries += 1;
        let smt_constraints =
            normalize_sat_constraints(cx, constraints, &mut self.normalization_cache);
        let cache_key = smt_constraints.clone();
        if let Some(result) = self.sat_cache.get(&cache_key) {
            self.sat_cache_hits += 1;
            trace!(result, "is_sat: normalized cache hit");
            return Ok(SatStep::Done(if *result {
                BranchFeasibility::Sat
            } else {
                BranchFeasibility::Unsat
            }));
        }
        if self.has_cached_unsat_subset(&cache_key) {
            self.sat_cache_hits += 1;
            trace!("is_sat: normalized unsat subset cache hit");
            self.cache_sat_result(cache_key, false);
            return Ok(SatStep::Done(BranchFeasibility::Unsat));
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
            return Ok(SatStep::Done(BranchFeasibility::Sat));
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
            return Ok(SatStep::Done(BranchFeasibility::Unsat));
        }
        if product_monotonic_unsat_normalized(&smt_constraints) {
            trace!("is_sat: monotonic product contradiction");
            self.cache_sat_result(cache_key, false);
            return Ok(SatStep::Done(BranchFeasibility::Unsat));
        }
        if let Some(result) = checked_xor_result(constraints) {
            let sat = matches!(result, CheckedXorResult::Model(_));
            trace!(sat, "is_sat: checked XOR specialist");
            self.cache_sat_result(cache_key.clone(), sat);
            if let CheckedXorResult::Model(model) = result {
                self.cache_model_result(cache_key, model);
            }
            return Ok(SatStep::Done(if sat {
                BranchFeasibility::Sat
            } else {
                BranchFeasibility::Unsat
            }));
        }
        if !constraints.is_empty()
            && eval_model_constraints(constraints, &SymbolicModel::default())
            && !constraints.iter().any(SymBoolExpr::contains_gasleft)
        {
            self.cache_sat_result(cache_key, true);
            return Ok(SatStep::Done(BranchFeasibility::Sat));
        }
        if let Some(model) = fallback_single_var_model(&smt_constraints)
            && eval_model_constraints(constraints, &model)
        {
            self.cache_sat_result(cache_key, true);
            return Ok(SatStep::Done(BranchFeasibility::Sat));
        }
        if let Some(model) = fallback_bounded_model(&smt_constraints)
            && eval_model_constraints(constraints, &model)
        {
            self.cache_sat_result(cache_key, true);
            return Ok(SatStep::Done(BranchFeasibility::Sat));
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
            return Ok(SatStep::Done(BranchFeasibility::Sat));
        }
        if constraints_prefer_hard_arith_fallback_first(cx, &smt_constraints) {
            if validated_hard_arith_fallback_model(cx, &smt_constraints, constraints).is_some() {
                self.heuristic_witnesses += 1;
                trace!("is_sat: validated hard arithmetic fallback model before solver");
                self.cache_sat_result(cache_key, true);
                return Ok(SatStep::Done(BranchFeasibility::Sat));
            }
            if defer_hard_arith_without_witness {
                trace!("is_sat: deferring hard arithmetic branch without local witness");
                return Ok(SatStep::Done(BranchFeasibility::NeedsSolver));
            }
        }
        if self.recent_model_satisfies(constraints) {
            trace!("is_sat: recent model satisfies the constraints");
            self.model_reuse_hits += 1;
            self.cache_sat_result(cache_key, true);
            return Ok(SatStep::Done(BranchFeasibility::Sat));
        }
        Ok(SatStep::Query { cache_key, smt_constraints })
    }

    /// Interprets the solver's answer to a query [`Self::sat_prepare`] returned.
    fn sat_finish(
        &mut self,
        cx: &mut SymCx,
        constraints: &[SymBoolExpr],
        cache_key: Vec<SymBoolExpr>,
        smt_constraints: &[SymBoolExpr],
        output: Result<String, SymbolicError>,
    ) -> Result<BranchFeasibility, SymbolicError> {
        let smt_constraints = smt_constraints.to_vec();
        let output = match output {
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
                if self.recent_models.is_some() && output.contains("define-fun") {
                    let symbols = model_symbols_for_constraints(cx, constraints);
                    if let Ok(model) = parse_model_with_symbols(&output, &symbols) {
                        self.remember_model(&model);
                    }
                }
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
        let timeout = self.query_timeout()?;
        let (smt, bucket, ordered_commands) = self.build_query(cx, smt_constraints, model)?;
        let sessions = std::mem::take(self.worker_sessions(0, ordered_commands.len()));
        let started = Instant::now();
        let (result, sessions) = run_solver_commands(
            &ordered_commands,
            &smt,
            timeout,
            model.then_some((cx, model_constraints)),
            sessions,
            None,
        );
        *self.worker_sessions(0, ordered_commands.len()) = sessions;
        self.finish_query(bucket, &ordered_commands, started.elapsed(), result)
    }

    /// Returns satisfiability of each constraint set, sending the ones local reasoning cannot
    /// settle to the solver portfolio up to [`Self::set_jobs`] at a time.
    ///
    /// Each set comes with the storage symbols concrete replay can set on its path.
    ///
    /// With `stop_at_sat`, the first satisfiable query stops the queries still running or not yet
    /// started, whose results are then `Err(SymbolicError::Solver(BATCH_STOPPED))`: they were not
    /// decided, and the caller re-runs them if that satisfiable result turns out not to count.
    pub(crate) fn is_sat_batch(
        &mut self,
        cx: &mut SymCx,
        queries: &[(Vec<SymBoolExpr>, SymbolicVars)],
        stop_at_sat: bool,
    ) -> Vec<Result<bool, SymbolicError>> {
        let stop = AtomicBool::new(false);
        let mut results = (0..queries.len()).map(|_| None).collect::<Vec<_>>();
        let mut pending = Vec::new();
        for (index, (constraints, replayable)) in queries.iter().enumerate() {
            // Local reasoning on a large batch can take long on its own: stop at the deadline.
            if let Err(err) = self.query_timeout() {
                results[index] = Some(Err(err));
                continue;
            }
            let step = self.with_replayable_storage(replayable, |solver| {
                solver.sat_prepare(cx, constraints, false)
            });
            match step {
                Ok(SatStep::Done(result)) => results[index] = Some(result.into_result()),
                Ok(SatStep::Query { cache_key, smt_constraints }) => {
                    pending.push((index, cache_key, smt_constraints));
                }
                Err(err) => results[index] = Some(Err(err)),
            }
        }

        for chunk in pending.chunks(self.jobs.max(1)) {
            if stop.load(Ordering::SeqCst) {
                for (index, ..) in chunk {
                    results[*index] = Some(Err(SymbolicError::Solver(BATCH_STOPPED.to_string())));
                }
                continue;
            }
            let timeout = match self.query_timeout() {
                Ok(timeout) => timeout,
                Err(err) => {
                    for (index, ..) in chunk {
                        let _ = &err;
                        results[*index] =
                            Some(Err(SymbolicError::Timeout(self.timeout.unwrap_or_default())));
                    }
                    continue;
                }
            };
            let mut built = Vec::with_capacity(chunk.len());
            let want_model = self.recent_models.is_some();
            for (index, _, smt_constraints) in chunk {
                match self.build_query(cx, smt_constraints, want_model) {
                    Ok(query) => built.push((*index, query)),
                    Err(err) => results[*index] = Some(Err(err)),
                }
            }
            let workers = built.len();
            let commands_len = built.first().map_or(0, |(_, (_, _, commands))| commands.len());
            let mut worker_sessions = (0..workers)
                .map(|worker| std::mem::take(self.worker_sessions(worker, commands_len)))
                .collect::<Vec<_>>();
            let started = Instant::now();
            let outcomes = thread::scope(|scope| {
                let handles = built
                    .iter()
                    .zip(worker_sessions.iter_mut())
                    .map(|((_, (smt, _, commands)), sessions)| {
                        let taken = std::mem::take(sessions);
                        let stop = &stop;
                        scope.spawn(move || {
                            let run = run_solver_commands(
                                commands,
                                smt,
                                timeout,
                                None,
                                taken,
                                stop_at_sat.then_some(stop),
                            );
                            let sat = run.0.output.as_ref().is_ok_and(|output| {
                                output.lines().next().is_some_and(|line| line.trim() == "sat")
                            });
                            if stop_at_sat && sat {
                                stop.store(true, Ordering::SeqCst);
                            }
                            run
                        })
                    })
                    .collect::<Vec<_>>();
                handles
                    .into_iter()
                    .map(|handle| handle.join().expect("solver worker panicked"))
                    .collect::<Vec<_>>()
            });
            let elapsed = started.elapsed();
            let mut outcomes = outcomes.into_iter();
            for (worker, (index, (_, bucket, commands))) in built.into_iter().enumerate() {
                let (result, sessions) = outcomes.next().expect("one outcome per query");
                *self.worker_sessions(worker, commands.len()) = sessions;
                let output = self.finish_query(bucket, &commands, elapsed, result);
                let (_, constraints_replayable) = &queries[index];
                let (constraints, _) = &queries[index];
                let (_, cache_key, smt_constraints) = chunk
                    .iter()
                    .find(|(pending_index, ..)| *pending_index == index)
                    .expect("built query is pending");
                let result = self.with_replayable_storage(constraints_replayable, |solver| {
                    solver.sat_finish(cx, constraints, cache_key.clone(), smt_constraints, output)
                });
                results[index] = Some(result.and_then(BranchFeasibility::into_result));
            }
            drop(worker_sessions);
        }
        results.into_iter().map(|result| result.expect("every query has a result")).collect()
    }

    /// Builds the SMT-LIB text of a query and picks the solver order for it.
    fn build_query(
        &mut self,
        cx: &SymCx,
        smt_constraints: &[SymBoolExpr],
        model: bool,
    ) -> Result<BuiltQuery, SymbolicError> {
        self.smt_queries += 1;
        let build_started = Instant::now();
        let mut vars = SymbolicVars::default();
        for constraint in smt_constraints {
            constraint.collect_vars(&mut vars);
        }

        let bucket = query_bucket(smt_constraints);
        let configured_commands = self.commands()?.to_vec();
        let ordered_commands =
            self.portfolio_scheduler.ordered_commands(&configured_commands, bucket);

        let mut smt = String::with_capacity(256 + smt_constraints.len().saturating_mul(192));
        smt.push_str("(set-logic QF_BV)\n");
        if ordered_commands.iter().all(|(_, command)| command.smt_timeout)
            && let Some(timeout) = self.query_timeout()?.filter(|timeout| *timeout > 0)
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
            let _ = writeln!(
                std::io::stderr(),
                "--- symbolic SMT query {query} ({bucket:?}) ---\n{smt}"
            );
        }
        Ok((smt, bucket, ordered_commands))
    }

    /// Records a solved query's timing and portfolio outcomes and returns its output.
    fn finish_query(
        &mut self,
        bucket: QueryBucket,
        ordered_commands: &[(usize, SolverCommand)],
        query_time: Duration,
        result: SolverCommandRun,
    ) -> Result<String, SymbolicError> {
        self.solver_time += query_time;
        self.smt_max_query_time = self.smt_max_query_time.max(query_time);
        self.portfolio_scheduler.record(bucket, ordered_commands, &result.summaries);
        if self.dump_smt && !result.summaries.is_empty() {
            let _ = write!(
                std::io::stderr(),
                "{}",
                format_solver_portfolio_summaries(&result.summaries)
            );
        }
        result.output
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
    normalization_cache: &mut NormalizationCache,
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

/// Most recent models kept for reuse.
const RECENT_MODELS: usize = 32;

/// A model that assigns zero to every symbol it does not mention.
struct ZeroDefaultModel<'a>(&'a SymbolicModel);

impl SymbolicModelLookup for ZeroDefaultModel<'_> {
    fn value(&self, name: Symbol) -> Option<U256> {
        Some(self.0.get(&name).copied().unwrap_or_default())
    }
}

/// Most rounds of Keccak model refinement.
const KECCAK_REFINEMENT_ROUNDS: usize = 8;

/// Returns, for each Keccak node whose model value is not the real hash of its model preimage,
/// the lemma "this preimage hashes to its real value".
fn keccak_lemmas(
    cx: &mut SymCx,
    constraints: &[SymBoolExpr],
    model: &SymbolicModel,
) -> Vec<SymBoolExpr> {
    let mut nodes = Vec::new();
    let mut seen = HashSet::<SymExpr>::default();
    for constraint in constraints {
        let _ = visit_unique::<()>(None, Some(constraint), |expr| {
            if matches!(expr.kind(), SymExprKind::Keccak { .. }) && seen.insert(expr.clone()) {
                nodes.push(expr.clone());
            }
            ControlFlow::Continue(())
        });
    }
    let mut lemmas = Vec::new();
    for node in nodes {
        let SymExprKind::Keccak { name, len, bytes } = node.kind() else { continue };
        let Some(assigned) = model.get(name).copied() else { continue };
        let Ok(real) = node.eval_model(model) else { continue };
        if assigned == real {
            continue;
        }
        let Ok(length) = len.eval_model(model) else { continue };
        let Ok(length) = usize::try_from(length) else { continue };
        let mut premise = vec![SymBoolExpr::eq_word_const(cx, len, U256::from(length))];
        for byte in bytes.iter().take(length) {
            let Ok(value) = byte.eval_model(model) else { continue };
            premise.push(SymBoolExpr::eq_word_const(cx, byte, value));
        }
        let premise = SymBoolExpr::and(cx, premise);
        let hashes = SymBoolExpr::eq_word_const(cx, &node, real);
        let not_premise = premise.not(cx);
        lemmas.push(SymBoolExpr::or(cx, vec![not_premise, hashes]));
    }
    lemmas
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

/// Result of [`SmtLibSubprocessSolver::sat_prepare`].
enum SatStep {
    /// Settled without the SMT solver.
    Done(BranchFeasibility),
    /// Needs the SMT solver on `smt_constraints`.
    Query { cache_key: Vec<SymBoolExpr>, smt_constraints: Vec<SymBoolExpr> },
}

/// Orders the configured solvers for each query by bucket, from a default order per bucket and
/// each solver's recent results on queries of that bucket.
#[derive(Clone, Debug, Default)]
struct PortfolioScheduler {
    /// Recent signals per bucket and configured solver.
    history: [Vec<VecDeque<PortfolioSchedulerSignal>>; QueryBucket::ALL.len()],
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
    /// Returns configured commands ordered for a query of `bucket`: by recent performance on
    /// that bucket, then by the bucket's default order, then as configured.
    fn ordered_commands(
        &mut self,
        commands: &[SolverCommand],
        bucket: QueryBucket,
    ) -> Vec<(usize, SolverCommand)> {
        self.history[bucket.index()].resize_with(commands.len(), VecDeque::new);
        let mut ordered = commands.iter().cloned().enumerate().collect::<Vec<_>>();
        ordered.sort_by(|(left_index, left), (right_index, right)| {
            self.score(bucket, *right_index)
                .cmp(&self.score(bucket, *left_index))
                .then_with(|| bucket.prior_rank(left.name).cmp(&bucket.prior_rank(right.name)))
                .then_with(|| left_index.cmp(right_index))
        });
        ordered
    }

    /// Records one query's portfolio summaries against original configured solver indexes.
    fn record(
        &mut self,
        bucket: QueryBucket,
        ordered_commands: &[(usize, SolverCommand)],
        summaries: &[SolverRunSummary],
    ) {
        for summary in summaries {
            let Some(run_index) = summary.index else { continue };
            let Some((configured_index, _)) = ordered_commands.get(run_index) else { continue };
            let Some(history) = self.history[bucket.index()].get_mut(*configured_index) else {
                continue;
            };
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

    /// Returns the recent-performance score for one configured solver index on `bucket`.
    fn score(&self, bucket: QueryBucket, index: usize) -> i64 {
        self.history[bucket.index()]
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
        "z3" | "z3-inc" => (vec!["z3", "-in", "-smt2"], true),
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
        "bitwuzla-prop" => (vec!["bitwuzla", "--produce-models", "--bv-solver", "prop"], false),
        // Preserve existing behavior for custom z3-compatible executable names/paths.
        custom => (vec![custom, "-in", "-smt2"], true),
    };
    let session = match solver {
        "z3" | "z3-inc" => Some((&[][..], Some(SessionTimeoutFlag::SmtOption))),
        "cvc5" | "cvc5-int" => {
            Some((&["--incremental"][..], Some(SessionTimeoutFlag::Joined("--tlimit-per="))))
        }
        "bitwuzla" | "bitwuzla-abs" | "bitwuzla-prop" => {
            Some((&[][..], Some(SessionTimeoutFlag::Separate("-T"))))
        }
        _ => None,
    };
    let name = match solver {
        "z3" => Some("z3"),
        "z3-inc" => Some("z3-inc"),
        "yices" => Some("yices"),
        "cvc5" => Some("cvc5"),
        "cvc5-int" => Some("cvc5-int"),
        "bitwuzla" => Some("bitwuzla"),
        "bitwuzla-abs" => Some("bitwuzla-abs"),
        "bitwuzla-prop" => Some("bitwuzla-prop"),
        _ => None,
    };
    let parts = parts.into_iter().map(str::to_string).collect::<Vec<_>>();
    let command = SolverCommand::new(parts, smt_timeout)?;
    Ok(match (name, session) {
        (Some(name), Some((args, flag))) => command.known(name, args, flag),
        (Some(name), None) => SolverCommand { name: Some(name), ..command },
        _ => command,
    })
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
    /// Position in this query's launch order.
    index: usize,
    /// Configured index, which selects the solver's session slot.
    slot: usize,
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
///
/// `commands` pairs each solver with its configured index, which selects its slot in
/// `sessions`; the sessions still usable are returned with the result. A `sat` answer whose model
/// fails validation is not decisive, and the first other `sat` or `unsat` answer cancels the rest.
fn run_solver_commands(
    commands: &[(usize, SolverCommand)],
    smt: &str,
    timeout: Option<u32>,
    validate_model: Option<(&SymCx, &[SymBoolExpr])>,
    mut sessions: Vec<Option<SolverSession>>,
    stop: Option<&AtomicBool>,
) -> (SolverCommandRun, Vec<Option<SolverSession>>) {
    let stopped = || stop.is_some_and(|stop| stop.load(Ordering::SeqCst));
    if stopped() {
        let output = Err(SymbolicError::Solver(BATCH_STOPPED.to_string()));
        return (SolverCommandRun { output, summaries: Vec::new() }, sessions);
    }
    if commands.is_empty() {
        let output = Err(SymbolicError::Solver("symbolic solver portfolio is empty".to_string()));
        return (SolverCommandRun { output, summaries: Vec::new() }, sessions);
    }
    let slots = commands.iter().map(|(index, _)| index + 1).max().unwrap_or_default();
    if sessions.len() < slots {
        sessions.resize_with(slots, || None);
    }
    if let [(index, command)] = commands {
        let never = AtomicBool::new(false);
        let (outcome, session) = run_solver_query(
            command,
            smt,
            timeout,
            stop.unwrap_or(&never),
            sessions[*index].take(),
        );
        sessions[*index] = session;
        let output = match outcome {
            SolverProcessOutcome::Cancelled if stopped() => {
                Err(SymbolicError::Solver(BATCH_STOPPED.to_string()))
            }
            outcome => outcome.into_result(),
        };
        return (SolverCommandRun { output, summaries: Vec::new() }, sessions);
    }

    let cancel = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();
    let run = thread::scope(|scope| {
        let started_at = Instant::now();
        let mut pending = scheduled_portfolio(commands);
        let mut running = 0usize;

        let mut saw_unknown = false;
        let mut saw_invalid_sat_model = false;
        let mut errors = Vec::new();
        let mut decisive = None;
        let mut summaries = Vec::new();
        let mut batch_stopped = false;

        while running > 0 || !pending.is_empty() {
            // Another query of the batch already found a satisfiable pair: stop this race.
            if decisive.is_none() && !batch_stopped && stopped() {
                batch_stopped = true;
                cancel.store(true, Ordering::SeqCst);
                while let Some(solver) = pending.pop_front() {
                    summaries.push(summary_for_unstarted_solver(solver));
                }
            }
            if decisive.is_none() && !batch_stopped {
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
                    let session = sessions[solver.slot].take();
                    // A solver started late gets only what is left of the query's limit.
                    let timeout = remaining_timeout(timeout, started_after);
                    running += 1;
                    launched = true;
                    scope.spawn(move || {
                        let start = Instant::now();
                        let (outcome, session) = match timeout {
                            Some(0) => (SolverProcessOutcome::Unknown, session),
                            timeout => {
                                run_solver_query(&solver.command, smt, timeout, &cancel, session)
                            }
                        };
                        let _ = tx.send((
                            SolverProcessResult {
                                index: solver.index,
                                display: solver.command.display,
                                scheduled_after: solver.launch_after,
                                started_after,
                                elapsed: start.elapsed(),
                                outcome,
                            },
                            solver.slot,
                            session,
                        ));
                    });
                }
            }

            if running == 0 {
                continue;
            }

            let result = if decisive.is_none() && !batch_stopped {
                // Wake up regularly to notice the batch stopping.
                let launch_wait = next_portfolio_launch_wait(started_at, &pending);
                let wait = match (launch_wait, stop) {
                    (Some(wait), Some(_)) => Some(wait.min(BATCH_STOP_POLL_INTERVAL)),
                    (None, Some(_)) => Some(BATCH_STOP_POLL_INTERVAL),
                    (wait, None) => wait,
                };
                wait.map_or_else(|| rx.recv().ok(), |wait| rx.recv_timeout(wait).ok())
            } else {
                rx.recv().ok()
            };
            let Some((result, slot, session)) = result else {
                continue;
            };
            sessions[slot] = session;
            running = running.saturating_sub(1);
            let SolverProcessResult {
                index,
                display,
                scheduled_after,
                started_after,
                elapsed,
                outcome,
            } = result;
            if decisive.is_some() || batch_stopped {
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
            let first_line = match &outcome {
                SolverProcessOutcome::Output(output) => {
                    output.lines().next().unwrap_or_default().trim().to_string()
                }
                _ => String::new(),
            };
            match outcome {
                SolverProcessOutcome::Output(output) if first_line == "sat" => {
                    if let Some((cx, constraints)) = validate_model
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
                }
                SolverProcessOutcome::Output(output) if first_line == "unsat" => {
                    summaries.push(
                        SolverRunSummary::new(display, elapsed, SolverOutcome::Unsat)
                            .with_schedule(index, scheduled_after, Some(started_after))
                            .winner(),
                    );
                    decisive = Some(output);
                }
                SolverProcessOutcome::Output(_) if first_line == "unknown" => {
                    summaries.push(
                        SolverRunSummary::new(display, elapsed, SolverOutcome::Unknown)
                            .with_schedule(index, scheduled_after, Some(started_after)),
                    );
                    saw_unknown = true;
                }
                SolverProcessOutcome::Output(_) => {
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
            if decisive.is_some() {
                cancel.store(true, Ordering::SeqCst);
                while let Some(solver) = pending.pop_front() {
                    summaries.push(summary_for_unstarted_solver(solver));
                }
            }
        }

        let output = if let Some(output) = decisive {
            Ok(output)
        } else if batch_stopped {
            Err(SymbolicError::Solver(BATCH_STOPPED.to_string()))
        } else if saw_invalid_sat_model {
            Err(SymbolicError::Solver(errors.join("; ")))
        } else if saw_unknown {
            Err(SymbolicError::SolverUnknown)
        } else {
            Err(SymbolicError::Solver(errors.join("; ")))
        };

        SolverCommandRun { output, summaries }
    });
    (run, sessions)
}

/// Returns the staged launch plan for a configured portfolio.
fn scheduled_portfolio(commands: &[(usize, SolverCommand)]) -> VecDeque<ScheduledSolver> {
    commands
        .iter()
        .cloned()
        .enumerate()
        .map(|(index, (slot, command))| ScheduledSolver {
            index,
            slot,
            command,
            launch_after: portfolio_launch_delay(index),
        })
        .collect()
}

/// Returns what is left of `timeout`, in whole seconds rounded up, once `elapsed` has passed:
/// `Some(0)` when nothing is, and `None` for no limit.
fn remaining_timeout(timeout: Option<u32>, elapsed: Duration) -> Option<u32> {
    let seconds = timeout.filter(|seconds| *seconds > 0)?;
    let left = Duration::from_secs(seconds.into()).saturating_sub(elapsed);
    if left.is_zero() {
        return Some(0);
    }
    Some(u32::try_from(left.as_secs_f64().ceil() as u64).unwrap_or(u32::MAX))
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
