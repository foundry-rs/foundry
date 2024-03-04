use foundry_common::term::Spinner;
use parking_lot::Mutex;
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

static GLOBAL_INVARIANT_REPORTER_STATE: AtomicUsize = AtomicUsize::new(UN_SET);

const UN_SET: usize = 0;
const SETTING: usize = 1;
const SET: usize = 2;

static mut GLOBAL_INVARIANT_REPORTER: Option<InvariantRunsReporter> = None;

/// Stores information about overall invariant tests progress (completed runs / total runs).
#[derive(Default)]
struct InvariantProgress {
    /// Total number of runs (for all invariant tests).
    total_runs: u32,
    /// Number of completed runs (for all invariant tests).
    completed_runs: u32,
}

/// Reporter of the invariant test progress, set as a global reporter.
/// The number of invariant runs are incremented prior of each test execution.
/// Completed runs are incremented on each test execution.
/// Status is displayed in terminal as a spinner message on a thread that polls progress every
/// 100ms.
#[derive(Clone)]
pub struct InvariantRunsReporter {
    inner: Arc<Mutex<InvariantProgress>>,
}

impl InvariantRunsReporter {
    pub fn add_runs(&self, runs: u32) {
        self.inner.lock().total_runs += runs;
    }

    pub fn complete_run(&self) {
        self.inner.lock().completed_runs += 1;
    }
}

impl Default for InvariantRunsReporter {
    fn default() -> Self {
        let inner_reporter = Arc::new(Mutex::new(InvariantProgress::default()));

        let inner_reporter_clone = inner_reporter.clone();
        // Spawn thread to periodically poll invariant progress and to display status in console.
        thread::spawn(move || {
            let mut spinner = Spinner::new("");
            loop {
                thread::sleep(Duration::from_millis(100));
                spinner.tick();
                let progress = &inner_reporter_clone.lock();
                if progress.total_runs != 0 && progress.completed_runs != 0 {
                    spinner.message(format!(
                        "Invariant runs {:.0}% ({}/{})",
                        (100.0 * (progress.completed_runs as f32 / progress.total_runs as f32))
                            .floor(),
                        progress.completed_runs,
                        progress.total_runs
                    ));
                }
            }
        });

        Self { inner: inner_reporter }
    }
}

/// Create invariant reporter and set it as a global reporter.
pub fn init(show_progress: bool) {
    if show_progress {
        set_global_reporter(InvariantRunsReporter::default());
    }
}

/// Add test runs to the total runs counter.
pub fn add_runs(runs: u32) {
    if let Some(reporter) = get_global_reporter() {
        reporter.add_runs(runs);
    }
}

/// Increment invariant completed runs counter.
pub fn complete_run() {
    if let Some(reporter) = get_global_reporter() {
        reporter.complete_run();
    }
}

fn set_global_reporter(reporter: InvariantRunsReporter) {
    if GLOBAL_INVARIANT_REPORTER_STATE
        .compare_exchange(UN_SET, SETTING, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
    {
        unsafe {
            GLOBAL_INVARIANT_REPORTER = Some(reporter);
        }
        GLOBAL_INVARIANT_REPORTER_STATE.store(SET, Ordering::SeqCst);
    }
}

fn get_global_reporter() -> Option<&'static InvariantRunsReporter> {
    if GLOBAL_INVARIANT_REPORTER_STATE.load(Ordering::SeqCst) != SET {
        return None;
    }
    unsafe {
        // This is safe given the invariant that setting the global reporter
        // also sets `GLOBAL_INVARIANT_REPORTER_STATE` to `SET`.
        Some(GLOBAL_INVARIANT_REPORTER.as_ref().expect(
            "Reporter invariant violated: GLOBAL_INVARIANT_REPORTER must be initialized before GLOBAL_INVARIANT_REPORTER_STATE is set",
        ))
    }
}
