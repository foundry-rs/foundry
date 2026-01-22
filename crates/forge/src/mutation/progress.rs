//! Progress display for mutation testing.

use crate::mutation::mutant::{Mutant, MutationResult};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use parking_lot::Mutex;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

/// State for mutation testing progress display.
#[derive(Debug)]
pub struct MutationProgressState {
    multi: MultiProgress,
    overall_progress: ProgressBar,
    /// Current file being tested
    current_file: String,
    /// Active mutant progress bars (up to N for parallel workers)
    active_mutants: Vec<ProgressBar>,
}

impl MutationProgressState {
    pub fn new(total_mutants: usize, num_workers: usize) -> Self {
        let multi = MultiProgress::new();

        // Overall progress bar - matches forge test style
        let overall_progress = multi.add(ProgressBar::new(total_mutants as u64));
        overall_progress.set_style(
            ProgressStyle::with_template(
                "{bar:40.cyan/blue} {pos:>4}/{len:4} runs ({prefix} jobs) {wide_msg}",
            )
            .unwrap()
            .progress_chars("##-"),
        );
        overall_progress.set_prefix(num_workers.to_string());

        Self {
            multi,
            overall_progress,
            current_file: String::new(),
            active_mutants: Vec::with_capacity(num_workers),
        }
    }

    /// Set the current file being tested
    pub fn set_current_file(&mut self, file: &str) {
        self.current_file = file.to_string();
        self.overall_progress.set_message(file.to_string());
    }

    /// Add a mutant being tested
    pub fn add_mutant_progress(&mut self, mutant: &Mutant) {
        let pb = self.multi.add(ProgressBar::new_spinner());
        pb.set_style(
            ProgressStyle::with_template("  {spinner} line {wide_msg}")
                .unwrap()
                .tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈ "),
        );
        pb.enable_steady_tick(std::time::Duration::from_millis(100));

        let msg = format!(
            "{}: `{}` → `{}`",
            mutant.line_number,
            truncate_str(&mutant.original, 40),
            truncate_str(&mutant.mutation.to_string(), 40),
        );
        pb.set_message(msg);
        self.active_mutants.push(pb);
    }

    /// Complete a mutant and show result
    pub fn complete_mutant(&mut self, _mutant: &Mutant, _result: &MutationResult) {
        self.overall_progress.inc(1);

        // Remove the oldest active mutant progress
        if !self.active_mutants.is_empty() {
            let pb = self.active_mutants.remove(0);
            pb.finish_and_clear();
        }
    }

    /// Clear all progress bars
    pub fn clear(&mut self) {
        for pb in self.active_mutants.drain(..) {
            pb.finish_and_clear();
        }
        self.overall_progress.finish_and_clear();
        let _ = self.multi.clear();
    }

    /// Finish with a message
    pub fn finish(&mut self, message: &str) {
        for pb in self.active_mutants.drain(..) {
            pb.finish_and_clear();
        }
        self.overall_progress.finish_with_message(message.to_string());
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
        Self {
            inner: Arc::new(Mutex::new(MutationProgressState::new(total_mutants, num_workers))),
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
        self.inner.lock().clear();
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
