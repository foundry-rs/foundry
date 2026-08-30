//! Progress display for mutation testing.

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use parking_lot::Mutex;
use yansi::Paint;

use crate::mutation::mutant::{Mutant, MutationResult};

/// Live tally of mutant outcomes, rendered into the overall progress bar.
#[derive(Debug, Default, Clone, Copy)]
struct LiveCounts {
    killed: usize,
    survived: usize,
    invalid: usize,
    skipped: usize,
    timed_out: usize,
}

impl LiveCounts {
    const fn record(&mut self, result: &MutationResult) {
        match result {
            MutationResult::Dead => self.killed += 1,
            MutationResult::Alive => self.survived += 1,
            MutationResult::Invalid => self.invalid += 1,
            MutationResult::Skipped => self.skipped += 1,
            MutationResult::TimedOut => self.timed_out += 1,
        }
    }
}

/// State stored per active mutant so we can show per-mutant elapsed time and
/// remove the correct row when a mutant completes (rather than FIFO which
/// breaks under parallel completion).
#[derive(Debug)]
struct ActiveMutant {
    pb: ProgressBar,
    started_at: Instant,
}

/// State for mutation testing progress display.
#[derive(Debug)]
pub struct MutationProgressState {
    multi: MultiProgress,
    overall_progress: ProgressBar,
    /// Active mutant progress bars keyed by a stable identifier (path + span +
    /// mutation string) so completion correctly removes the right row.
    active_mutants: HashMap<String, ActiveMutant>,
    /// Running per-result counts displayed on the overall bar.
    counts: LiveCounts,
    /// Optional per-mutant timeout (seconds), shown next to each active row.
    timeout_secs: Option<u32>,
    /// Number of parallel workers, used in the prefix.
    num_workers: usize,
}

impl MutationProgressState {
    pub fn new(total_mutants: usize, num_workers: usize) -> Self {
        Self::with_timeout(total_mutants, num_workers, None)
    }

    pub fn with_timeout(
        total_mutants: usize,
        num_workers: usize,
        timeout_secs: Option<u32>,
    ) -> Self {
        let multi = MultiProgress::new();

        // Overall progress bar: includes elapsed wall-clock plus a running
        // tally (killed / survived / invalid / timed-out / skipped).
        let overall_progress = multi.add(ProgressBar::new(total_mutants as u64));
        overall_progress.set_style(
            ProgressStyle::with_template(
                "{bar:40.cyan/blue} {pos:>4}/{len:4} mutants ({prefix} jobs) [{elapsed_precise}] {wide_msg}",
            )
            .unwrap()
            .progress_chars("##-"),
        );
        overall_progress.set_prefix(num_workers.to_string());
        overall_progress.enable_steady_tick(Duration::from_millis(100));

        Self {
            multi,
            overall_progress,
            active_mutants: HashMap::with_capacity(num_workers),
            counts: LiveCounts::default(),
            timeout_secs,
            num_workers,
        }
    }

    /// Set the current file being tested. Renders as part of the overall
    /// bar's message together with the running tally.
    pub fn set_current_file(&mut self, file: &str) {
        // Re-emit message so the file is reflected immediately.
        let msg = self.format_message(file);
        self.overall_progress.set_message(msg);
    }

    fn format_message(&self, file: &str) -> String {
        let counts = self.counts;
        let timeout_suffix = match self.timeout_secs {
            Some(t) => format!(" · timeout {t}s/mutant"),
            None => String::new(),
        };
        format!(
            "k:{} s:{} i:{} t:{} sk:{}{} · {}",
            counts.killed,
            counts.survived,
            counts.invalid,
            counts.timed_out,
            counts.skipped,
            timeout_suffix,
            file,
        )
    }

    fn refresh_message(&self) {
        // Preserve the file segment from the last-rendered message if any.
        let current = self.overall_progress.message();
        let file = current.rsplit(" · ").next().unwrap_or("");
        self.overall_progress.set_message(self.format_message(file));
    }

    /// Stable identifier for a mutant — used as the key in `active_mutants`.
    fn mutant_key(mutant: &Mutant) -> String {
        format!(
            "{}:{}-{}:{}",
            mutant.path.display(),
            mutant.span.lo().0,
            mutant.span.hi().0,
            mutant.mutation,
        )
    }

    /// Add a mutant being tested
    pub fn add_mutant_progress(&mut self, mutant: &Mutant) {
        let pb = self.multi.add(ProgressBar::new_spinner());
        pb.set_style(
            ProgressStyle::with_template("  {spinner} {wide_msg}").unwrap().tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈ "),
        );
        pb.enable_steady_tick(Duration::from_millis(100));

        let display = format!(
            "line {}: `{}` → `{}`",
            mutant.line_number,
            truncate_str(&mutant.original, 40),
            truncate_str(&mutant.mutation.to_string(), 40),
        );
        pb.set_message(display);

        self.active_mutants
            .insert(Self::mutant_key(mutant), ActiveMutant { pb, started_at: Instant::now() });
    }

    /// Complete a mutant and show result. Prints a one-line summary above the
    /// bars (via `multi.suspend`) before clearing that mutant's spinner.
    pub fn complete_mutant(&mut self, mutant: &Mutant, result: &MutationResult) {
        self.counts.record(result);
        self.overall_progress.inc(1);

        let elapsed = self
            .active_mutants
            .remove(&Self::mutant_key(mutant))
            .map(|am| {
                let el = am.started_at.elapsed();
                am.pb.finish_and_clear();
                el
            })
            .unwrap_or_default();

        // Only emit per-result completion lines for things the user cares
        // about (kills, survivors, timeouts). Invalid and skipped are noisy.
        // Pad the raw label *before* applying color so ANSI escapes don't
        // throw off alignment.
        let raw_label = format!("{:9}", result.label());
        let label = match result {
            MutationResult::Dead => Paint::green(&raw_label).bold().to_string(),
            MutationResult::Alive => Paint::red(&raw_label).bold().to_string(),
            MutationResult::TimedOut => Paint::yellow(&raw_label).bold().to_string(),
            MutationResult::Invalid | MutationResult::Skipped => {
                self.refresh_message();
                return;
            }
        };

        let line = format!(
            "  {label} line {ln}: `{orig}` → `{mut_}` ({elapsed:.1?})",
            ln = mutant.line_number,
            orig = truncate_str(&mutant.original, 40),
            mut_ = truncate_str(&mutant.mutation.to_string(), 40),
            elapsed = elapsed,
        );
        self.multi.suspend(|| {
            let _ = foundry_common::sh_println!("{line}");
        });
        self.refresh_message();
    }

    /// Clear all progress bars
    pub fn clear(&mut self) {
        for (_, am) in self.active_mutants.drain() {
            am.pb.finish_and_clear();
        }
        self.overall_progress.finish_and_clear();
        let _ = self.multi.clear();
    }

    /// Finish with a message
    pub fn finish(&mut self, message: &str) {
        for (_, am) in self.active_mutants.drain() {
            am.pb.finish_and_clear();
        }
        self.overall_progress.finish_with_message(message.to_string());
    }

    /// Used for tests / introspection.
    #[allow(dead_code)]
    pub const fn num_workers(&self) -> usize {
        self.num_workers
    }
}

/// Thread-safe wrapper for mutation progress
#[derive(Debug, Clone)]
pub struct MutationProgress {
    pub inner: Arc<Mutex<MutationProgressState>>,
    pub cancelled: Arc<AtomicBool>,
    pub completed: Arc<AtomicUsize>,
    pub total: usize,
}

impl MutationProgress {
    pub fn new(total_mutants: usize, num_workers: usize) -> Self {
        Self::with_timeout(total_mutants, num_workers, None)
    }

    pub fn with_timeout(
        total_mutants: usize,
        num_workers: usize,
        timeout_secs: Option<u32>,
    ) -> Self {
        Self {
            inner: Arc::new(Mutex::new(MutationProgressState::with_timeout(
                total_mutants,
                num_workers,
                timeout_secs,
            ))),
            cancelled: Arc::new(AtomicBool::new(false)),
            completed: Arc::new(AtomicUsize::new(0)),
            total: total_mutants,
        }
    }

    /// Check if testing was cancelled (Ctrl+C)
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// Signal cancellation
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// Set the current file
    pub fn set_current_file(&self, file: &str) {
        self.inner.lock().set_current_file(file);
    }

    /// Record a mutant starting
    pub fn start_mutant(&self, mutant: &Mutant) {
        self.inner.lock().add_mutant_progress(mutant);
    }

    /// Record a mutant completing
    pub fn complete_mutant(&self, mutant: &Mutant, result: &MutationResult) -> usize {
        let completed = self.completed.fetch_add(1, Ordering::SeqCst) + 1;
        self.inner.lock().complete_mutant(mutant, result);
        completed
    }

    /// Clear progress display
    pub fn clear(&self) {
        MutationProgressState::clear(&mut self.inner.lock());
    }

    /// Finish with message
    pub fn finish(&self, message: &str) {
        self.inner.lock().finish(message);
    }
}

/// Truncate a string to max length, centering around the middle (where the operator typically is)
fn truncate_str(s: &str, max_len: usize) -> String {
    let s = s.trim();
    if s.len() <= max_len {
        return s.to_string();
    }

    // Center the truncation around the middle of the string
    let half = max_len.saturating_sub(3) / 2; // Leave room for "..."
    let mid = s.len() / 2;
    let start = mid.saturating_sub(half);
    let end = (start + max_len.saturating_sub(3)).min(s.len());

    if start == 0 {
        format!("{}...", &s[..end])
    } else if end == s.len() {
        format!("...{}", &s[start..])
    } else {
        format!("...{}...", &s[start..end])
    }
}
