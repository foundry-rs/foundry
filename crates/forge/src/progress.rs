use indicatif::{MultiProgress, ProgressBar};
use parking_lot::Mutex;
use std::{collections::HashMap, sync::Arc, time::Duration};

/// State of [ProgressBar]s displayed for the given test run.
/// Shows progress of all test suites matching filter.
/// For each test within the test suite an individual progress bar is displayed.
/// When a test suite completes, their progress is removed from overall progress and result summary
/// is displayed.
#[derive(Debug)]
pub struct TestsProgressState {
    /// Main [MultiProgress] instance showing progress for all test suites.
    multi: MultiProgress,
    /// Progress bar counting completed / remaining test suites.
    overall_progress: ProgressBar,
    /// Individual test suites progress.
    suites_progress: HashMap<String, ProgressBar>,
}

impl TestsProgressState {
    // Creates overall tests progress state.
    pub fn new(suites_len: usize, threads_no: usize) -> Self {
        let multi = MultiProgress::new();
        let overall_progress = multi.add(ProgressBar::new(suites_len as u64));
        overall_progress.set_style(
            indicatif::ProgressStyle::with_template("{bar:40.cyan/blue} {pos:>7}/{len:7} {msg}")
                .unwrap()
                .progress_chars("##-"),
        );
        overall_progress.set_message(format!("completed (with {} threads)", threads_no as u64));
        Self { multi, overall_progress, suites_progress: HashMap::default() }
    }

    /// Creates new test suite progress and add it to overall progress.
    pub fn start_suite_progress(&mut self, suite_name: &String) {
        let suite_progress = self.multi.add(ProgressBar::new_spinner());
        suite_progress.set_style(
            indicatif::ProgressStyle::with_template("{spinner} {wide_msg:.bold.dim}")
                .unwrap()
                .tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈ "),
        );
        suite_progress.set_message(format!("{suite_name} "));
        suite_progress.enable_steady_tick(Duration::from_millis(100));
        self.suites_progress.insert(suite_name.to_owned(), suite_progress);
    }

    /// Prints suite result summary and removes it from overall progress.
    pub fn end_suite_progress(&mut self, suite_name: &String, result_summary: String) {
        if let Some(suite_progress) = self.suites_progress.remove(suite_name) {
            self.multi.suspend(|| {
                println!("{suite_name}\n  ↪ {result_summary}");
            });
            suite_progress.finish_and_clear();
            // Increment test progress bar to reflect completed test suite.
            self.overall_progress.inc(1);
        }
    }

    /// Creates progress entry for fuzz tests.
    /// Set the prefix and total number of runs. Message is updated during execution with current
    /// phase. Test progress is placed under test suite progress entry so all tests within suite
    /// are grouped.
    pub fn start_fuzz_progress(
        &mut self,
        suite_name: &str,
        test_name: &String,
        runs: u32,
    ) -> Option<ProgressBar> {
        if let Some(suite_progress) = self.suites_progress.get(suite_name) {
            let fuzz_progress =
                self.multi.insert_after(suite_progress, ProgressBar::new(runs as u64));
            fuzz_progress.set_style(
                indicatif::ProgressStyle::with_template(
                    "    ↪ {prefix:.bold.dim}: [{pos}/{len}]{msg} Runs",
                )
                .unwrap()
                .tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈ "),
            );
            fuzz_progress.set_prefix(test_name.to_string());
            Some(fuzz_progress)
        } else {
            None
        }
    }

    /// Removes overall test progress.
    pub fn clear(&mut self) {
        self.multi.clear().unwrap();
    }
}

/// Clonable wrapper around [TestsProgressState].
#[derive(Debug, Clone)]
pub struct TestsProgress {
    pub inner: Arc<Mutex<TestsProgressState>>,
}

impl TestsProgress {
    pub fn new(suites_len: usize, threads_no: usize) -> Self {
        Self { inner: Arc::new(Mutex::new(TestsProgressState::new(suites_len, threads_no))) }
    }
}

/// Helper function for creating fuzz test progress bar.
pub fn start_fuzz_progress(
    tests_progress: Option<&TestsProgress>,
    suite_name: &str,
    test_name: &String,
    runs: u32,
) -> Option<ProgressBar> {
    if let Some(progress) = tests_progress {
        progress.inner.lock().start_fuzz_progress(suite_name, test_name, runs)
    } else {
        None
    }
}
