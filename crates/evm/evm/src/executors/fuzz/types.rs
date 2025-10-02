use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicU32, Ordering},
};

use crate::executors::{FailFast, FuzzTestTimer, RawCallResult, corpus::GlobalCorpusMetrics};
use alloy_primitives::{Bytes, Log, map::HashMap};
use foundry_common::evm::Breakpoints;
use foundry_evm_coverage::HitMaps;
use foundry_evm_fuzz::FuzzCase;
use foundry_evm_traces::SparsedTraceArena;
use proptest::prelude::TestCaseError;
use revm::interpreter::InstructionResult;

/// Returned by a single fuzz in the case of a successful run
#[derive(Debug)]
pub struct CaseOutcome {
    /// Data of a single fuzz test case.
    pub case: FuzzCase,
    /// The traces of the call.
    pub traces: Option<SparsedTraceArena>,
    /// The coverage info collected during the call.
    pub coverage: Option<HitMaps>,
    /// Breakpoints char pc map.
    pub breakpoints: Breakpoints,
    /// logs of a single fuzz test case.
    pub logs: Vec<Log>,
    // Deprecated cheatcodes mapped to their replacements.
    pub deprecated_cheatcodes: HashMap<&'static str, Option<&'static str>>,
}

/// Returned by a single fuzz when a counterexample has been discovered
#[derive(Debug)]
pub struct CounterExampleOutcome {
    /// Minimal reproduction test case for failing test.
    pub counterexample: (Bytes, RawCallResult),
    /// The status of the call.
    pub exit_reason: Option<InstructionResult>,
    /// Breakpoints char pc map.
    pub breakpoints: Breakpoints,
}

/// Outcome of a single fuzz
#[derive(Debug)]
#[expect(clippy::large_enum_variant)]
pub enum FuzzOutcome {
    Case(CaseOutcome),
    CounterExample(CounterExampleOutcome),
}

/// Shared state for coordinating parallel fuzz workers
pub struct SharedFuzzState {
    /// Total runs across workers
    total_runs: Arc<AtomicU32>,
    /// Found failure
    ///
    /// The worker that found the failure sets it's ID.
    ///
    /// This ID is then used to correctly extract the failure reason and counterexample.
    found_failure: OnceLock<u32>,
    /// Maximum number of runs
    max_runs: u32,
    /// Total rejects across workers
    total_rejects: Arc<AtomicU32>,
    /// Fuzz timer
    timer: FuzzTestTimer,
    /// Fail Fast coordinator
    fail_fast: FailFast,
    /// Global corpus metrics
    pub(crate) global_corpus_metrics: GlobalCorpusMetrics,
}

impl SharedFuzzState {
    pub fn new(max_runs: u32, timeout: Option<u32>, fail_fast: FailFast) -> Self {
        Self {
            total_runs: Arc::new(AtomicU32::new(0)),
            found_failure: OnceLock::new(),
            max_runs,
            total_rejects: Arc::new(AtomicU32::new(0)),
            timer: FuzzTestTimer::new(timeout),
            fail_fast,
            global_corpus_metrics: GlobalCorpusMetrics::default(),
        }
    }

    pub fn try_increment_runs(&self) -> Option<u32> {
        // If using timer, always increment
        if self.timer.is_enabled() {
            return Some(self.total_runs.fetch_add(1, Ordering::Relaxed) + 1);
        }

        // Simple atomic increment with check
        let current = self.total_runs.fetch_add(1, Ordering::Relaxed);

        if current < self.max_runs {
            Some(current + 1)
        } else {
            // We went over the limit, decrement back
            self.total_runs.fetch_sub(1, Ordering::Relaxed);
            None
        }
    }

    pub fn increment_runs(&self) -> u32 {
        self.total_runs.fetch_add(1, Ordering::Relaxed)
    }

    pub fn increment_rejects(&self) -> u32 {
        self.total_rejects.fetch_add(1, Ordering::Relaxed)
    }

    pub fn should_continue(&self) -> bool {
        // Check fail-fast
        if self.fail_fast.should_stop() {
            return false;
        }

        if self.timer.is_enabled() {
            // Check timer
            !self.timer.is_timed_out()
        } else {
            // Check runs
            let total_runs = self.total_runs.load(Ordering::Relaxed);
            total_runs < self.max_runs
        }
    }

    /// Returns true if the worker was able to claim the failure, false if failure was set by
    /// another worker
    pub fn try_claim_failure(&self, worker_id: u32) -> bool {
        if self.found_failure.get().is_some() {
            return false;
        }

        let claimed = self.found_failure.set(worker_id).is_ok();
        if claimed {
            // Record failure in FailFast as well
            self.fail_fast.record_fail();
        }

        claimed
    }

    pub fn total_runs(&self) -> u32 {
        self.total_runs.load(Ordering::Relaxed)
    }

    pub fn total_rejects(&self) -> u32 {
        self.total_rejects.load(Ordering::Relaxed)
    }

    pub fn failed_worked_id(&self) -> Option<u32> {
        self.found_failure.get().copied()
    }
}

#[derive(Default)]
pub struct FuzzWorker {
    /// Worker identifier
    pub worker_id: u32,
    /// First fuzz case this worker encountered (with global run number)
    pub first_case: Option<(u32, FuzzCase)>,
    /// Gas usage for all cases this worker ran
    pub gas_by_case: Vec<(u64, u64)>,
    /// Counterexample if this worker found one
    pub counterexample: (Bytes, RawCallResult),
    /// Traces collected by this worker
    ///
    /// Stores upto `max_traces_to_collect` which is `config.gas_report_samples / num_workers`
    pub traces: Vec<SparsedTraceArena>,
    /// Last breakpoints from this worker
    pub breakpoints: Option<Breakpoints>,
    /// Coverage collected by this worker
    pub coverage: Option<HitMaps>,
    /// Logs from all cases this worker ran
    pub logs: Vec<Log>,
    /// Deprecated cheatcodes seen by this worker
    pub deprecated_cheatcodes: HashMap<&'static str, Option<&'static str>>,
    /// Number of runs this worker completed
    pub runs: u32,
    /// Number of rejects this worker encountered
    pub rejects: u32,
    /// Failure reason if this worker failed
    pub failure: Option<TestCaseError>,
    /// Last run timestamp in milliseconds
    ///
    /// Used to identify which worker ran last and collect its traces and call breakpoints
    pub last_run_timestamp: u128,
    /// Failed corpus replays
    pub failed_corpus_replays: usize,
}

impl FuzzWorker {
    pub fn new(worker_id: u32) -> Self {
        Self { worker_id, ..Default::default() }
    }
}
