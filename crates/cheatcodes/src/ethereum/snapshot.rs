//! Native state snapshots and rollback of restorations made inside live frames.

use super::{
    ApplyError, Cheatcodes,
    forks::{Fork, Forks},
};
use alloy_primitives::U256;
use evm2::{
    Evm, Version,
    env::BlockEnvExt,
    evm::{EmptyDB, JournalEntry, StateCheckpoint, StateSnapshot},
    interpreter::{Interpreter, Message},
};
use foundry_evm_core::{
    constants::{CHEATCODE_ADDRESS, GLOBAL_FAIL_SLOT},
    ethereum::{FoundryContext, FoundryEvmTypes, SavedState},
};
use std::{collections::BTreeMap, mem, sync::Arc};

#[derive(Clone, Debug)]
pub(super) struct Snapshot {
    state: Arc<StateSnapshot>,
    transaction_epoch: u64,
    normalized: bool,
    version: Version,
    block: BlockEnvExt,
    context: FoundryContext,
    fork: Option<(U256, Fork)>,
}

#[derive(Clone, Debug)]
pub(super) struct SnapshotFrame {
    checkpoint: StateCheckpoint,
    fork: Option<(U256, u64)>,
    source: Option<Arc<StateSnapshot>>,
    entered: BTreeMap<Option<(U256, u64)>, Arc<StateSnapshot>>,
    pub(super) replaced: Option<Arc<StateSnapshot>>,
}

impl SnapshotFrame {
    pub(super) fn capture_source(
        frames: &mut [Self],
        host: &mut Evm<'_, FoundryEvmTypes>,
        fork: Option<(U256, u64)>,
    ) {
        if frames.iter().any(|frame| frame.fork == fork && frame.source.is_none()) {
            let captured = capture_transaction(host);
            for frame in frames {
                if frame.fork == fork {
                    frame.source.get_or_insert_with(|| Arc::clone(&captured));
                }
            }
        }
    }

    pub(super) fn capture_entry(
        frames: &mut [Self],
        host: &mut Evm<'_, FoundryEvmTypes>,
        forks: &Forks,
    ) {
        let fork = forks.identity();
        if frames.iter().any(|frame| !frame.entered.contains_key(&fork)) {
            let captured = capture_transaction(host);
            for frame in frames {
                if !frame.entered.contains_key(&fork) {
                    let entry = if frame.fork.is_none()
                        && let Some(source) = &frame.source
                    {
                        // Before the first selection, writes still belong to the unforked
                        // frame. Import its rolled-back persistent accounts into the selected
                        // fork rather than making those writes durable at the first switch.
                        let mut source = source.as_ref().clone().into_state(EmptyDB::default());
                        source.rollback(frame.checkpoint.clone(), host.version().features);
                        let mut entry = captured.as_ref().clone().into_state(EmptyDB::default());
                        forks.merge_persistent(&source, &mut entry);
                        Arc::new(entry.snapshot())
                    } else {
                        Arc::clone(&captured)
                    };
                    frame.entered.insert(fork, entry);
                }
            }
        }
    }
}

/// Captures rollback state without copying or retaining the accepted cache.
fn capture_transaction(host: &mut Evm<'_, FoundryEvmTypes>) -> Arc<StateSnapshot> {
    let state = host.state_mut();
    let cache = mem::take(&mut state.overlay_db_mut().cache);
    let captured = Arc::new(state.snapshot());
    state.overlay_db_mut().cache = cache;
    captured
}

impl Cheatcodes {
    pub(super) fn snapshot(&mut self, host: &mut Evm<'_, FoundryEvmTypes>) -> U256 {
        SavedState::materialize_initial(host.state_mut());
        self.snapshots.insert(Snapshot {
            state: Arc::new(host.state().snapshot()),
            transaction_epoch: self.transaction_epoch,
            normalized: false,
            version: *host.version(),
            block: *host.block(),
            context: *host.ext(),
            fork: self.forks.snapshot(),
        })
    }

    pub(super) fn restore_snapshot(
        &mut self,
        host: &mut Evm<'_, FoundryEvmTypes>,
        id: U256,
        delete: bool,
    ) -> Result<bool, ApplyError> {
        let snapshot =
            if delete { self.snapshots.remove_at(id) } else { self.snapshots.get(id).cloned() };
        let Some(mut snapshot) = snapshot else { return Ok(false) };
        if snapshot.transaction_epoch != self.transaction_epoch && !snapshot.normalized {
            // Project historical pending state only when used in a later transaction. All
            // touched/selfdestructed accounts are loaded in the captured transaction layer.
            let mut state = snapshot.state.as_ref().clone().into_state(EmptyDB::default());
            state.finalize_transaction(&snapshot.version)?;
            state.commit_transaction();
            state.clear_transaction_state();
            snapshot.state = Arc::new(state.snapshot());
            snapshot.normalized = true;
            if !delete {
                self.snapshots.insert_at(snapshot.clone(), id);
            }
        }
        SavedState::materialize_initial(host.state_mut());
        let prewarm = host.state().prewarm_set().clone();
        self.isolated_snapshot_restored |= self.isolation_depth.is_some();
        self.snapshot_failure |= host
            .state()
            .get_storage(&CHEATCODE_ADDRESS, &GLOBAL_FAIL_SLOT)
            .is_some_and(|value| !value.is_zero());
        // Every active frame needs only the first live state replaced since it began.
        // Share that capture when several frames encounter their first restoration together.
        if self.snapshot_frames.iter().any(|frame| frame.replaced.is_none()) {
            let replaced = Arc::new(host.state().snapshot());
            for frame in &mut self.snapshot_frames {
                frame.replaced.get_or_insert_with(|| Arc::clone(&replaced));
            }
        }
        let was_forked = self.forks.active.is_some();
        let fork_database = self.forks.restore(snapshot.fork);
        let spec = host.spec_id();
        let state = host.state_mut();
        let logs = mem::take(state.logs_mut());
        let mut database =
            mem::replace(&mut state.overlay_db_mut().db, Box::new(EmptyDB::default()));
        if let Some(source) = fork_database {
            database = Box::new(source.with_spec(spec));
        } else if was_forked {
            database = Box::new(EmptyDB::default());
        }
        *state = snapshot.state.as_ref().clone().into_state(database);
        state.set_prewarm_set(prewarm);
        *state.logs_mut() = logs;
        host.set_block(snapshot.block);
        // Origin belongs to the active transaction/prank, rather than the saved environment.
        let context = host.ext_mut();
        context.basefee_override = snapshot.context.basefee_override;
        context.gas_price_override = snapshot.context.gas_price_override;
        context.chain_id_override = snapshot.context.chain_id_override;
        Ok(true)
    }

    pub(super) fn start_snapshot_frame(
        &mut self,
        host: &Evm<'_, FoundryEvmTypes>,
        message: &Message<FoundryEvmTypes>,
    ) {
        if message.depth == 0 {
            self.snapshot_frames.clear();
        }
        self.snapshot_frames.push(SnapshotFrame {
            checkpoint: host.state().checkpoint(),
            fork: self.forks.identity(),
            source: None,
            entered: BTreeMap::new(),
            replaced: None,
        });
    }

    pub(super) fn initialize_snapshot_frame(
        &mut self,
        interp: &mut Interpreter<'_, '_, FoundryEvmTypes>,
    ) {
        if !interp.message().kind.is_create() || interp.message().depth == 0 {
            return;
        }
        let caller = interp.message().caller;
        let Some(frame) = self.snapshot_frames.last_mut() else { return };
        let journal = interp.host().state().journal();
        // CREATE's creator nonce is incremented before its rollback checkpoint. Preserve that
        // increment when a constructor restores a snapshot and subsequently reverts.
        if let Some(offset) = journal[frame.checkpoint.journal_len()..].iter().position(|entry| {
            matches!(entry, JournalEntry::AccountChange { address, .. } if *address == caller)
        }) {
            frame.checkpoint = StateCheckpoint::new(
                frame.checkpoint.journal_len() + offset + 1,
                frame.checkpoint.logs_len(),
            );
        }
    }

    pub(super) fn finish_snapshot_frame(
        &mut self,
        host: &mut Evm<'_, FoundryEvmTypes>,
        execution_failed: bool,
        validation_failed: bool,
    ) {
        let Some(frame) = self.snapshot_frames.pop() else { return };
        // State replacement must not resurrect receipt logs captured inside a failed frame.
        // Observed inspector logs follow their own policy and are deliberately retained.
        let receipt_logs = (execution_failed || validation_failed).then(|| {
            let mut logs = mem::take(host.state_mut().logs_mut());
            logs.truncate(frame.checkpoint.logs_len());
            logs
        });
        if execution_failed || validation_failed {
            let current = self.forks.identity();
            let captured = if current == frame.fork {
                frame.source.as_ref().map(|state| (state, true))
            } else {
                frame.entered.get(&current).map(|state| (state, false))
            };
            if let Some((captured, unwind)) = captured {
                let features = host.version().features;
                let state = host.state_mut();
                let cache = mem::take(&mut state.overlay_db_mut().cache);
                let database =
                    mem::replace(&mut state.overlay_db_mut().db, Box::new(EmptyDB::default()));
                *state = captured.as_ref().clone().into_state(database);
                state.overlay_db_mut().cache = cache;
                if unwind {
                    state.rollback(frame.checkpoint, features);
                }
                *state.logs_mut() = receipt_logs.expect("failed frame receipt logs");
                return;
            }
        }
        if (execution_failed || validation_failed)
            && let Some(replaced) = frame.replaced
        {
            let features = host.version().features;
            let state = host.state_mut();
            let database =
                mem::replace(&mut state.overlay_db_mut().db, Box::new(EmptyDB::default()));
            *state = replaced.as_ref().clone().into_state(database);
            state.rollback(frame.checkpoint, features);
        } else if validation_failed {
            let features = host.version().features;
            host.state_mut().rollback(frame.checkpoint, features);
        }
        if let Some(logs) = receipt_logs {
            *host.state_mut().logs_mut() = logs;
        }
    }
}
