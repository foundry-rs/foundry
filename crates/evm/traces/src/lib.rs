//! # foundry-evm-traces
//!
//! EVM trace identifying and decoding.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg))]

#[macro_use]
extern crate foundry_common;

#[macro_use]
extern crate tracing;

use foundry_common::{
    contracts::{ContractsByAddress, ContractsByArtifact},
    shell,
};
use revm::bytecode::opcode::OpCode;
use revm_inspectors::tracing::{
    OpcodeFilter,
    types::{DecodedTraceStep, TraceMemberOrder},
};
use serde::{Deserialize, Serialize};
use std::{
    borrow::Cow,
    collections::BTreeSet,
    ops::{Deref, DerefMut},
};

use alloy_primitives::map::HashMap;

pub use revm_inspectors::tracing::{
    CallTraceArena, FourByteInspector, GethTraceBuilder, ParityTraceBuilder, StackSnapshotType,
    TraceWriter, TracingInspector, TracingInspectorConfig,
    types::{
        CallKind, CallLog, CallTrace, CallTraceNode, DecodedCallData, DecodedCallLog,
        DecodedCallTrace,
    },
};

/// Call trace address identifiers.
///
/// Identifiers figure out what ABIs and labels belong to all the addresses of the trace.
pub mod identifier;
use identifier::LocalTraceIdentifier;

mod decoder;
pub use decoder::{CallTraceDecoder, CallTraceDecoderBuilder};

pub mod debug;
pub use debug::DebugTraceIdentifier;

pub mod folded_stack_trace;

pub mod backtrace;

pub type Traces = Vec<(TraceKind, SparsedTraceArena)>;

/// Trace arena keeping track of ignored trace items.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SparsedTraceArena {
    /// Full trace arena.
    #[serde(flatten)]
    pub arena: CallTraceArena,
    /// Ranges of trace steps to ignore in format (start_node, start_step) -> (end_node, end_step).
    /// See `foundry_cheatcodes::utils::IgnoredTraces` for more information.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub ignored: HashMap<(usize, usize), (usize, usize)>,
}

impl SparsedTraceArena {
    /// Goes over entire trace arena and removes ignored trace items.
    fn resolve_arena(&self) -> Cow<'_, CallTraceArena> {
        if self.ignored.is_empty() {
            Cow::Borrowed(&self.arena)
        } else {
            let mut arena = self.arena.clone();

            fn clear_node(
                nodes: &mut [CallTraceNode],
                node_idx: usize,
                ignored: &HashMap<(usize, usize), (usize, usize)>,
                cur_ignore_end: &mut Option<(usize, usize)>,
            ) {
                // Prepend an additional None item to the ordering to handle the beginning of the
                // trace.
                let items = std::iter::once(None)
                    .chain(nodes[node_idx].ordering.clone().into_iter().map(Some))
                    .enumerate();

                let mut internal_calls = Vec::new();
                let mut items_to_remove = BTreeSet::new();
                for (item_idx, item) in items {
                    if let Some(end_node) = ignored.get(&(node_idx, item_idx)) {
                        *cur_ignore_end = Some(*end_node);
                    }

                    let mut remove = cur_ignore_end.is_some() & item.is_some();

                    match item {
                        // we only remove calls if they did not start/pause tracing
                        Some(TraceMemberOrder::Call(child_idx)) => {
                            clear_node(
                                nodes,
                                nodes[node_idx].children[child_idx],
                                ignored,
                                cur_ignore_end,
                            );
                            remove &= cur_ignore_end.is_some();
                        }
                        // we only remove decoded internal calls if they did not start/pause tracing
                        Some(TraceMemberOrder::Step(step_idx)) => {
                            // If this is an internal call beginning, track it in `internal_calls`
                            if let Some(decoded) = &nodes[node_idx].trace.steps[step_idx].decoded
                                && let DecodedTraceStep::InternalCall(_, end_step_idx) = &**decoded
                            {
                                internal_calls.push((item_idx, remove, *end_step_idx));
                                // we decide if we should remove it later
                                remove = false;
                            }
                            // Handle ends of internal calls
                            internal_calls.retain(|(start_item_idx, remove_start, end_idx)| {
                                if *end_idx != step_idx {
                                    return true;
                                }
                                // only remove start if end should be removed as well
                                if *remove_start && remove {
                                    items_to_remove.insert(*start_item_idx);
                                } else {
                                    remove = false;
                                }

                                false
                            });
                        }
                        _ => {}
                    }

                    if remove {
                        items_to_remove.insert(item_idx);
                    }

                    if let Some((end_node, end_step_idx)) = cur_ignore_end
                        && node_idx == *end_node
                        && item_idx == *end_step_idx
                    {
                        *cur_ignore_end = None;
                    }
                }

                for (offset, item_idx) in items_to_remove.into_iter().enumerate() {
                    nodes[node_idx].ordering.remove(item_idx - offset - 1);
                }
            }

            clear_node(arena.nodes_mut(), 0, &self.ignored, &mut None);

            Cow::Owned(arena)
        }
    }
}

impl Deref for SparsedTraceArena {
    type Target = CallTraceArena;

    fn deref(&self) -> &Self::Target {
        &self.arena
    }
}

impl DerefMut for SparsedTraceArena {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.arena
    }
}

/// Decode a collection of call traces.
///
/// The traces will be decoded using the given decoder, if possible.
pub async fn decode_trace_arena(arena: &mut CallTraceArena, decoder: &CallTraceDecoder) {
    decoder.prefetch_signatures(arena.nodes()).await;
    decoder.populate_traces(arena.nodes_mut()).await;
}

/// Render a collection of call traces to a string.
pub fn render_trace_arena(arena: &SparsedTraceArena) -> String {
    render_trace_arena_inner(arena, false, false)
}

/// Prunes trace depth if depth is provided as an argument
pub fn prune_trace_depth(arena: &mut CallTraceArena, depth: usize) {
    for node in arena.nodes_mut() {
        if node.trace.depth >= depth {
            node.ordering.clear();
        }
    }
}

/// Render a collection of call traces to a string optionally including contract creation bytecodes
/// and in JSON format.
pub fn render_trace_arena_inner(
    arena: &SparsedTraceArena,
    with_bytecodes: bool,
    with_storage_changes: bool,
) -> String {
    if shell::is_json() {
        return serde_json::to_string(&arena.resolve_arena()).expect("Failed to serialize traces");
    }

    let mut w = TraceWriter::new(Vec::<u8>::new())
        .color_cheatcodes(true)
        .use_colors(convert_color_choice(shell::color_choice()))
        .write_bytecodes(with_bytecodes)
        .with_storage_changes(with_storage_changes);
    w.write_arena(&arena.resolve_arena()).expect("Failed to write traces");
    String::from_utf8(w.into_writer()).expect("trace writer wrote invalid UTF-8")
}

const fn convert_color_choice(choice: shell::ColorChoice) -> revm_inspectors::ColorChoice {
    match choice {
        shell::ColorChoice::Auto => revm_inspectors::ColorChoice::Auto,
        shell::ColorChoice::Always => revm_inspectors::ColorChoice::Always,
        shell::ColorChoice::Never => revm_inspectors::ColorChoice::Never,
    }
}

/// Specifies the kind of trace.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TraceKind {
    Deployment,
    Setup,
    Execution,
}

impl TraceKind {
    /// Returns `true` if the trace kind is [`Deployment`].
    ///
    /// [`Deployment`]: TraceKind::Deployment
    #[must_use]
    pub const fn is_deployment(self) -> bool {
        matches!(self, Self::Deployment)
    }

    /// Returns `true` if the trace kind is [`Setup`].
    ///
    /// [`Setup`]: TraceKind::Setup
    #[must_use]
    pub const fn is_setup(self) -> bool {
        matches!(self, Self::Setup)
    }

    /// Returns `true` if the trace kind is [`Execution`].
    ///
    /// [`Execution`]: TraceKind::Execution
    #[must_use]
    pub const fn is_execution(self) -> bool {
        matches!(self, Self::Execution)
    }
}

/// Given a list of traces and artifacts, it returns a map connecting address to abi
pub fn load_contracts<'a>(
    traces: impl IntoIterator<Item = &'a CallTraceArena>,
    known_contracts: &ContractsByArtifact,
) -> ContractsByAddress {
    let mut local_identifier = LocalTraceIdentifier::new(known_contracts);
    let decoder = CallTraceDecoder::new();
    let mut contracts = ContractsByAddress::new();
    for trace in traces {
        for address in decoder.identify_addresses(trace, &mut local_identifier) {
            if let (Some(contract), Some(abi)) = (address.contract, address.abi) {
                contracts.insert(address.address, (contract, abi.into_owned()));
            }
        }
    }
    contracts
}

/// Different kinds of internal functions tracing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum InternalTraceMode {
    #[default]
    None,
    /// Traces internal functions without decoding inputs/outputs from memory.
    Simple,
    /// Same as `Simple`, but also tracks memory snapshots.
    Full,
}

impl From<InternalTraceMode> for TraceMode {
    fn from(mode: InternalTraceMode) -> Self {
        match mode {
            InternalTraceMode::None => Self::None,
            InternalTraceMode::Simple => Self::JumpSimple,
            InternalTraceMode::Full => Self::Jump,
        }
    }
}

// Different kinds of traces used by different foundry components.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum TraceMode {
    /// Disabled tracing.
    #[default]
    None,
    /// Simple call trace, no steps tracing required.
    Call,
    /// Call trace with steps tracing for JUMP and JUMPDEST opcodes.
    ///
    /// Does not enable tracking memory or stack snapshots.
    Steps,
    /// Call trace with tracing for JUMP and JUMPDEST opcode steps.
    ///
    /// Used for internal functions identification. Does not track memory snapshots.
    JumpSimple,
    /// Call trace with tracing for JUMP and JUMPDEST opcode steps.
    ///
    /// Same as `JumpSimple`, but tracks memory snapshots as well.
    Jump,
    /// Call trace with complete steps tracing.
    ///
    /// Used by debugger.
    Debug,
    /// Step trace with storage change recording.
    ///
    /// Records JUMP/JUMPDEST steps (like `Steps`) plus storage diffs on SLOAD/SSTORE.
    /// Does not enable memory/stack snapshots or unfiltered opcode recording.
    RecordStateDiff,
}

impl TraceMode {
    pub const fn is_none(self) -> bool {
        matches!(self, Self::None)
    }

    pub const fn is_call(self) -> bool {
        matches!(self, Self::Call)
    }

    pub const fn is_steps(self) -> bool {
        matches!(self, Self::Steps)
    }

    pub const fn is_jump_simple(self) -> bool {
        matches!(self, Self::JumpSimple)
    }

    pub const fn is_jump(self) -> bool {
        matches!(self, Self::Jump)
    }

    pub const fn record_state_diff(self) -> bool {
        matches!(self, Self::RecordStateDiff)
    }

    pub const fn is_debug(self) -> bool {
        matches!(self, Self::Debug)
    }

    pub fn with_debug(self, yes: bool) -> Self {
        if yes { std::cmp::max(self, Self::Debug) } else { self }
    }

    pub fn with_decode_internal(self, mode: InternalTraceMode) -> Self {
        std::cmp::max(self, mode.into())
    }

    pub fn with_state_changes(self, yes: bool) -> Self {
        if yes { std::cmp::max(self, Self::RecordStateDiff) } else { self }
    }

    pub fn with_verbosity(self, verbosity: u8) -> Self {
        match verbosity {
            0..3 => self,
            3..=4 => std::cmp::max(self, Self::Call),
            // Enable step recording and state diff recording when verbosity is 5 or higher.
            // This includes backtraces (JUMP/JUMPDEST steps) and storage changes.
            _ => std::cmp::max(self, Self::RecordStateDiff),
        }
    }

    pub fn into_config(self) -> Option<TracingInspectorConfig> {
        if self.is_none() {
            None
        } else {
            // RecordStateDiff is Steps + state diff recording, not Debug + state diff.
            // It should not enable memory/stack snapshots.
            // State diff recording requires all opcodes (no filter) since it needs
            // SLOAD/SSTORE steps, not just JUMP/JUMPDEST.
            let effective = if self.record_state_diff() { Self::Steps } else { self };
            TracingInspectorConfig {
                record_steps: self >= Self::Steps,
                record_memory_snapshots: effective >= Self::Jump,
                record_stack_snapshots: if effective > Self::Steps {
                    StackSnapshotType::Full
                } else {
                    StackSnapshotType::None
                },
                record_logs: true,
                record_state_diff: self.record_state_diff(),
                record_returndata_snapshots: effective.is_debug(),
                // State diff needs all opcodes recorded to capture SLOAD/SSTORE.
                record_opcodes_filter: if self.record_state_diff() {
                    None
                } else {
                    (effective.is_steps() || effective.is_jump() || effective.is_jump_simple())
                        .then(|| {
                            OpcodeFilter::new().enabled(OpCode::JUMP).enabled(OpCode::JUMPDEST)
                        })
                },
                exclude_precompile_calls: false,
                record_immediate_bytes: effective.is_debug(),
            }
            .into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- TraceMode::with_verbosity level tests --

    #[test]
    fn verbosity_0_through_2_is_noop() {
        for v in 0..=2 {
            assert_eq!(TraceMode::None.with_verbosity(v), TraceMode::None, "v={v}");
            assert_eq!(TraceMode::Call.with_verbosity(v), TraceMode::Call, "v={v}");
            assert_eq!(TraceMode::Debug.with_verbosity(v), TraceMode::Debug, "v={v}");
        }
    }

    #[test]
    fn verbosity_3_and_4_raises_to_call() {
        for v in 3..=4 {
            assert_eq!(TraceMode::None.with_verbosity(v), TraceMode::Call, "v={v}");
            // Already above Call — must not downgrade.
            assert_eq!(TraceMode::Debug.with_verbosity(v), TraceMode::Debug, "v={v}");
            assert_eq!(
                TraceMode::RecordStateDiff.with_verbosity(v),
                TraceMode::RecordStateDiff,
                "v={v}"
            );
        }
    }

    #[test]
    fn verbosity_5_raises_to_record_state_diff() {
        assert_eq!(TraceMode::None.with_verbosity(5), TraceMode::RecordStateDiff);
        assert_eq!(TraceMode::Call.with_verbosity(5), TraceMode::RecordStateDiff);
        assert_eq!(TraceMode::Steps.with_verbosity(5), TraceMode::RecordStateDiff);
        assert_eq!(TraceMode::Debug.with_verbosity(5), TraceMode::RecordStateDiff);
        // Already at the top — stays the same.
        assert_eq!(TraceMode::RecordStateDiff.with_verbosity(5), TraceMode::RecordStateDiff);
    }

    // -- into_config at each verbosity level --

    #[test]
    fn config_at_verbosity_0_is_none() {
        let mode = TraceMode::None.with_verbosity(0);
        assert!(mode.into_config().is_none());
    }

    #[test]
    fn config_at_verbosity_3_records_calls_only() {
        let cfg = TraceMode::None.with_verbosity(3).into_config().unwrap();
        assert!(!cfg.record_steps, "verbosity 3 should not record steps");
        assert!(!cfg.record_state_diff, "verbosity 3 should not record state diff");
        assert!(cfg.record_logs, "verbosity 3 should record logs");
    }

    #[test]
    fn config_at_verbosity_5_records_steps_and_state_diff() {
        let cfg = TraceMode::None.with_verbosity(5).into_config().unwrap();
        assert!(cfg.record_steps, "verbosity 5 must record steps for backtraces");
        assert!(cfg.record_state_diff, "verbosity 5 must record state diff");
        assert!(cfg.record_logs, "verbosity 5 must record logs");
        // RecordStateDiff should NOT enable expensive debug-level features.
        assert!(!cfg.record_memory_snapshots, "verbosity 5 should not record memory snapshots");
        assert_eq!(
            cfg.record_stack_snapshots,
            StackSnapshotType::None,
            "verbosity 5 should not record stack snapshots"
        );
        // State diff requires all opcodes to capture SLOAD/SSTORE, so no filter.
        assert!(
            cfg.record_opcodes_filter.is_none(),
            "verbosity 5 needs unfiltered opcodes for state diff"
        );
    }

    #[test]
    fn config_debug_mode_unchanged() {
        // Debug mode must still enable full recording for the debugger.
        let cfg = TraceMode::Debug.into_config().unwrap();
        assert!(cfg.record_steps);
        assert!(cfg.record_memory_snapshots, "Debug must record memory snapshots");
        assert_eq!(
            cfg.record_stack_snapshots,
            StackSnapshotType::Full,
            "Debug must record full stack snapshots"
        );
        assert!(cfg.record_returndata_snapshots, "Debug must record returndata");
        assert!(cfg.record_immediate_bytes, "Debug must record immediate bytes");
        assert!(cfg.record_opcodes_filter.is_none(), "Debug must record all opcodes (no filter)");
        assert!(!cfg.record_state_diff, "Debug alone should not record state diff");
    }
}
