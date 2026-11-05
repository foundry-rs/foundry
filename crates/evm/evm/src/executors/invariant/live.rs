//! Bounded, asynchronous assistance for an invariant campaign. Candidates are executed by the
//! ordinary campaign before they can contribute coverage or failures.

use super::RecordedCallSequence;
use crate::{
    executors::{
        EarlyExit, EvmExecutionCancellation, Executor,
        fuzz::{FuzzBranchFrontier, FuzzFrontierRecorder, comparison_result},
    },
    inspectors::CmpOperands,
};
use alloy_primitives::{Address, map::HashSet};
use eyre::Result;
use foundry_config::SymbolicConfig;
use foundry_evm_core::evm::FoundryEvmNetwork;
use foundry_evm_fuzz::BasicTxDetails;
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const MAX_PENDING: usize = 32;
const MAX_CALLS: usize = 1024;
const MAX_CALLDATA_BYTES: usize = 64 * 1024;
/// Distinct comparison outcomes remembered to select solve targets. Real targets reach far more
/// outcomes than `frontier_limit` in their first sequence, so this bound is separate from the
/// submission budget.
const MAX_OBSERVED_OUTCOMES: usize = 1 << 16;

/// One side of a comparison site: address, pc, opcode, and result.
type Outcome = (Address, usize, u8, bool);

/// Counters for the live worker, separate from bounded symbolic verification results.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LiveInvariantStats {
    /// Frontiers accepted into the request queue.
    pub submitted: usize,
    /// Frontiers examined by the worker.
    pub completed: usize,
    /// Total queries issued to the solver.
    pub solver_queries: usize,
    /// Total time spent in solver queries, in milliseconds.
    pub solver_time_ms: u64,
    /// Candidate sequences handed to the concrete campaign.
    pub scheduled: usize,
}

/// A solver response. The campaign re-executes the candidate using its own predicate policy.
#[derive(Default)]
pub struct LiveInvariantSolution {
    /// Candidate sequence, still subject to normal campaign execution.
    pub sequence: Option<Vec<BasicTxDetails>>,
    /// Total queries issued to the solver.
    pub solver_queries: usize,
    /// Total time spent in solver queries, in milliseconds.
    pub solver_time_ms: u64,
}

/// One solver thread, 32 retained frontiers, and one slot in each channel.
pub struct LiveInvariantSolver {
    requests: Option<SyncSender<FuzzBranchFrontier>>,
    responses: Receiver<(Vec<BasicTxDetails>, Outcome)>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<LiveInvariantStats>>,
    observed: HashSet<Outcome>,
    /// Outcomes already queued for the worker; each is attempted once unless it is flipped.
    admitted: HashSet<Outcome>,
    /// Admitted outcomes for which the worker returned a candidate.
    flipped: HashSet<Outcome>,
    /// Flipped outcomes already retried from a storage state.
    retried: HashSet<(Outcome, u64)>,
    /// Queued frontiers, flagged when they retry an admitted outcome from a new prefix.
    pending: VecDeque<(FuzzBranchFrontier, bool)>,
    limit: usize,
    deadline: Instant,
    stats: LiveInvariantStats,
}

impl LiveInvariantSolver {
    /// Starts one worker with campaign-wide time, query, and frontier budgets.
    pub fn spawn<FEN: FoundryEvmNetwork>(
        setup: Executor<FEN>,
        config: &SymbolicConfig,
        mut solve: impl FnMut(Executor<FEN>, &FuzzBranchFrontier, Instant, u32) -> LiveInvariantSolution
        + Send
        + 'static,
    ) -> Result<Self> {
        let (requests, input) = mpsc::sync_channel::<FuzzBranchFrontier>(1);
        let (output, responses) = mpsc::sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let cancelled = Arc::clone(&stop);
        let deadline = Instant::now() + Duration::from_secs(config.timeout.unwrap_or(30).into());
        let max_queries = config.max_solver_queries as usize;
        let thread = thread::Builder::new().name("invariant-symbolic".into()).spawn(move || {
            let mut stats = LiveInvariantStats::default();
            while !cancelled.load(Ordering::Relaxed)
                && Instant::now() < deadline
                && stats.solver_queries < max_queries
            {
                let frontier = match input.recv_timeout(Duration::from_millis(10)) {
                    Ok(frontier) => frontier,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                };
                let mut executor = setup.clone();
                executor.inspector_mut().set_execution_cancellation(
                    EvmExecutionCancellation::campaign(
                        EarlyExit::new(false),
                        Arc::clone(&cancelled),
                        Some(deadline),
                    ),
                );
                let result = solve(
                    executor,
                    &frontier,
                    deadline,
                    (max_queries - stats.solver_queries) as u32,
                );
                stats.completed += 1;
                stats.solver_queries += result.solver_queries;
                stats.solver_time_ms += result.solver_time_ms;
                if let Some(sequence) = result.sequence {
                    let (cmp, result) = frontier.comparison();
                    let mut response = (sequence, outcome(&cmp, result));
                    // Preserve useful responses under backpressure, while keeping shutdown
                    // bounded when the concrete campaign has already stopped consuming.
                    while !cancelled.load(Ordering::Relaxed) && Instant::now() < deadline {
                        match output.try_send(response) {
                            Ok(()) | Err(mpsc::TrySendError::Disconnected(_)) => break,
                            Err(mpsc::TrySendError::Full(pending)) => response = pending,
                        }
                        thread::sleep(Duration::from_millis(1));
                    }
                }
            }
            stats
        })?;
        Ok(Self {
            requests: (max_queries > 0 && config.frontier_limit > 0).then_some(requests),
            responses,
            stop,
            thread: Some(thread),
            observed: HashSet::default(),
            admitted: HashSet::default(),
            flipped: HashSet::default(),
            retried: HashSet::default(),
            pending: VecDeque::new(),
            limit: config.frontier_limit,
            deadline,
            stats: LiveInvariantStats::default(),
        })
    }

    /// Offers a completed run's comparisons. `states` lists the storage states the run reached
    /// in call order, as the index of the call that produced each and a feature identifying it.
    pub(super) fn submit(
        &mut self,
        sequence: &RecordedCallSequence,
        depth: usize,
        states: &[(usize, u64)],
    ) {
        if !self.is_active()
            || !Self::accepts_sequence(
                sequence.inputs.len(),
                sequence.inputs.iter().map(|tx| tx.call_details.calldata.len()).sum(),
            )
        {
            return;
        }
        let cmp_seq = &sequence.cmp_seq[..sequence.cmp_seq.len().min(depth)];
        for cmp in cmp_seq.iter().flatten() {
            if self.observed.len() >= MAX_OBSERVED_OUTCOMES {
                break;
            }
            self.observed.insert(outcome(cmp, comparison_result(cmp)));
        }
        self.dispatch();
        let room = (MAX_PENDING - self.pending.len())
            .min(self.limit - self.stats.submitted - self.pending.len());
        if room > 0 {
            let state_before = |index: usize| {
                states.iter().rev().find(|(call, _)| *call < index).map(|&(_, state)| state)
            };
            // A first attempt needs an outcome whose other side fuzzing has not reached; one that
            // does not fit stays unadmitted, so a later run offers it again. A flipped outcome is
            // retried once from each new storage state, which may need different arguments, unless
            // a retry of it is already queued.
            let targets = cmp_seq
                .iter()
                .enumerate()
                .map(|(index, cmps)| {
                    cmps.iter()
                        .copied()
                        .filter(|cmp| {
                            let result = comparison_result(cmp);
                            let reached = outcome(cmp, result);
                            self.observed.contains(&reached)
                                && if self.admitted.contains(&reached) {
                                    self.flipped.contains(&reached)
                                        && state_before(index).is_some_and(|state| {
                                            !self.retried.contains(&(reached, state))
                                        })
                                        && !self.pending.iter().any(|(frontier, _)| {
                                            let (cmp, result) = frontier.comparison();
                                            outcome(&cmp, result) == reached
                                        })
                                } else {
                                    !self.observed.contains(&outcome(cmp, !result))
                                }
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            let mut recorder = FuzzFrontierRecorder::new(room);
            recorder.prefer_shortest_prefix();
            recorder.capture_sequence(&sequence.inputs, &targets);
            for frontier in recorder.into_frontiers() {
                let (cmp, result) = frontier.comparison();
                let reached = outcome(&cmp, result);
                let retry = !self.admitted.insert(reached);
                if retry && let Some(state) = state_before(frontier.sequence().len() - 1) {
                    self.retried.insert((reached, state));
                }
                self.pending.push_back((frontier, retry));
            }
        }
        self.dispatch();
    }

    fn dispatch(&mut self) {
        // Fuzzing may have reached the other side of a first attempt since it was admitted.
        self.pending.retain(|(frontier, retry)| {
            let (cmp, result) = frontier.comparison();
            *retry || !self.observed.contains(&outcome(&cmp, !result))
        });
        while let Some((frontier, retry)) = self.pending.pop_front() {
            let Some(requests) = &self.requests else { break };
            match requests.try_send(frontier) {
                Ok(()) => self.stats.submitted += 1,
                Err(mpsc::TrySendError::Full(frontier)) => {
                    self.pending.push_front((frontier, retry));
                    break;
                }
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    self.requests = None;
                    self.pending.clear();
                    break;
                }
            }
        }
    }

    pub(super) fn next(&mut self, depth: usize) -> Option<Vec<BasicTxDetails>> {
        self.dispatch();
        let (sequence, flipped) = self.responses.try_recv().ok()?;
        self.flipped.insert(flipped);
        if sequence.is_empty() || sequence.len() > depth {
            return None;
        }
        self.stats.scheduled += 1;
        Some(sequence)
    }

    pub(super) fn finish(&mut self) -> LiveInvariantStats {
        self.stop.store(true, Ordering::Relaxed);
        self.requests = None;
        if let Some(thread) = self.thread.take() {
            match thread.join() {
                Ok(stats) => {
                    self.stats.completed = stats.completed;
                    self.stats.solver_queries = stats.solver_queries;
                    self.stats.solver_time_ms = stats.solver_time_ms;
                }
                Err(_) => {
                    tracing::warn!("live symbolic worker panicked; concrete campaign continued")
                }
            }
        }
        self.stats.clone()
    }

    /// Whether recording more frontiers can still contribute work to this campaign.
    pub(super) fn is_active(&self) -> bool {
        self.requests.is_some()
            && self.stats.submitted + self.pending.len() < self.limit
            && Instant::now() < self.deadline
            && self.thread.as_ref().is_some_and(|thread| !thread.is_finished())
    }

    /// Applies the same input bounds during collection and before channel admission.
    pub(super) const fn accepts_sequence(calls: usize, calldata_bytes: usize) -> bool {
        calls <= MAX_CALLS && calldata_bytes <= MAX_CALLDATA_BYTES
    }
}

impl Drop for LiveInvariantSolver {
    fn drop(&mut self) {
        self.finish();
    }
}

/// Identifies one side of a comparison site.
const fn outcome(cmp: &CmpOperands, result: bool) -> Outcome {
    (cmp.address, cmp.pc, cmp.opcode, result)
}
