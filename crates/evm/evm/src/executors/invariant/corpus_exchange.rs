use crate::executors::corpus::SharedCorpusEntry;
use foundry_config::{InvariantCorpusSyncConfig, InvariantCorpusSyncMode};
use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

struct ExchangeEntry {
    source_worker: u32,
    entry: SharedCorpusEntry,
}

/// Campaign-local exchange for immutable corpus candidates discovered by invariant workers.
///
/// The exchange does not own executor state and never decides coverage usefulness. Workers publish
/// snapshots here and sibling workers replay imported candidates against their own local coverage.
pub(super) struct InvariantCorpusExchange {
    /// Append-only campaign log. Entry index + 1 is the exchange epoch.
    entries: Mutex<Vec<ExchangeEntry>>,
}

impl InvariantCorpusExchange {
    pub(super) const fn new() -> Self {
        Self { entries: Mutex::new(Vec::new()) }
    }

    pub(super) fn publish(&self, worker_id: u32, entries: Vec<SharedCorpusEntry>) {
        if entries.is_empty() {
            return;
        }

        let mut exchange_entries = self.entries.lock().expect("invariant corpus exchange poisoned");
        exchange_entries.extend(
            entries.into_iter().map(|entry| ExchangeEntry { source_worker: worker_id, entry }),
        );
    }

    pub(super) fn import_since(
        &self,
        worker_id: u32,
        last_seen_epoch: u64,
        limit: usize,
    ) -> (Vec<SharedCorpusEntry>, u64) {
        if limit == 0 {
            return (Vec::new(), last_seen_epoch);
        }

        let exchange_entries = self.entries.lock().expect("invariant corpus exchange poisoned");
        let mut entries = Vec::new();
        let mut newest_epoch = last_seen_epoch;
        for (idx, entry) in exchange_entries.iter().enumerate().skip(last_seen_epoch as usize) {
            newest_epoch = idx as u64 + 1;
            if entry.source_worker == worker_id {
                continue;
            }

            entries.push(entry.entry.clone());
            if entries.len() == limit {
                break;
            }
        }
        (entries, newest_epoch)
    }
}

pub(super) struct InvariantCorpusSyncState {
    runs_since_new_coverage: u32,
    last_new_coverage_at: Instant,
    last_seen_epoch: u64,
}

impl InvariantCorpusSyncState {
    pub(super) const fn new(now: Instant) -> Self {
        Self { runs_since_new_coverage: 0, last_new_coverage_at: now, last_seen_epoch: 0 }
    }

    pub(super) const fn last_seen_epoch(&self) -> u64 {
        self.last_seen_epoch
    }

    pub(super) const fn set_last_seen_epoch(&mut self, epoch: u64) {
        self.last_seen_epoch = epoch;
    }

    pub(super) const fn record_completed_run(&mut self, new_coverage: bool, now: Instant) {
        if new_coverage {
            self.runs_since_new_coverage = 0;
            self.last_new_coverage_at = now;
        } else {
            self.runs_since_new_coverage = self.runs_since_new_coverage.saturating_add(1);
        }
    }

    pub(super) fn should_sync(&self, config: &InvariantCorpusSyncConfig, now: Instant) -> bool {
        if !matches!(config.mode, InvariantCorpusSyncMode::Plateau) {
            return false;
        }

        self.runs_since_new_coverage >= config.plateau_runs
            || config.plateau_seconds.is_some_and(|seconds| {
                now.duration_since(self.last_new_coverage_at) >= Duration::from_secs(seconds.into())
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executors::corpus::test_support::shared_corpus_entry;
    use foundry_evm_fuzz::{BasicTxDetails, CallDetails};

    fn entry(sender: u8) -> SharedCorpusEntry {
        shared_corpus_entry(vec![BasicTxDetails {
            warp: None,
            roll: None,
            sender: alloy_primitives::Address::repeat_byte(sender),
            call_details: CallDetails {
                target: alloy_primitives::Address::repeat_byte(sender),
                calldata: alloy_primitives::Bytes::from(vec![sender]),
                value: None,
            },
        }])
    }

    #[test]
    fn exchange_imports_from_other_workers_in_epoch_order() {
        let exchange = InvariantCorpusExchange::new();
        exchange.publish(1, vec![entry(1)]);
        exchange.publish(2, vec![entry(2)]);

        let (first, epoch) = exchange.import_since(0, 0, 1);
        assert_eq!(first.len(), 1);
        assert_eq!(epoch, 1);

        let (second, epoch) = exchange.import_since(0, epoch, 8);
        assert_eq!(second.len(), 1);
        assert_eq!(epoch, 2);
    }

    #[test]
    fn exchange_does_not_import_own_entries() {
        let exchange = InvariantCorpusExchange::new();
        exchange.publish(0, vec![entry(1)]);
        let (entries, epoch) = exchange.import_since(0, 0, 8);
        assert!(entries.is_empty());
        assert_eq!(epoch, 1);
    }

    #[test]
    fn exchange_advances_past_own_entries() {
        let exchange = InvariantCorpusExchange::new();
        exchange.publish(0, vec![entry(1), entry(2)]);

        let (entries, epoch) = exchange.import_since(0, 0, 8);
        assert!(entries.is_empty());
        assert_eq!(epoch, 2);

        exchange.publish(1, vec![entry(3)]);
        let (entries, epoch) = exchange.import_since(0, epoch, 8);
        assert_eq!(entries.len(), 1);
        assert_eq!(epoch, 3);
    }

    #[test]
    fn plateau_sync_triggers_after_runs_without_coverage() {
        let mut state = InvariantCorpusSyncState::new(Instant::now());
        let config = InvariantCorpusSyncConfig {
            mode: InvariantCorpusSyncMode::Plateau,
            plateau_runs: 2,
            plateau_seconds: None,
            max_imports_per_sync: 8,
        };

        state.record_completed_run(false, Instant::now());
        assert!(!state.should_sync(&config, Instant::now()));
        state.record_completed_run(false, Instant::now());
        assert!(state.should_sync(&config, Instant::now()));
        state.record_completed_run(true, Instant::now());
        assert!(!state.should_sync(&config, Instant::now()));
    }
}
