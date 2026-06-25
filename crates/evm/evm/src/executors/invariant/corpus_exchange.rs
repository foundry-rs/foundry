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
    ) -> (Vec<SharedCorpusEntry>, u64) {
        let exchange_entries = self.entries.lock().expect("invariant corpus exchange poisoned");
        let mut entries = Vec::new();
        let mut newest_epoch = last_seen_epoch;
        for (idx, entry) in exchange_entries.iter().enumerate().skip(last_seen_epoch as usize) {
            newest_epoch = idx as u64 + 1;
            if entry.source_worker == worker_id {
                continue;
            }

            entries.push(entry.entry.clone());
        }
        (entries, newest_epoch)
    }
}

pub(super) struct InvariantCorpusSyncState {
    last_new_coverage_at: Instant,
    last_seen_epoch: u64,
}

impl InvariantCorpusSyncState {
    pub(super) const fn new(now: Instant) -> Self {
        Self { last_new_coverage_at: now, last_seen_epoch: 0 }
    }

    pub(super) const fn last_seen_epoch(&self) -> u64 {
        self.last_seen_epoch
    }

    pub(super) const fn set_last_seen_epoch(&mut self, epoch: u64) {
        self.last_seen_epoch = epoch;
    }

    pub(super) const fn record_completed_run(&mut self, new_coverage: bool, now: Instant) {
        if new_coverage {
            self.last_new_coverage_at = now;
        }
    }

    pub(super) const fn record_import_progress(&mut self, now: Instant) {
        self.last_new_coverage_at = now;
    }

    pub(super) fn should_sync(&self, config: &InvariantCorpusSyncConfig, now: Instant) -> bool {
        if !matches!(config.mode, InvariantCorpusSyncMode::Plateau) {
            return false;
        }

        now.duration_since(self.last_new_coverage_at)
            >= Duration::from_secs(config.plateau_seconds.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use foundry_evm_fuzz::{BasicTxDetails, CallDetails};

    fn entry(sender: u8) -> SharedCorpusEntry {
        SharedCorpusEntry::new(
            vec![BasicTxDetails {
                warp: None,
                roll: None,
                sender: alloy_primitives::Address::repeat_byte(sender),
                call_details: CallDetails {
                    target: alloy_primitives::Address::repeat_byte(sender),
                    calldata: alloy_primitives::Bytes::from(vec![sender]),
                    value: None,
                },
            }],
            Vec::new(),
            true,
        )
    }

    #[test]
    fn exchange_imports_from_other_workers_in_epoch_order() {
        let exchange = InvariantCorpusExchange::new();
        exchange.publish(1, vec![entry(1)]);
        exchange.publish(2, vec![entry(2)]);

        let (entries, epoch) = exchange.import_since(0, 0);
        assert_eq!(entries.len(), 2);
        assert_eq!(epoch, 2);
    }

    #[test]
    fn exchange_does_not_import_own_entries() {
        let exchange = InvariantCorpusExchange::new();
        exchange.publish(0, vec![entry(1)]);
        let (entries, epoch) = exchange.import_since(0, 0);
        assert!(entries.is_empty());
        assert_eq!(epoch, 1);
    }

    #[test]
    fn exchange_advances_past_own_entries() {
        let exchange = InvariantCorpusExchange::new();
        exchange.publish(0, vec![entry(1), entry(2)]);

        let (entries, epoch) = exchange.import_since(0, 0);
        assert!(entries.is_empty());
        assert_eq!(epoch, 2);

        exchange.publish(1, vec![entry(3)]);
        let (entries, epoch) = exchange.import_since(0, epoch);
        assert_eq!(entries.len(), 1);
        assert_eq!(epoch, 3);
    }

    #[test]
    fn plateau_sync_triggers_after_time_without_coverage() {
        let now = Instant::now();
        let mut state = InvariantCorpusSyncState::new(now);
        let config = InvariantCorpusSyncConfig {
            mode: InvariantCorpusSyncMode::Plateau,
            plateau_seconds: 60,
            ..Default::default()
        };

        state.record_completed_run(false, now + Duration::from_secs(30));
        assert!(!state.should_sync(&config, now + Duration::from_secs(59)));
        assert!(state.should_sync(&config, now + Duration::from_secs(60)));
        state.record_completed_run(true, now + Duration::from_secs(60));
        assert!(!state.should_sync(&config, now + Duration::from_secs(119)));
    }

    #[test]
    fn accepted_import_resets_plateau_state() {
        let now = Instant::now();
        let mut state = InvariantCorpusSyncState::new(now);
        let config = InvariantCorpusSyncConfig {
            mode: InvariantCorpusSyncMode::Plateau,
            plateau_seconds: 60,
            ..Default::default()
        };

        assert!(state.should_sync(&config, now + Duration::from_secs(60)));

        state.record_import_progress(now + Duration::from_secs(60));
        assert!(!state.should_sync(&config, now + Duration::from_secs(119)));
    }
}
