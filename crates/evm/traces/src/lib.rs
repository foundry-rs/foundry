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
    collections::{BTreeMap, BTreeSet},
    ops::{Deref, DerefMut},
};

use alloy_primitives::{U256, map::HashMap};
use tempo_contracts::precompiles::TIP20_CHANNEL_RESERVE_ADDRESS;

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
pub mod speedscope;

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

    // Remove overlapping decoded trace data
    let mut cloned_arena = arena.clone();
    for node in cloned_arena.nodes_mut() {
        for step in &mut node.trace.steps {
            if step.decoded.is_some() {
                step.storage_change = None;
            }
        }
    }

    let resolved = cloned_arena.resolve_arena();
    let mut w = TraceWriter::new(Vec::<u8>::new())
        .color_cheatcodes(true)
        .use_colors(convert_color_choice(shell::color_choice()))
        .write_bytecodes(with_bytecodes)
        .with_storage_changes(with_storage_changes);
    w.write_arena(&resolved).expect("Failed to write traces");
    let mut rendered =
        String::from_utf8(w.into_writer()).expect("trace writer wrote invalid UTF-8");
    if with_storage_changes {
        append_tempo_channel_storage_decodes(&mut rendered, &arena.resolve_arena());
    }
    rendered
}

fn append_tempo_channel_storage_decodes(rendered: &mut String, arena: &CallTraceArena) {
    let decoded_changes = arena
        .nodes()
        .iter()
        .filter(|node| node.trace.address == TIP20_CHANNEL_RESERVE_ADDRESS)
        .flat_map(compact_channel_storage_changes)
        .collect::<Vec<_>>();

    if decoded_changes.is_empty() {
        return;
    }

    if !rendered.ends_with('\n') {
        rendered.push('\n');
    }
    rendered.push_str("Decoded TIP20ChannelReserve storage:\n");
    for (slot, before, after) in decoded_changes {
        rendered.push_str(&format!(
            "  @ {}: {} -> {}\n",
            format_storage_word(slot),
            format_channel_state(before),
            format_channel_state(after),
        ));
    }
}

fn compact_channel_storage_changes(node: &CallTraceNode) -> Vec<(U256, U256, U256)> {
    let mut changes_map = BTreeMap::new();
    for step in &node.trace.steps {
        if let Some(change) = &step.storage_change
            && change.had_value.is_some()
        {
            let (_first, last) = changes_map.entry(change.key).or_insert((&**change, &**change));
            *last = &**change;
        }
    }

    changes_map
        .into_iter()
        .filter_map(|(key, (first, last))| {
            let before = first.had_value.unwrap_or_default();
            let after = last.value;
            (before != after).then_some((key, before, after))
        })
        .collect()
}

fn format_channel_state(value: U256) -> String {
    let (settled, deposit, close_requested_at) = decode_channel_state(value);
    format!("{{settled: {settled}, deposit: {deposit}, closeRequestedAt: {close_requested_at}}}")
}

fn decode_channel_state(value: U256) -> (U256, U256, u32) {
    let mask96 = (U256::from(1) << 96) - U256::from(1);
    let mask32 = (U256::from(1) << 32) - U256::from(1);
    let settled: U256 = value & mask96;
    let deposit: U256 = (value >> 96usize) & mask96;
    let close_requested_at_word: U256 = (value >> 192usize) & mask32;
    let close_requested_at = close_requested_at_word.to::<u32>();
    (settled, deposit, close_requested_at)
}

fn format_storage_word(value: U256) -> String {
    if value < U256::from(1_000_000u64) { value.to_string() } else { format!("0x{value:x}") }
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InternalTraceMode {
    #[default]
    None,
    /// Traces internal functions without decoding inputs/outputs from memory.
    Simple,
    /// Same as `Simple`, but also tracks memory snapshots.
    Full,
}

/// Opcode step recording granularity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StepRecording {
    /// No opcode steps.
    #[default]
    None,
    /// Record only JUMP/JUMPDEST steps.
    Jumps,
    /// Record all opcode steps.
    All,
}

impl StepRecording {
    const fn merge(self, other: Self) -> Self {
        match (self, other) {
            (Self::All, _) | (_, Self::All) => Self::All,
            (Self::Jumps, _) | (_, Self::Jumps) => Self::Jumps,
            (Self::None, Self::None) => Self::None,
        }
    }
}

/// Trace data requirements composed across independent feature axes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TraceRequirements {
    calls: bool,
    steps: StepRecording,
    memory_snapshots: bool,
    stack_snapshots: bool,
    returndata_snapshots: bool,
    immediate_bytes: bool,
    state_diff: bool,
}

impl TraceRequirements {
    pub const fn none() -> Self {
        Self {
            calls: false,
            steps: StepRecording::None,
            memory_snapshots: false,
            stack_snapshots: false,
            returndata_snapshots: false,
            immediate_bytes: false,
            state_diff: false,
        }
    }

    pub const fn with_calls(mut self, yes: bool) -> Self {
        self.calls |= yes;
        self
    }

    pub const fn merge(mut self, other: Self) -> Self {
        self.calls |= other.calls;
        self.steps = self.steps.merge(other.steps);
        self.memory_snapshots |= other.memory_snapshots;
        self.stack_snapshots |= other.stack_snapshots;
        self.returndata_snapshots |= other.returndata_snapshots;
        self.immediate_bytes |= other.immediate_bytes;
        self.state_diff |= other.state_diff;
        self
    }

    pub const fn with_steps(mut self, steps: StepRecording) -> Self {
        self.steps = self.steps.merge(steps);
        self
    }

    pub const fn with_memory_snapshots(mut self, yes: bool) -> Self {
        self.memory_snapshots |= yes;
        self
    }

    pub const fn with_stack_snapshots(mut self, yes: bool) -> Self {
        self.stack_snapshots |= yes;
        self
    }

    pub const fn with_debug(mut self, yes: bool) -> Self {
        if yes {
            self.calls = true;
            self.steps = StepRecording::All;
            self.memory_snapshots = true;
            self.stack_snapshots = true;
            self.returndata_snapshots = true;
            self.immediate_bytes = true;
            self.state_diff = true;
        }
        self
    }

    pub const fn with_decode_internal(self, mode: InternalTraceMode) -> Self {
        match mode {
            InternalTraceMode::None => self,
            InternalTraceMode::Simple => {
                self.with_calls(true).with_steps(StepRecording::Jumps).with_stack_snapshots(true)
            }
            InternalTraceMode::Full => self
                .with_calls(true)
                .with_steps(StepRecording::Jumps)
                .with_memory_snapshots(true)
                .with_stack_snapshots(true),
        }
    }

    pub const fn with_all_steps(self, yes: bool) -> Self {
        if yes { self.with_calls(true).with_steps(StepRecording::All) } else { self }
    }

    pub const fn with_state_changes(mut self, yes: bool) -> Self {
        self.state_diff |= yes;
        if yes {
            self.calls = true;
        }
        self
    }

    pub const fn with_verbosity(self, verbosity: u8) -> Self {
        match verbosity {
            0..3 => self,
            3..=4 => self.with_calls(true),
            _ if matches!(self.steps, StepRecording::All) => self.with_calls(true),
            _ => self.with_state_changes(true),
        }
    }

    pub fn into_config(self) -> Option<TracingInspectorConfig> {
        if !self.calls && self.steps == StepRecording::None && !self.state_diff {
            return None;
        }

        let steps = if self.state_diff { StepRecording::All } else { self.steps };
        TracingInspectorConfig {
            record_steps: steps != StepRecording::None,
            record_memory_snapshots: self.memory_snapshots,
            record_stack_snapshots: if self.stack_snapshots {
                StackSnapshotType::Full
            } else {
                StackSnapshotType::None
            },
            record_logs: true,
            record_state_diff: self.state_diff,
            record_returndata_snapshots: self.returndata_snapshots,
            record_opcodes_filter: match steps {
                StepRecording::None | StepRecording::All => None,
                StepRecording::Jumps => {
                    Some(OpcodeFilter::new().enabled(OpCode::JUMP).enabled(OpCode::JUMPDEST))
                }
            },
            exclude_precompile_calls: false,
            record_immediate_bytes: self.immediate_bytes,
        }
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_tip1034_packed_channel_state() {
        let settled = U256::from(123u64);
        let deposit = U256::from(456u64);
        let close_requested_at = U256::from(1_780_495_200u64);
        let packed = settled | (deposit << 96usize) | (close_requested_at << 192usize);

        assert_eq!(decode_channel_state(packed), (settled, deposit, 1_780_495_200));
        assert_eq!(
            format_channel_state(packed),
            "{settled: 123, deposit: 456, closeRequestedAt: 1780495200}"
        );
    }

    #[test]
    fn verbosity_0_through_2_is_noop() {
        for v in 0..=2 {
            assert_eq!(
                TraceRequirements::none().with_verbosity(v),
                TraceRequirements::none(),
                "v={v}"
            );
            assert_eq!(
                TraceRequirements::none().with_calls(true).with_verbosity(v),
                TraceRequirements::none().with_calls(true),
                "v={v}"
            );
            assert_eq!(
                TraceRequirements::none().with_debug(true).with_verbosity(v),
                TraceRequirements::none().with_debug(true),
                "v={v}"
            );
        }
    }

    #[test]
    fn verbosity_3_and_4_raises_to_call() {
        for v in 3..=4 {
            assert_eq!(
                TraceRequirements::none().with_verbosity(v),
                TraceRequirements::none().with_calls(true),
                "v={v}"
            );
            assert_eq!(
                TraceRequirements::none().with_debug(true).with_verbosity(v),
                TraceRequirements::none().with_debug(true),
                "v={v}"
            );
            assert_eq!(
                TraceRequirements::none().with_state_changes(true).with_verbosity(v),
                TraceRequirements::none().with_state_changes(true),
                "v={v}"
            );
        }
    }

    #[test]
    fn verbosity_5_raises_to_record_state_diff() {
        let state_changes = TraceRequirements::none().with_state_changes(true);

        assert_eq!(TraceRequirements::none().with_verbosity(5), state_changes);
        assert_eq!(TraceRequirements::none().with_calls(true).with_verbosity(5), state_changes);
        let cfg = TraceRequirements::none()
            .with_calls(true)
            .with_steps(StepRecording::Jumps)
            .with_verbosity(5)
            .into_config()
            .unwrap();
        assert!(cfg.record_state_diff);
        assert!(cfg.record_opcodes_filter.is_none());
        assert_eq!(
            TraceRequirements::none().with_debug(true).with_verbosity(5),
            TraceRequirements::none().with_debug(true)
        );
        assert_eq!(
            TraceRequirements::none().with_state_changes(true).with_verbosity(5),
            state_changes
        );
    }

    #[test]
    fn config_at_verbosity_0_is_none() {
        assert!(TraceRequirements::none().with_verbosity(0).into_config().is_none());
    }

    #[test]
    fn config_at_verbosity_3_records_calls_only() {
        let cfg = TraceRequirements::none().with_verbosity(3).into_config().unwrap();
        assert!(!cfg.record_steps, "verbosity 3 should not record steps");
        assert!(!cfg.record_state_diff, "verbosity 3 should not record state diff");
        assert!(cfg.record_logs, "verbosity 3 should record logs");
    }

    #[test]
    fn config_at_verbosity_5_records_steps_and_state_diff() {
        let cfg = TraceRequirements::none().with_verbosity(5).into_config().unwrap();
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
        let cfg = TraceRequirements::none().with_debug(true).into_config().unwrap();
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
        assert!(cfg.record_state_diff, "Debug should record storage accesses for the debugger");
    }

    #[test]
    fn requirements_preserve_internal_decode_with_state_diff() {
        let cfg = TraceRequirements::none()
            .with_decode_internal(InternalTraceMode::Full)
            .with_state_changes(true)
            .into_config()
            .unwrap();

        assert!(cfg.record_steps, "requirements should record opcode steps");
        assert!(cfg.record_memory_snapshots, "Full internal decoding needs memory snapshots");
        assert_eq!(
            cfg.record_stack_snapshots,
            StackSnapshotType::Full,
            "internal decoding needs stack snapshots"
        );
        assert!(cfg.record_state_diff, "state changes should be recorded");
        assert!(cfg.record_opcodes_filter.is_none(), "state diff needs unfiltered opcodes");
    }

    #[test]
    fn requirements_all_steps_avoid_debug_snapshots() {
        let cfg =
            TraceRequirements::none().with_all_steps(true).with_verbosity(5).into_config().unwrap();

        assert!(cfg.record_steps, "all steps must record opcode steps");
        assert!(cfg.record_opcodes_filter.is_none(), "all steps must record every opcode step");
        assert!(!cfg.record_memory_snapshots, "all steps should not record memory snapshots");
        assert_eq!(
            cfg.record_stack_snapshots,
            StackSnapshotType::None,
            "all steps should not record stack snapshots"
        );
        assert!(!cfg.record_returndata_snapshots, "all steps should not record returndata");
        assert!(!cfg.record_immediate_bytes, "all steps should not record immediate bytes");
        assert!(!cfg.record_state_diff, "all steps should not record state diffs");
    }
}
