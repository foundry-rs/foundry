//! # foundry-evm-traces
//!
//! EVM trace identifying and decoding.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

#[macro_use]
extern crate tracing;

use foundry_common::contracts::{ContractsByAddress, ContractsByArtifact};
use revm::interpreter::OpCode;
use revm_inspectors::tracing::{
    types::{DecodedTraceStep, TraceMemberOrder},
    OpcodeFilter,
};
use serde::{Deserialize, Serialize};
use std::{
    borrow::Cow,
    collections::{BTreeSet, HashMap},
    ops::{Deref, DerefMut},
};

pub use revm_inspectors::tracing::{
    types::{
        CallKind, CallLog, CallTrace, CallTraceNode, DecodedCallData, DecodedCallLog,
        DecodedCallTrace,
    },
    CallTraceArena, FourByteInspector, GethTraceBuilder, ParityTraceBuilder, StackSnapshotType,
    TraceWriter, TracingInspector, TracingInspectorConfig,
};

/// Call trace address identifiers.
///
/// Identifiers figure out what ABIs and labels belong to all the addresses of the trace.
pub mod identifier;
use identifier::{LocalTraceIdentifier, TraceIdentifier};

mod decoder;
pub use decoder::{CallTraceDecoder, CallTraceDecoderBuilder};

pub mod debug;
pub use debug::DebugTraceIdentifier;

pub mod folded_stack_trace;

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

                let mut iternal_calls = Vec::new();
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
                            // If this is an internal call beginning, track it in `iternal_calls`
                            if let Some(DecodedTraceStep::InternalCall(_, end_step_idx)) =
                                &nodes[node_idx].trace.steps[step_idx].decoded
                            {
                                iternal_calls.push((item_idx, remove, *end_step_idx));
                                // we decide if we should remove it later
                                remove = false;
                            }
                            // Handle ends of internal calls
                            iternal_calls.retain(|(start_item_idx, remove_start, end_step_idx)| {
                                if *end_step_idx != step_idx {
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

                    if let Some((end_node, end_step_idx)) = cur_ignore_end {
                        if node_idx == *end_node && item_idx == *end_step_idx {
                            *cur_ignore_end = None;
                        }
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
pub async fn decode_trace_arena(
    arena: &mut CallTraceArena,
    decoder: &CallTraceDecoder,
) -> Result<(), std::fmt::Error> {
    decoder.prefetch_signatures(arena.nodes()).await;
    decoder.populate_traces(arena.nodes_mut()).await;

    Ok(())
}

/// Render a collection of call traces to a string.
pub fn render_trace_arena(arena: &SparsedTraceArena) -> String {
    render_trace_arena_with_bytecodes(arena, false)
}

/// Render a collection of call traces to a string optionally including contract creation bytecodes.
pub fn render_trace_arena_with_bytecodes(
    arena: &SparsedTraceArena,
    with_bytecodes: bool,
) -> String {
    let mut w = TraceWriter::new(Vec::<u8>::new()).write_bytecodes(with_bytecodes);
    w.write_arena(&arena.resolve_arena()).expect("Failed to write traces");
    String::from_utf8(w.into_writer()).expect("trace writer wrote invalid UTF-8")
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
    pub fn is_deployment(self) -> bool {
        matches!(self, Self::Deployment)
    }

    /// Returns `true` if the trace kind is [`Setup`].
    ///
    /// [`Setup`]: TraceKind::Setup
    #[must_use]
    pub fn is_setup(self) -> bool {
        matches!(self, Self::Setup)
    }

    /// Returns `true` if the trace kind is [`Execution`].
    ///
    /// [`Execution`]: TraceKind::Execution
    #[must_use]
    pub fn is_execution(self) -> bool {
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
        for address in local_identifier.identify_addresses(decoder.trace_addresses(trace)) {
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
}

impl TraceMode {
    pub const fn is_none(self) -> bool {
        matches!(self, Self::None)
    }

    pub const fn is_call(self) -> bool {
        matches!(self, Self::Call)
    }

    pub const fn is_jump_simple(self) -> bool {
        matches!(self, Self::JumpSimple)
    }

    pub const fn is_jump(self) -> bool {
        matches!(self, Self::Jump)
    }

    pub const fn is_debug(self) -> bool {
        matches!(self, Self::Debug)
    }

    pub fn with_debug(self, yes: bool) -> Self {
        if yes {
            std::cmp::max(self, Self::Debug)
        } else {
            self
        }
    }

    pub fn with_decode_internal(self, mode: InternalTraceMode) -> Self {
        std::cmp::max(self, mode.into())
    }

    pub fn with_verbosity(self, verbosiy: u8) -> Self {
        if verbosiy >= 3 {
            std::cmp::max(self, Self::Call)
        } else {
            self
        }
    }

    pub fn into_config(self) -> Option<TracingInspectorConfig> {
        if self.is_none() {
            None
        } else {
            TracingInspectorConfig {
                record_steps: self >= Self::JumpSimple,
                record_memory_snapshots: self >= Self::Jump,
                record_stack_snapshots: if self >= Self::JumpSimple {
                    StackSnapshotType::Full
                } else {
                    StackSnapshotType::None
                },
                record_logs: true,
                record_state_diff: false,
                record_returndata_snapshots: self.is_debug(),
                record_opcodes_filter: (self.is_jump() || self.is_jump_simple())
                    .then(|| OpcodeFilter::new().enabled(OpCode::JUMP).enabled(OpCode::JUMPDEST)),
                exclude_precompile_calls: false,
                record_immediate_bytes: self.is_debug(),
            }
            .into()
        }
    }
}
