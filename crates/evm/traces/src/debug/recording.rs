//! Debug steps recorded straight into a SolDB step table.

use alloy_primitives::{Bytes, U256, hex};
use evm2::interpreter::op as opcode;
use soldb_core::{StepTable, StorageChange};
use std::{
    collections::BTreeMap,
    fmt,
    ops::Range,
    sync::{Arc, Mutex, MutexGuard},
};

/// The step table that the debug recorders of one session append to, in execution order.
///
/// Each executor call runs its own clone of the inspector stack. The clones share this sink, so
/// the steps of the deployment, the setup, and the test go into one table.
#[derive(Clone)]
pub struct DebugStepSink(Arc<Mutex<Option<StepTable>>>);

impl DebugStepSink {
    /// An empty sink.
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(Some(StepTable::new()))))
    }

    /// Takes the table, or returns `None` if a recorder holds it.
    pub fn take(&self) -> Option<StepTable> {
        self.lock().take()
    }

    /// Gives the table back.
    pub fn put(&self, table: StepTable) {
        *self.lock() = Some(table);
    }

    /// Locks the table. It is `None` while a recorder holds it.
    pub fn lock(&self) -> MutexGuard<'_, Option<StepTable>> {
        self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Whether `self` and `other` are the same sink.
    pub fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Default for DebugStepSink {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for DebugStepSink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DebugStepSink").finish_non_exhaustive()
    }
}

/// The steps that one inspector stack recorded for its call trace arena.
#[derive(Clone, Debug)]
pub struct DebugRecording {
    /// The table that holds the steps.
    pub sink: DebugStepSink,
    /// The steps of the arena in the sink's table.
    pub steps: Range<usize>,
    /// The steps of each arena node, by node index. The step indices count from `steps.start`.
    pub frames: Vec<RecordedFrame>,
}

/// The steps that one call or creation spans, its subcalls included.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RecordedFrame {
    /// The journal depth of the call, as the call trace records it.
    pub depth: usize,
    /// The first step of the frame.
    pub entry_step: usize,
    /// The step after the last step of the frame.
    pub exit_step: usize,
}

/// Converts the state of each step into SolDB's form, sharing the buffers of unchanged steps.
///
/// The debug recorder and the SolDB frontend both convert steps through this.
#[derive(Debug, Default)]
pub struct StepConverter {
    /// The last return data buffer and its hex encoding.
    return_data: Option<(Bytes, Arc<str>)>,
    /// The storage of the steps that touch no slot.
    no_storage: Arc<BTreeMap<String, String>>,
}

/// The storage slots that one step touches, in SolDB's form.
#[derive(Debug)]
pub struct StepStorage {
    /// The slot an `SLOAD` or `SSTORE` touches and the value it reads or writes, or no slot.
    pub storage: Arc<BTreeMap<String, String>>,
    /// The change an `SSTORE` makes, or an `SLOAD`'s read.
    pub storage_diff: Option<BTreeMap<String, StorageChange>>,
    /// The slot a `TLOAD` or `TSTORE` touches and the value it reads or writes.
    pub transient_storage: Option<BTreeMap<String, String>>,
}

impl StepConverter {
    /// The hex encoding of the return data `buffer`.
    pub fn return_data(&mut self, buffer: &Bytes) -> Arc<str> {
        match &self.return_data {
            // Unchanged return data is the same buffer, so comparing the pointers first skips
            // most full comparisons.
            Some((last, hex))
                if (last.as_ptr() == buffer.as_ptr() && last.len() == buffer.len())
                    || last == buffer =>
            {
                hex.clone()
            }
            _ => {
                let hex = Arc::<str>::from(hex::encode(buffer));
                self.return_data = Some((buffer.clone(), hex.clone()));
                hex
            }
        }
    }

    /// The slots that opcode `op` touches.
    ///
    /// `inputs` ends with the top stack words before the step, `loaded` is the value an `SLOAD`
    /// or `TLOAD` pushed, and `had_value` the value an `SSTORE` overwrote, if known.
    pub fn storage(
        &self,
        op: u8,
        inputs: &[U256],
        loaded: Option<U256>,
        had_value: Option<U256>,
    ) -> StepStorage {
        let touched = match op {
            opcode::SLOAD | opcode::TLOAD => inputs.last().copied().zip(loaded),
            opcode::SSTORE | opcode::TSTORE => match inputs {
                [.., value, slot] => Some((*slot, *value)),
                _ => None,
            },
            _ => None,
        };
        let mut step = StepStorage {
            storage: self.no_storage.clone(),
            storage_diff: None,
            transient_storage: None,
        };
        let Some((slot, value)) = touched else { return step };
        let (slot, value) = (quantity(slot), quantity(value));
        if matches!(op, opcode::SLOAD | opcode::SSTORE) {
            let before = had_value.filter(|_| op == opcode::SSTORE).map(quantity);
            let change = StorageChange { before, after: Some(value.clone()) };
            step.storage = Arc::new(BTreeMap::from([(slot.clone(), value)]));
            step.storage_diff = Some(BTreeMap::from([(slot, change)]));
        } else {
            step.transient_storage = Some(BTreeMap::from([(slot, value)]));
        }
        step
    }
}

/// Formats `value` as a JSON-RPC quantity.
pub fn quantity(value: U256) -> String {
    format!("{value:#x}")
}
