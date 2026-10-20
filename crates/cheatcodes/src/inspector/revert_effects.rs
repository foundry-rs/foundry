//! Preserve unjournaled host writes without replacing ordinary EVM undo operations.

use alloy_primitives::{
    Address, B256, U256,
    map::{AddressHashMap, HashMap, HashSet},
};
use evm2::{
    EvmFeatures,
    bytecode::Bytecode,
    evm::{
        AccountInfo, Database, DbResult, EmptyDB, JournalEntry, State, StateCheckpoint,
        StateSnapshot,
    },
};
use foundry_evm_core::evm::mark_created;
use std::{mem, ops::Range};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Field {
    Balance(Address),
    Nonce(Address),
    Code(Address),
    Storage(Address, U256),
    Created(Address),
}

#[derive(Clone, Debug)]
pub(crate) enum SurvivingWrite {
    Balance(Address, U256),
    Nonce(Address, u64),
    Code(Address, B256, Option<Bytecode>),
    Storage(Address, U256, U256),
    Created(Address, bool),
}

impl SurvivingWrite {
    fn address(&self) -> Address {
        match self {
            Self::Balance(address, _)
            | Self::Nonce(address, _)
            | Self::Code(address, ..)
            | Self::Storage(address, ..)
            | Self::Created(address, _) => *address,
        }
    }

    fn field(&self) -> Field {
        match self {
            Self::Balance(address, _) => Field::Balance(*address),
            Self::Nonce(address, _) => Field::Nonce(*address),
            Self::Code(address, ..) => Field::Code(*address),
            Self::Storage(address, key, _) => Field::Storage(*address, *key),
            Self::Created(address, _) => Field::Created(*address),
        }
    }

    fn apply(&self, state: &mut State<'_>) -> DbResult<()> {
        match self {
            Self::Balance(address, balance) => state.account(address)?.set_balance(*balance),
            Self::Nonce(address, nonce) => state.account(address)?.set_nonce(*nonce),
            Self::Code(address, hash, code) => {
                let code = match code {
                    Some(code) => code.clone(),
                    None => state.overlay_db_mut().get_code_by_hash(hash)?,
                };
                state.account(address)?.set_code(*hash, code);
            }
            Self::Storage(address, key, value) => state.storage_slot(address, *key)?.set(*value),
            Self::Created(address, true) => {
                state.account(address)?.get_or_insert();
                mark_created(state, address)?;
            }
            // Native rollback already cleared creation when the enclosing frame owned it.
            Self::Created(_, false) => {}
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
struct JournalAnnotations {
    /// Entries from host writes and rollback repairs, not ordinary EVM operations.
    excluded: Vec<HostWrites>,
    /// Code setters must undo even when they wrote the same bytes.
    code: HashSet<usize>,
    /// Creation resets the destination nonce instead of decrementing it.
    created: HashSet<usize>,
}

#[derive(Clone, Debug)]
struct HostWrites {
    range: Range<usize>,
    /// Earliest before-image per account, so repeated host writes can be skipped in one step.
    before: AddressHashMap<(usize, AccountInfo)>,
}

#[derive(Clone, Debug)]
struct Frame {
    checkpoint: StateCheckpoint,
    features: EvmFeatures,
    writes: HashMap<Field, SurvivingWrite>,
    creating: bool,
    displaced: Option<Box<(StateSnapshot, JournalAnnotations)>>,
}

/// Sparse rollback corrections for one native journal. Nested EVMs use their own instance.
///
/// Only host-write boundaries scan the native journal. Ordinary execution needs neither a
/// second journal nor an opcode hook. Each frame remembers the host-affected fields after its
/// ordinary undo operations, rather than blindly restoring the last host-assigned values.
#[derive(Clone, Debug, Default)]
pub struct RevertEffects {
    frames: Vec<Frame>,
    journal: JournalAnnotations,
    /// Whether this execution restored a snapshot, including one undone by a frame revert.
    pub restored_snapshot: bool,
    /// Whether a snapshot's replacement journal survived the root frame.
    pub replaced_journal: bool,
    /// Fork replacements survive even when a later snapshot restoration is undone.
    pub replaced_fork: bool,
}

impl RevertEffects {
    /// Starts a frame before its value transfer or account creation.
    pub fn start_frame(&mut self, state: &State<'_>, features: EvmFeatures) {
        self.frames.push(Frame {
            checkpoint: state.checkpoint(),
            features,
            writes: HashMap::default(),
            creating: false,
            displaced: None,
        });
    }

    /// Records the creation boundary after native CREATE preparation, before initcode executes.
    pub(crate) fn created(&mut self, state: &State<'_>, address: Address, value: U256) {
        let Some(frame) = self.frames.last_mut() else { return };
        // Destination warming precedes the constructor checkpoint. Creation itself is the
        // destination's last account write before initializing the interpreter.
        if let Some(index) = state.journal().iter().enumerate().rev().find_map(|(index, entry)| {
            matches!(entry, JournalEntry::AccountChange { address: changed, .. } if *changed == address)
                .then_some(index)
        }) {
            self.journal.created.insert(index);
            // The caller's nonce bump survives constructor failure, but not its parent's revert.
            // The only intervening mutation is the endowment debit from the caller.
            frame.checkpoint = StateCheckpoint::new(
                index - usize::from(!value.is_zero()), frame.checkpoint.logs_len(),
            );
            frame.creating = true;
        }
    }

    /// Marks an ordinary code setter, including same-value writes.
    pub fn code_changed(&mut self, index: usize) {
        self.journal.code.insert(index);
    }

    /// Computes rollback values at each open frame boundary for a batch of host writes.
    pub(crate) fn record(
        &mut self,
        state: &mut State<'_>,
        start: usize,
        mut writes: Vec<SurvivingWrite>,
    ) -> DbResult<()> {
        self.exclude(state, start..state.journal().len());
        let mut accounts = AddressHashMap::default();
        let mut fields = AddressHashMap::<Vec<usize>>::default();
        let mut slots = HashMap::<_, _>::default();
        for (index, write) in writes.iter().enumerate() {
            let address = write.address();
            if let SurvivingWrite::Storage(_, key, _) = write {
                slots.insert((address, *key), index);
            } else {
                fields.entry(address).or_default().push(index);
            }
        }
        for address in fields.keys() {
            accounts.insert(*address, state.account_info_untracked(address)?.unwrap_or_default());
        }
        // A snapshot restoration displaces this state until its owning frame succeeds. Writes
        // in the restored state cannot escape a rollback that reinstates the displaced state.
        let barrier = self.frames.iter().rposition(|frame| frame.displaced.is_some());
        let mut frames = self
            .frames
            .iter_mut()
            .enumerate()
            .rev()
            .take_while(|(index, _)| barrier.is_none_or(|barrier| *index > barrier))
            .peekable();
        let mut end = state.journal().len();
        while end > 0 {
            let index = end - 1;
            let excluded = self.journal.excluded.partition_point(|span| span.range.end <= index);
            let excluded =
                self.journal.excluded.get(excluded).filter(|span| span.range.contains(&index));
            end = excluded.map_or(index, |span| span.range.start);
            while frames.peek().is_some_and(|(_, frame)| frame.checkpoint.journal_len() > end) {
                frames
                    .next()
                    .unwrap()
                    .1
                    .writes
                    .extend(writes.iter().map(|w| (w.field(), w.clone())));
            }
            if frames.peek().is_none() {
                break;
            }
            if let Some(span) = excluded {
                for (address, after) in &mut accounts {
                    if let Some((_, before)) = span.before.get(address) {
                        *after = before.clone();
                    }
                }
                continue;
            }
            match &state.journal()[index] {
                JournalEntry::AccountChange {
                    address, previous, previous_just_created, ..
                } => {
                    if let Some(after) = accounts.get_mut(address) {
                        let previous = previous.clone().unwrap_or_default();
                        for field in &fields[address] {
                            match &mut writes[*field] {
                                SurvivingWrite::Balance(_, balance) => {
                                    *balance = balance
                                        .wrapping_sub(after.balance)
                                        .wrapping_add(previous.balance);
                                }
                                SurvivingWrite::Nonce(_, nonce) => {
                                    if self.journal.created.contains(&index) {
                                        *nonce = 0;
                                    } else {
                                        *nonce = nonce.saturating_sub(
                                            after.nonce.saturating_sub(previous.nonce),
                                        );
                                    }
                                }
                                SurvivingWrite::Code(_, hash, code) => {
                                    if self.journal.code.contains(&index)
                                        || after.code_hash != previous.code_hash
                                    {
                                        *hash = previous.code_hash;
                                        *code = previous.code.clone();
                                    }
                                }
                                SurvivingWrite::Created(_, created) => {
                                    if self.journal.created.contains(&index) {
                                        *created = *previous_just_created;
                                    }
                                }
                                SurvivingWrite::Storage(..) => unreachable!(),
                            }
                        }
                        *after = previous;
                    }
                }
                JournalEntry::StorageChange { address, key, previous } => {
                    if let Some(index) = slots.get(&(*address, *key))
                        && let SurvivingWrite::Storage(_, _, value) = &mut writes[*index]
                    {
                        *value = *previous;
                    }
                }
                _ => {}
            }
        }
        for (_, frame) in frames {
            frame.writes.extend(writes.iter().map(|w| (w.field(), w.clone())));
        }
        Ok(())
    }

    /// Repairs host-affected fields after the native engine has undone a failed frame.
    pub fn finish_frame(&mut self, state: &mut State<'_>, success: bool) -> DbResult<()> {
        let Some(mut frame) = self.frames.pop() else { return Ok(()) };
        if self.frames.is_empty() {
            self.replaced_journal = success && frame.displaced.is_some();
        }
        if success && frame.creating {
            // Successful CREATE installs runtime code after executing the constructor, even
            // for empty code. Intercepted creates never initialized a constructor frame.
            self.code_changed(state.journal().len() - 1);
        }
        if !success {
            if let Some(displaced) = frame.displaced.take() {
                let (displaced, journal) = *displaced;
                let mut current = mem::replace(state, State::new(EmptyDB::default()));
                let cache = mem::take(&mut current.overlay_db_mut().cache);
                let prewarm = current.prewarm_set().clone();
                *state = displaced.into_state(current.into_db());
                // Snapshot restoration changes the accepted context, not just the transaction.
                // Only its displaced transaction state participates in enclosing rollback.
                state.overlay_db_mut().cache = cache;
                state.set_prewarm_set(prewarm);
                state.rollback(frame.checkpoint, frame.features);
                self.journal = journal;
            }
            let end = state.journal().len();
            self.journal.excluded.retain_mut(|span| {
                span.range.end = span.range.end.min(end);
                span.before.retain(|_, (index, _)| *index < end);
                span.range.start < span.range.end
            });
            self.journal.code.retain(|index| *index < end);
            self.journal.created.retain(|index| *index < end);
            for write in frame.writes.into_values() {
                write.apply(state)?;
            }
            self.exclude(state, end..state.journal().len());
        } else if let Some(parent) = self.frames.last_mut()
            && parent.displaced.is_none()
        {
            parent.displaced = frame.displaced;
        }
        Ok(())
    }

    /// Discards corrections when the running fork is replaced.
    pub(crate) fn clear(&mut self) {
        self.journal = JournalAnnotations::default();
        self.restored_snapshot = false;
        self.replaced_fork = true;
        for frame in &mut self.frames {
            frame.checkpoint = StateCheckpoint::new(0, 0);
            frame.writes.clear();
            frame.displaced = None;
        }
    }

    /// Captures the caller's state before a successful snapshot restoration displaces it.
    pub(crate) fn before_restore(&mut self, state: &State<'_>) {
        // The last frame is the intercepted cheatcode call, not the caller owning the restore.
        if let Some(frame) = self.frames.iter_mut().rev().nth(1)
            && frame.displaced.is_none()
        {
            frame.displaced = Some(Box::new((state.snapshot(), self.journal.clone())));
        }
        self.restored_snapshot = true;
        // Existing frames are behind the displacement barrier; new frames start after the
        // restored prefix. Neither needs annotations belonging to the snapshot's old journal.
        self.journal = JournalAnnotations::default();
    }

    /// Moves correction ownership when isolation adopts an entirely different state.
    pub fn replace_after_isolation(&mut self, state: &State<'_>, restored_snapshot: bool) {
        if !restored_snapshot {
            self.clear();
            return;
        }
        // Here the last frame is the isolated call itself, not the restore cheatcode.
        if let Some(frame) = self.frames.last_mut()
            && frame.displaced.is_none()
        {
            frame.displaced = Some(Box::new((state.snapshot(), mem::take(&mut self.journal))));
        }
        self.journal = JournalAnnotations::default();
        self.restored_snapshot = true;
    }

    fn exclude(&mut self, state: &State<'_>, range: Range<usize>) {
        if range.is_empty() {
            return;
        }
        if let Some(last) = self.journal.excluded.last_mut()
            && last.range.end == range.start
        {
            last.range.end = range.end;
        } else {
            self.journal
                .excluded
                .push(HostWrites { range: range.clone(), before: AddressHashMap::default() });
        }
        let span = self.journal.excluded.last_mut().unwrap();
        for index in range {
            if let JournalEntry::AccountChange { address, previous, .. } = &state.journal()[index] {
                span.before
                    .entry(*address)
                    .or_insert_with(|| (index, previous.clone().unwrap_or_default()));
            }
        }
    }
}
