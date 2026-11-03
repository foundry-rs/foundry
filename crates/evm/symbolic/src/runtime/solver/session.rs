//! Long-lived solver processes that answer one query after another.

use super::*;

/// Marker the session echoes after each query's responses.
const SESSION_QUERY_END: &str = "foundry-query-complete";

/// A solver process kept alive across queries.
///
/// Each query runs inside `(push 1)` and `(pop 1)`, or after `(reset)` for solvers whose scoped
/// mode is slow (see [`SolverCommand::session_resets`]), so declarations and assertions do not
/// leak into the next one. Reusing the process saves the solver's start-up time, which dominates
/// small queries. A session that times out, is cancelled or fails is dropped, which kills the
/// process.
pub(super) struct SolverSession {
    child: SolverChild,
    /// Feeds the solver's stdin from its own thread, so a solver that stops reading never blocks
    /// the query loop, which must keep watching for cancellation and timeouts.
    stdin: Option<mpsc::Sender<Vec<u8>>>,
    stdin_thread: Option<JoinHandle<()>>,
    stdout: Receiver<Result<String, String>>,
    stderr: Receiver<String>,
    stdout_thread: Option<JoinHandle<()>>,
    stderr_thread: Option<JoinHandle<()>>,
}

impl SolverSession {
    /// Starts a session for `command`, which must be session capable.
    pub(super) fn spawn(command: &SolverCommand, timeout: Option<u32>) -> Result<Self, String> {
        let mut child = spawn_solver_process(&command.session_command(timeout))?;
        let mut stdin = child.child_mut().stdin.take().expect("piped solver stdin is available");
        let stdout = child.child_mut().stdout.take().expect("piped solver stdout is available");
        let stderr = child.child_mut().stderr.take().expect("piped solver stderr is available");

        let (stdin_tx, stdin_rx) = mpsc::channel::<Vec<u8>>();
        let stdin_thread = thread::spawn(move || {
            for bytes in stdin_rx {
                if stdin.write_all(&bytes).and_then(|()| stdin.flush()).is_err() {
                    break;
                }
            }
        });
        stdin_tx
            .send(b"(set-logic QF_BV)\n".to_vec())
            .map_err(|err| format!("failed to start solver session: {err}"))?;

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
            stdin: Some(stdin_tx),
            stdin_thread: Some(stdin_thread),
            stdout: stdout_rx,
            stderr: stderr_rx,
            stdout_thread: Some(stdout_thread),
            stderr_thread: Some(stderr_thread),
        })
    }

    /// Runs one query and returns the solver's responses to it.
    ///
    /// `smt` is a whole query as [`SmtLibSubprocessSolver`] builds it; its `set-logic` line is
    /// dropped because the session set the logic once.
    pub(super) fn query(
        &mut self,
        command: &SolverCommand,
        smt: &str,
        timeout: Option<u32>,
        cancel: &AtomicBool,
    ) -> SolverProcessOutcome {
        let resets = command.session_resets();
        let body =
            if resets { smt } else { smt.strip_prefix("(set-logic QF_BV)\n").unwrap_or(smt) };
        let mut query = Vec::with_capacity(body.len() + 96);
        query.extend_from_slice(if resets { b"(reset)\n" } else { b"(push 1)\n" });
        if command.session_timeout_flag == Some(SessionTimeoutFlag::SmtOption)
            && let Some(seconds) = timeout.filter(|seconds| *seconds > 0)
        {
            let _ = writeln!(query, "(set-option :timeout {})", u64::from(seconds) * 1000);
        }
        query.extend_from_slice(body.as_bytes());
        let _ = writeln!(query, "(echo \"{SESSION_QUERY_END}\")");
        if !resets {
            query.extend_from_slice(b"(pop 1)\n");
        }
        if self.stdin.as_ref().is_none_or(|stdin| stdin.send(query).is_err()) {
            return SolverProcessOutcome::Error("failed to write solver query".to_string());
        }

        let started_at = Instant::now();
        // Solvers that enforce the per-query limit themselves answer `unknown` and stay usable;
        // the wall-clock limit, a little later, only catches a solver that ignores it.
        let timeout = timeout.filter(|seconds| *seconds > 0).map(|seconds| {
            let limit = Duration::from_secs(seconds.into());
            if command.session_timeout_flag.is_some() {
                limit + SESSION_TIMEOUT_GRACE
            } else {
                limit
            }
        });
        let mut output = String::new();
        loop {
            if cancel.load(Ordering::SeqCst) {
                return SolverProcessOutcome::Cancelled;
            }
            let Some(wait) = solver_wait_duration(started_at.elapsed(), timeout) else {
                return SolverProcessOutcome::Unknown;
            };
            match self.stdout.recv_timeout(wait) {
                Ok(Ok(line)) if line.trim().trim_matches('"') == SESSION_QUERY_END => {
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

impl Drop for SolverSession {
    fn drop(&mut self) {
        self.stdin.take();
        self.child.terminate();
        if let Some(thread) = self.stdin_thread.take() {
            let _ = thread.join();
        }
        if let Some(thread) = self.stdout_thread.take() {
            let _ = thread.join();
        }
        if let Some(thread) = self.stderr_thread.take() {
            let _ = thread.join();
        }
    }
}

/// Runs `smt` on `command`, reusing and returning `session` when the command supports one.
///
/// The returned session is `None` when the query left it unusable or the command runs one process
/// per query.
pub(super) fn run_solver_query(
    command: &SolverCommand,
    smt: &str,
    timeout: Option<u32>,
    cancel: &AtomicBool,
    session: Option<SolverSession>,
) -> (SolverProcessOutcome, Option<SolverSession>) {
    if !command.session_capable() {
        return (run_solver_process(command, smt, timeout, cancel), None);
    }
    let mut session = match session {
        Some(session) => session,
        None => match SolverSession::spawn(command, timeout) {
            Ok(session) => session,
            Err(err) => return (SolverProcessOutcome::Error(err), None),
        },
    };
    let started_at = Instant::now();
    match session.query(command, smt, timeout, cancel) {
        output @ SolverProcessOutcome::Output(_) => (output, Some(session)),
        SolverProcessOutcome::Error(_) if !cancel.load(Ordering::SeqCst) => {
            // A broken session should not fail the query: retry it in a fresh process, within
            // what is left of the query's limit.
            drop(session);
            match remaining_timeout(timeout, started_at.elapsed()) {
                Some(0) => (SolverProcessOutcome::Unknown, None),
                timeout => (run_solver_process(command, smt, timeout, cancel), None),
            }
        }
        other => (other, None),
    }
}
