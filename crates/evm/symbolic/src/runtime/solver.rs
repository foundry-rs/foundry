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

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SolverCommand {
    pub(crate) program: String,
    pub(crate) args: Vec<String>,
    pub(crate) display: String,
    pub(crate) smt_timeout: bool,
}

impl SolverCommand {
    /// Constructs a solver command from a program plus arguments.
    pub(crate) fn new(parts: Vec<String>, smt_timeout: bool) -> Result<Self, String> {
        let mut parts = parts.into_iter();
        let Some(program) = parts.next().filter(|part| !part.is_empty()) else {
            return Err("symbolic solver command is empty".to_string());
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
    pub(crate) commands: Result<Vec<SolverCommand>, String>,
    pub(crate) timeout: Option<u32>,
    pub(crate) max_queries: usize,
    pub(crate) queries: usize,
    pub(crate) dump_smt: bool,
}

impl SmtLibSubprocessSolver {
    /// Constructs a new instance.
    pub(crate) const fn new(
        commands: Result<Vec<SolverCommand>, String>,
        timeout: Option<u32>,
        max_queries: usize,
        dump_smt: bool,
    ) -> Self {
        Self { commands, timeout, max_queries, queries: 0, dump_smt }
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
        SymbolicStats { paths: 0, solver_queries: self.queries }
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
            other => Err(SymbolicError::Solver(format!("unexpected solver response `{other}`"))),
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
            "sat" => {
                let model = parse_model(&output)?;
                if constraints
                    .iter()
                    .all(|constraint| eval_bool_expr(constraint, &model).unwrap_or(false))
                {
                    Ok(model)
                } else {
                    Err(SymbolicError::Solver(
                        "solver model does not satisfy path constraints".to_string(),
                    ))
                }
            }
            "unsat" => Err(SymbolicError::Solver("counterexample path became unsat".to_string())),
            "unknown" => fallback_single_var_model(constraints).ok_or(SymbolicError::SolverUnknown),
            other => Err(SymbolicError::Solver(format!("unexpected solver response `{other}`"))),
        }
    }
}

impl SmtLibSubprocessSolver {
    /// Returns the resolved commands or the stored config error.
    pub(crate) fn commands(&self) -> Result<&[SolverCommand], SymbolicError> {
        self.commands.as_ref().map(Vec::as_slice).map_err(|err| SymbolicError::Solver(err.clone()))
    }

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

        let commands = self.commands()?;
        let mut smt = String::new();
        smt.push_str("(set-logic QF_BV)\n");
        if commands.iter().all(|command| command.smt_timeout)
            && let Some(timeout) = self.timeout.filter(|timeout| *timeout > 0)
        {
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

        run_solver_commands(commands, &smt, self.timeout)
    }
}

/// Returns the subprocess commands for the configured SMT solver setup.
pub(crate) fn solver_commands_for_config(
    config: &SymbolicConfig,
) -> Result<Vec<SolverCommand>, String> {
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
pub(crate) fn named_solver_command(solver: &str) -> Result<SolverCommand, String> {
    let (parts, smt_timeout) = match solver {
        "z3" => (vec!["z3", "-in", "-smt2"], true),
        "yices" | "yices-2.6.4" | "yices-2.6.5" => {
            (vec!["yices-smt2", "--smt2-model-format", "--bvconst-in-decimal"], false)
        }
        "cvc5" | "cvc5-1.2.1" => (
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
        "bitwuzla" | "bitwuzla-0.8.1" => (vec!["bitwuzla", "--produce-models"], false),
        "bitwuzla-abs" => (vec!["bitwuzla", "--produce-models", "--abstraction"], false),
        // Preserve existing behavior for custom z3-compatible executable names/paths.
        custom => (vec![custom, "-in", "-smt2"], true),
    };
    let parts = parts.into_iter().map(str::to_string).collect::<Vec<_>>();
    SolverCommand::new(parts, smt_timeout)
}

/// Returns the command for one configured portfolio entry.
pub(crate) fn solver_command_for_portfolio_entry(entry: &str) -> Result<SolverCommand, String> {
    if entry.chars().any(|ch| ch.is_whitespace() || matches!(ch, '"' | '\'' | '\\')) {
        SolverCommand::new(split_solver_command(entry)?, false)
    } else {
        named_solver_command(entry)
    }
}

/// Splits a shell-like solver command into argv parts.
pub(crate) fn split_solver_command(command: &str) -> Result<Vec<String>, String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;

    for ch in command.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if let Some(quote_ch) = quote {
            if ch == quote_ch {
                quote = None;
            } else {
                current.push(ch);
            }
            continue;
        }
        if matches!(ch, '"' | '\'') {
            quote = Some(ch);
        } else if ch.is_whitespace() {
            if !current.is_empty() {
                parts.push(std::mem::take(&mut current));
            }
        } else {
            current.push(ch);
        }
    }

    if let Some(quote_ch) = quote {
        return Err(format!("unterminated {quote_ch} quote in symbolic solver command"));
    }
    if escaped {
        current.push('\\');
    }
    if !current.is_empty() {
        parts.push(current);
    }

    if parts.is_empty() {
        return Err("symbolic solver command is empty".to_string());
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

/// Runs one or more solver commands and returns the first decisive SMT-LIB response.
fn run_solver_commands(
    commands: &[SolverCommand],
    smt: &str,
    timeout: Option<u32>,
) -> Result<String, SymbolicError> {
    if commands.is_empty() {
        return Err(SymbolicError::Solver("symbolic solver portfolio is empty".to_string()));
    }
    if commands.len() == 1 {
        return match run_solver_process(&commands[0], smt, timeout, &AtomicBool::new(false)) {
            SolverProcessOutcome::Output(output) => Ok(output),
            SolverProcessOutcome::Unknown => Err(SymbolicError::SolverUnknown),
            SolverProcessOutcome::Cancelled => {
                Err(SymbolicError::Solver("solver query was cancelled".to_string()))
            }
            SolverProcessOutcome::Error(err) => Err(SymbolicError::Solver(err)),
        };
    }

    let cancel = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();
    thread::scope(|scope| {
        for command in commands.iter().cloned() {
            let tx = tx.clone();
            let cancel = Arc::clone(&cancel);
            scope.spawn(move || {
                let outcome = run_solver_process(&command, smt, timeout, &cancel);
                let _ = tx.send((command.display, outcome));
            });
        }
        drop(tx);

        let mut saw_unknown = false;
        let mut errors = Vec::new();
        let mut decisive = None;
        while let Ok((display, outcome)) = rx.recv() {
            match outcome {
                SolverProcessOutcome::Output(output) if solver_output_is_decisive(&output) => {
                    decisive = Some(output);
                    cancel.store(true, Ordering::SeqCst);
                    break;
                }
                SolverProcessOutcome::Output(output) if solver_output_is_unknown(&output) => {
                    saw_unknown = true;
                }
                SolverProcessOutcome::Output(output) => {
                    errors.push(format!(
                        "{display}: unexpected solver response `{}`",
                        first_solver_line(&output)
                    ));
                }
                SolverProcessOutcome::Unknown => saw_unknown = true,
                SolverProcessOutcome::Cancelled => {}
                SolverProcessOutcome::Error(err) => errors.push(format!("{display}: {err}")),
            }
        }

        if decisive.is_some() {
            while rx.recv().is_ok() {}
        }

        if let Some(output) = decisive {
            Ok(output)
        } else if saw_unknown {
            Err(SymbolicError::SolverUnknown)
        } else {
            Err(SymbolicError::Solver(errors.join("; ")))
        }
    })
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
            Ok(None) => thread::sleep(Duration::from_millis(10)),
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

fn solver_output_is_decisive(output: &str) -> bool {
    matches!(first_solver_line(output), "sat" | "unsat")
}

fn solver_output_is_unknown(output: &str) -> bool {
    first_solver_line(output) == "unknown"
}

fn first_solver_line(output: &str) -> &str {
    output.lines().next().unwrap_or_default().trim()
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
