//! Bounded, asynchronous assistance for an invariant campaign. Candidates are executed by the
//! ordinary campaign before they can contribute coverage or failures.

use super::RecordedCallSequence;
use crate::executors::{
    EarlyExit, EvmExecutionCancellation, Executor,
    fuzz::{FuzzBranchFrontier, FuzzFrontierRecorder, comparison_result},
};
use alloy_primitives::{Address, B256, keccak256, map::HashSet};
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
    responses: Receiver<Vec<BasicTxDetails>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<LiveInvariantStats>>,
    seen: HashSet<B256>,
    observed: HashSet<(Address, usize, u8, bool)>,
    pending: VecDeque<FuzzBranchFrontier>,
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
                if let Some(mut sequence) = result.sequence {
                    // Preserve useful responses under backpressure, while keeping shutdown
                    // bounded when the concrete campaign has already stopped consuming.
                    while !cancelled.load(Ordering::Relaxed) && Instant::now() < deadline {
                        match output.try_send(sequence) {
                            Ok(()) | Err(mpsc::TrySendError::Disconnected(_)) => break,
                            Err(mpsc::TrySendError::Full(pending)) => sequence = pending,
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
            seen: HashSet::default(),
            observed: HashSet::default(),
            pending: VecDeque::new(),
            limit: config.frontier_limit,
            deadline,
            stats: LiveInvariantStats::default(),
        })
    }

    pub(super) fn submit(&mut self, sequence: &RecordedCallSequence, depth: usize) {
        if !self.is_active()
            || !Self::accepts_sequence(
                sequence.inputs.len(),
                sequence.inputs.iter().map(|tx| tx.call_details.calldata.len()).sum(),
            )
        {
            return;
        }
        let len = sequence.cmp_seq.len().min(depth);
        let mut novel = false;
        for cmp in sequence.cmp_seq[..len].iter().flatten() {
            if self.observed.len() >= self.limit {
                break;
            }
            novel |=
                self.observed.insert((cmp.address, cmp.pc, cmp.opcode, comparison_result(cmp)));
        }
        // Feed back newly reached control flow, rather than spending the budget on random
        // no-op prefixes that repeatedly reach the same comparison outcomes.
        if novel {
            let mut recorder = FuzzFrontierRecorder::new(self.limit - self.stats.submitted);
            recorder.prefer_shortest_prefix();
            recorder.capture_sequence(&sequence.inputs, &sequence.cmp_seq[..len]);
            for frontier in recorder.into_frontiers() {
                if self.pending.len() == MAX_PENDING || self.seen.len() == self.limit {
                    break;
                }
                let calls = frontier.sequence();
                let call = calls.last().expect("frontier contains a call");
                let (cmp, result) = frontier.comparison();
                let key = keccak256(
                    serde_json::to_vec(&(
                        &calls[..calls.len() - 1],
                        call.sender,
                        call.call_details.target,
                        call.call_details.calldata.get(..4),
                        cmp.address,
                        cmp.pc,
                        cmp.opcode,
                        result,
                    ))
                    .expect("frontier identity serializes"),
                );
                if self.seen.insert(key) {
                    self.pending.push_back(frontier);
                }
            }
        }
        self.dispatch();
    }

    fn dispatch(&mut self) {
        while let Some(frontier) = self.pending.pop_front() {
            let Some(requests) = &self.requests else { break };
            match requests.try_send(frontier) {
                Ok(()) => self.stats.submitted += 1,
                Err(mpsc::TrySendError::Full(frontier)) => {
                    self.pending.push_front(frontier);
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
        let sequence = self.responses.try_recv().ok()?;
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
            && self.seen.len() < self.limit
            && self.observed.len() < self.limit
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
