//! Progress tracking for contract verification.

use indicatif::{MultiProgress, ProgressBar};
use parking_lot::Mutex;
use std::{sync::Arc, time::Duration};

/// Verification status for a single contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerificationStatus {
    /// Verification is pending (not started yet).
    Pending,
    /// Verification is in progress.
    InProgress,
    /// Verification completed successfully.
    Success,
    /// Verification failed.
    Failed,
}

/// Progress bars for a single contract verification (header + status line).
#[derive(Debug, Clone)]
pub struct ContractProgressBar {
    /// Header line showing contract address and name with spinner.
    pub header: ProgressBar,
    /// Status line showing current verification status.
    pub status: ProgressBar,
}

/// State of [ProgressBar]s displayed for contract verification.
/// Shows progress of all verification tasks with individual contract progress bars.
#[derive(Debug)]
pub struct VerificationProgressState {
    /// Main [MultiProgress] instance showing progress for all verification tasks.
    multi: MultiProgress,
    /// Progress bar counting completed / remaining verifications.
    overall_progress: ProgressBar,
    /// Individual contract verification progress bars (kept alive for MultiProgress rendering).
    contracts_progress: Vec<ContractProgressBar>,
}

impl VerificationProgressState {
    /// Creates overall verification progress state.
    pub fn new(total: usize) -> Self {
        let multi = MultiProgress::new();
        let overall_progress = multi.add(ProgressBar::new(total as u64));
        overall_progress.set_style(
            indicatif::ProgressStyle::with_template("{bar:40.cyan/blue} {pos:>7}/{len:7} {msg}")
                .unwrap()
                .progress_chars("##-"),
        );
        overall_progress.set_message("contracts verified");
        Self { multi, overall_progress, contracts_progress: Vec::new() }
    }

    /// Adds a contract progress bar (shown as pending). Returns the created [ContractProgressBar].
    pub fn add_contract(
        &mut self,
        contract_address: &str,
        contract_name: &str,
    ) -> ContractProgressBar {
        let header = self.multi.add(ProgressBar::new_spinner());
        header.set_style(indicatif::ProgressStyle::with_template("{wide_msg:.dim}").unwrap());
        header.set_message(format!("{contract_address} ({contract_name}): pending"));

        let status = self.multi.insert_after(&header, ProgressBar::new_spinner());
        status.set_style(indicatif::ProgressStyle::with_template("  ↪ {wide_msg:.dim}").unwrap());
        status.set_message("waiting...");

        let progress = ContractProgressBar { header, status };
        self.contracts_progress.push(progress.clone());
        progress
    }

    /// Starts verification for a contract (activates spinner).
    /// `details` contains verification context like chain, evm version, solc version, etc.
    pub fn start_verification(
        progress: &ContractProgressBar,
        contract_address: &str,
        contract_name: &str,
        details: &str,
    ) {
        progress.header.set_style(
            indicatif::ProgressStyle::with_template("{spinner} {wide_msg:.bold}")
                .unwrap()
                .tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈ "),
        );
        let header_msg = if details.is_empty() {
            format!("{contract_address} ({contract_name}): verifying...")
        } else {
            format!("{contract_address} ({contract_name}): verifying... [{details}]")
        };
        progress.header.set_message(header_msg);
        progress.header.enable_steady_tick(Duration::from_millis(100));

        progress
            .status
            .set_style(indicatif::ProgressStyle::with_template("  ↪ {wide_msg:.dim}").unwrap());
        progress.status.set_message("starting...");
    }

    /// Updates the status line message for a specific contract.
    pub fn update_contract(progress: &ContractProgressBar, message: &str) {
        progress.status.set_message(message.to_string());
    }

    /// Completes a verification task: updates progress bars with final status.
    pub fn end_verification(
        &self,
        progress: &ContractProgressBar,
        contract_address: &str,
        contract_name: &str,
        status: VerificationStatus,
    ) {
        let status_msg = match status {
            VerificationStatus::Success => "✓ verified",
            VerificationStatus::Failed => "✗ failed",
            VerificationStatus::InProgress => "... in progress",
            VerificationStatus::Pending => "○ pending",
        };

        progress.header.set_style(indicatif::ProgressStyle::with_template("{wide_msg}").unwrap());
        progress.header.set_message(format!("{contract_address} ({contract_name}): {status_msg}"));
        progress.header.finish();

        progress.status.finish_and_clear();

        if matches!(status, VerificationStatus::Success | VerificationStatus::Failed) {
            self.overall_progress.inc(1);
        }
    }

    /// Removes overall verification progress.
    pub fn clear(&mut self) {
        self.multi.clear().unwrap();
    }
}

/// Cloneable wrapper around [VerificationProgressState].
#[derive(Debug, Clone)]
pub struct VerificationProgress {
    pub inner: Arc<Mutex<VerificationProgressState>>,
}

impl VerificationProgress {
    /// Creates a new verification progress tracker.
    pub fn new(total: usize) -> Self {
        Self { inner: Arc::new(Mutex::new(VerificationProgressState::new(total))) }
    }

    /// Adds a contract to track. Returns the created [ContractProgressBar] for future updates.
    pub fn add_contract(&self, contract_address: &str, contract_name: &str) -> ContractProgressBar {
        self.inner.lock().add_contract(contract_address, contract_name)
    }

    /// Starts tracking a verification task.
    /// `details` contains verification context like chain, evm version, solc version, etc.
    pub fn start_verification(
        progress: &ContractProgressBar,
        contract_address: &str,
        contract_name: &str,
        details: &str,
    ) {
        VerificationProgressState::start_verification(
            progress,
            contract_address,
            contract_name,
            details,
        )
    }

    /// Updates the status line message for a specific contract.
    pub fn update_contract(progress: &ContractProgressBar, message: &str) {
        VerificationProgressState::update_contract(progress, message)
    }

    /// Completes a verification task.
    pub fn end_verification(
        &self,
        progress: &ContractProgressBar,
        contract_address: &str,
        contract_name: &str,
        status: VerificationStatus,
    ) {
        self.inner.lock().end_verification(progress, contract_address, contract_name, status)
    }

    /// Clears all progress bars.
    pub fn clear(&self) {
        self.inner.lock().clear()
    }
}
