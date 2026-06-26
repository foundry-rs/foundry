use alloy_primitives::{Address, Bytes, hex, map::AddressHashMap};
use foundry_evm_core::precompiles;
use foundry_evm_traces::{CallKind, CallTrace, CallTraceArena};
use revm::bytecode::opcode::OpCode;
use revm_inspectors::tracing::types::{
    CallTraceStep, DecodedCallTrace, DecodedTraceStep, TraceMemberOrder,
};
use serde::{Deserialize, Serialize};

/// Represents a part of the execution frame before the next call or end of the execution.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DebugNode {
    /// Execution context.
    ///
    /// Note that this is the address of the *code*, not necessarily the address of the storage.
    pub address: Address,
    /// The kind of call this is.
    pub kind: CallKind,
    /// Calldata of the call.
    pub calldata: Bytes,
    /// The gas limit of the call.
    pub gas_limit: u64,
    /// Stable id for the original call trace node within the flattened debugger arena.
    #[serde(default)]
    pub trace_node_idx: usize,
    /// Index of the first step in the original call trace node.
    #[serde(default)]
    pub step_offset: usize,
    /// Decoded call data for the current execution context, if available.
    #[serde(default)]
    pub decoded: Option<Box<DecodedCallTrace>>,
    /// The debug steps.
    pub steps: Vec<CallTraceStep>,
}

impl DebugNode {
    /// Creates a new debug node.
    pub const fn new(
        address: Address,
        kind: CallKind,
        steps: Vec<CallTraceStep>,
        calldata: Bytes,
        gas_limit: u64,
        decoded: Option<Box<DecodedCallTrace>>,
    ) -> Self {
        Self {
            address,
            kind,
            steps,
            calldata,
            gas_limit,
            trace_node_idx: 0,
            step_offset: 0,
            decoded,
        }
    }
}

/// Flattens given [CallTraceArena] into a list of [DebugNode]s.
///
/// This is done by recursively traversing the call tree and collecting the steps in-between the
/// calls.
#[cfg(test)]
fn flatten_call_trace(arena: CallTraceArena, out: &mut Vec<DebugNode>) {
    flatten_call_trace_with_precompiles(arena, out, &AddressHashMap::default());
}

/// Flattens given [CallTraceArena] into a list of [DebugNode]s using active precompile labels.
pub fn flatten_call_trace_with_precompiles(
    arena: CallTraceArena,
    out: &mut Vec<DebugNode>,
    precompile_labels: &AddressHashMap<String>,
) {
    #[derive(Debug, Clone, Copy)]
    struct PendingNode {
        node_idx: usize,
        steps_count: usize,
        step_offset: usize,
    }

    fn inner(
        arena: &CallTraceArena,
        node_idx: usize,
        out: &mut Vec<PendingNode>,
        step_notices: &mut Vec<(usize, usize, String)>,
        precompile_labels: &AddressHashMap<String>,
    ) {
        let mut pending = PendingNode { node_idx, steps_count: 0, step_offset: 0 };
        let mut next_step_offset = 0;
        let mut last_step_idx: Option<usize> = None;
        let node = &arena.nodes()[node_idx];
        for order in &node.ordering {
            match order {
                TraceMemberOrder::Call(idx) => {
                    let child_idx = node.children[*idx];
                    if let Some(step_idx) = last_step_idx.take()
                        && let Some(step) = node.trace.steps.get(step_idx)
                        && is_call_like_op(step.op)
                        && let Some(notice) = precompile_call_notice(
                            &arena.nodes()[child_idx].trace,
                            precompile_labels,
                        )
                    {
                        step_notices.push((node_idx, step_idx, notice));
                    }
                    out.push(pending);
                    pending =
                        PendingNode { node_idx, steps_count: 0, step_offset: next_step_offset };
                    inner(arena, child_idx, out, step_notices, precompile_labels);
                }
                TraceMemberOrder::Step(step_idx) => {
                    if pending.steps_count == 0 {
                        pending.step_offset = *step_idx;
                    }
                    pending.steps_count += 1;
                    next_step_offset = step_idx.saturating_add(1);
                    last_step_idx = Some(*step_idx);
                }
                _ => {
                    last_step_idx = None;
                }
            }
        }
        out.push(pending);
    }
    let mut nodes = Vec::new();
    let mut step_notices = Vec::new();
    inner(&arena, 0, &mut nodes, &mut step_notices, precompile_labels);

    let mut arena_nodes = arena.into_nodes();
    for (node_idx, step_idx, notice) in step_notices {
        if let Some(step) =
            arena_nodes.get_mut(node_idx).and_then(|node| node.trace.steps.get_mut(step_idx))
        {
            set_step_notice(step, notice);
        }
    }

    let trace_node_idx_offset =
        out.iter().map(|node| node.trace_node_idx).max().map_or(0, |idx| idx.saturating_add(1));

    for pending in nodes {
        let steps = {
            let other_steps =
                arena_nodes[pending.node_idx].trace.steps.split_off(pending.steps_count);
            std::mem::replace(&mut arena_nodes[pending.node_idx].trace.steps, other_steps)
        };

        // Skip nodes with empty steps as there's nothing to display for them.
        if steps.is_empty() {
            continue;
        }

        let call = &arena_nodes[pending.node_idx].trace;
        let calldata = if call.kind.is_any_create() { Bytes::new() } else { call.data.clone() };
        let mut node = DebugNode::new(
            call.address,
            call.kind,
            steps,
            calldata,
            call.gas_limit,
            call.decoded.clone(),
        );
        node.trace_node_idx = trace_node_idx_offset.saturating_add(pending.node_idx);
        node.step_offset = pending.step_offset;

        out.push(node);
    }
}

fn set_step_notice(step: &mut CallTraceStep, notice: String) {
    match step.decoded.as_deref_mut() {
        None => step.decoded = Some(Box::new(DecodedTraceStep::Line(notice))),
        Some(DecodedTraceStep::Line(line)) if line.is_empty() => *line = notice,
        Some(_) => {}
    }
}

const fn is_call_like_op(op: OpCode) -> bool {
    matches!(
        op,
        OpCode::CALL
            | OpCode::STATICCALL
            | OpCode::DELEGATECALL
            | OpCode::CALLCODE
            | OpCode::CREATE
            | OpCode::CREATE2
    )
}

const PRECOMPILES_TRACE_LABEL: &str = "PRECOMPILES";

fn precompile_call_notice(
    trace: &CallTrace,
    precompile_labels: &AddressHashMap<String>,
) -> Option<String> {
    decoded_precompile_call_notice(&trace.decoded, trace.maybe_precompile.unwrap_or(false))
        .or_else(|| known_precompile_call_notice(trace))
        .or_else(|| labeled_precompile_call_notice(trace, precompile_labels))
        .or_else(|| marked_precompile_call_notice(trace))
}

fn decoded_precompile_call_notice(
    decoded: &Option<Box<DecodedCallTrace>>,
    is_precompile: bool,
) -> Option<String> {
    let decoded = decoded.as_ref()?;
    let label = decoded.label.as_deref()?;
    if label != PRECOMPILES_TRACE_LABEL && !is_precompile {
        return None;
    }

    Some(decoded_call_notice(label, decoded))
}

fn decoded_call_notice(label: &str, decoded: &DecodedCallTrace) -> String {
    let Some(call_data) = &decoded.call_data else {
        return format!("precompile: {label}");
    };
    let args = call_data.args.join(", ");
    let function_name =
        call_data.signature.split_once('(').map_or(call_data.signature.as_str(), |(name, _)| name);
    let mut notice = format!("precompile: {label}::{function_name}({args})");
    if let Some(return_data) = decoded.return_data.as_deref() {
        notice.push_str(" -> ");
        notice.push_str(return_data);
    }
    notice
}

fn labeled_precompile_call_notice(
    trace: &CallTrace,
    precompile_labels: &AddressHashMap<String>,
) -> Option<String> {
    let label = precompile_labels.get(&trace.address)?;
    if let Some(decoded) = trace.decoded.as_ref() {
        return Some(decoded_call_notice(
            decoded.label.as_deref().unwrap_or(label.as_str()),
            decoded,
        ));
    }

    let mut notice = format!("precompile: {label} @ {}", trace.address);
    append_raw_precompile_io(&mut notice, trace);
    Some(notice)
}

fn marked_precompile_call_notice(trace: &CallTrace) -> Option<String> {
    trace.maybe_precompile.unwrap_or(false).then(|| {
        let mut notice = format!("precompile: {}", trace.address);
        append_raw_precompile_io(&mut notice, trace);
        notice
    })
}

fn known_precompile_call_notice(trace: &CallTrace) -> Option<String> {
    let name = known_precompile_name(trace.address)?;
    let mut notice = format!("precompile: {name} @ {}", trace.address);
    append_raw_precompile_io(&mut notice, trace);
    Some(notice)
}

fn append_raw_precompile_io(notice: &mut String, trace: &CallTrace) {
    if !trace.data.is_empty() {
        notice.push_str(" input=");
        notice.push_str(&hex::encode_prefixed(&trace.data));
    }
    if !trace.output.is_empty() {
        notice.push_str(" output=");
        notice.push_str(&hex::encode_prefixed(&trace.output));
    }
}

// Standard EVM fallback for traces that have no decoded precompile metadata or execution marker.
const fn known_precompile_name(address: Address) -> Option<&'static str> {
    match address {
        precompiles::EC_RECOVER => Some("ecrecover"),
        precompiles::SHA_256 => Some("sha256"),
        precompiles::RIPEMD_160 => Some("ripemd"),
        precompiles::IDENTITY => Some("identity"),
        precompiles::MOD_EXP => Some("modexp"),
        precompiles::EC_ADD => Some("ecadd"),
        precompiles::EC_MUL => Some("ecmul"),
        precompiles::EC_PAIRING => Some("ecpairing"),
        precompiles::BLAKE_2F => Some("blake2f"),
        precompiles::POINT_EVALUATION => Some("pointEvaluation"),
        precompiles::BLS12_G1ADD => Some("bls12G1Add"),
        precompiles::BLS12_G1MSM => Some("bls12G1Msm"),
        precompiles::BLS12_G2ADD => Some("bls12G2Add"),
        precompiles::BLS12_G2MSM => Some("bls12G2Msm"),
        precompiles::BLS12_PAIRING_CHECK => Some("bls12PairingCheck"),
        precompiles::BLS12_MAP_FP_TO_G1 => Some("bls12MapFpToG1"),
        precompiles::BLS12_MAP_FP2_TO_G2 => Some("bls12MapFp2ToG2"),
        precompiles::P256_VERIFY => Some("p256Verify"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use foundry_evm_traces::CallTraceNode;
    use revm::interpreter::InstructionResult;
    use revm_inspectors::tracing::types::{DecodedCallData, DecodedInternalCall};

    fn step(pc: usize) -> CallTraceStep {
        CallTraceStep {
            pc,
            op: OpCode::STOP,
            stack: None,
            push_stack: None,
            memory: None,
            returndata: Bytes::new(),
            gas_remaining: 0,
            gas_refund_counter: 0,
            gas_used: 0,
            gas_cost: 0,
            storage_change: None,
            status: Some(InstructionResult::Stop),
            immediate_bytes: None,
            decoded: None,
        }
    }

    fn step_with_op(pc: usize, op: OpCode) -> CallTraceStep {
        CallTraceStep { op, ..step(pc) }
    }

    fn known_sha256_precompile_node() -> CallTraceNode {
        CallTraceNode {
            parent: Some(0),
            idx: 1,
            trace: CallTrace {
                address: precompiles::SHA_256,
                data: Bytes::from_static(b"hello"),
                output: alloy_primitives::hex!(
                    "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
                )
                .into(),
                ..Default::default()
            },
            ordering: Vec::new(),
            ..Default::default()
        }
    }

    fn arena_with_child_after_staticcall(child_trace: CallTrace) -> CallTraceArena {
        let mut arena = CallTraceArena::default();

        {
            let root = &mut arena.nodes_mut()[0];
            root.trace.steps =
                vec![step_with_op(0, OpCode::STATICCALL), step_with_op(1, OpCode::STOP)];
            root.ordering = vec![
                TraceMemberOrder::Step(0),
                TraceMemberOrder::Call(0),
                TraceMemberOrder::Step(1),
            ];
            root.children.push(1);
        }

        arena.nodes_mut().push(CallTraceNode {
            parent: Some(0),
            idx: 1,
            trace: child_trace,
            ordering: Vec::new(),
            ..Default::default()
        });

        arena
    }

    fn decoded_fee_manager_trace(maybe_precompile: Option<bool>) -> CallTrace {
        CallTrace {
            address: Address::from([0x42; 20]),
            maybe_precompile,
            decoded: Some(Box::new(DecodedCallTrace {
                label: Some("FeeManager".to_string()),
                call_data: Some(DecodedCallData {
                    signature: "userTokens(address)".to_string(),
                    args: vec!["0x0000000000000000000000000000000000000000".to_string()],
                }),
                return_data: Some("0x0000000000000000000000000000000000000000".to_string()),
            })),
            ..Default::default()
        }
    }

    fn assert_no_precompile_notice(step: &CallTraceStep) {
        assert!(
            !matches!(
                step.decoded.as_deref(),
                Some(DecodedTraceStep::Line(line)) if line.starts_with("precompile:")
            ),
            "unexpected precompile notice: {:?}",
            step.decoded
        );
    }

    fn assert_precompile_notice(step: &CallTraceStep) -> &str {
        let Some(DecodedTraceStep::Line(notice)) = step.decoded.as_deref() else {
            panic!("missing precompile notice");
        };
        notice
    }

    #[test]
    fn flatten_records_original_step_offsets_for_split_segments() {
        let mut arena = CallTraceArena::default();

        {
            let root = &mut arena.nodes_mut()[0];
            root.trace.steps = vec![step(0), step(1), step(2)];
            root.ordering = vec![
                TraceMemberOrder::Step(0),
                TraceMemberOrder::Call(0),
                TraceMemberOrder::Step(1),
                TraceMemberOrder::Step(2),
            ];
            root.children.push(1);
        }

        arena.nodes_mut().push(CallTraceNode {
            parent: Some(0),
            idx: 1,
            trace: CallTrace { kind: CallKind::Call, steps: vec![step(10)], ..Default::default() },
            ordering: vec![TraceMemberOrder::Step(0)],
            ..Default::default()
        });

        let mut flattened = Vec::new();
        flatten_call_trace(arena, &mut flattened);

        assert_eq!(flattened.len(), 3);
        assert_eq!((flattened[0].trace_node_idx, flattened[0].step_offset), (0, 0));
        assert_eq!(flattened[0].steps.iter().map(|step| step.pc).collect::<Vec<_>>(), [0]);
        assert_eq!((flattened[1].trace_node_idx, flattened[1].step_offset), (1, 0));
        assert_eq!(flattened[1].steps.iter().map(|step| step.pc).collect::<Vec<_>>(), [10]);
        assert_eq!((flattened[2].trace_node_idx, flattened[2].step_offset), (0, 1));
        assert_eq!(flattened[2].steps.iter().map(|step| step.pc).collect::<Vec<_>>(), [1, 2]);
    }

    #[test]
    fn flatten_keeps_trace_node_ids_unique_when_appending_multiple_arenas() {
        let mut flattened = vec![DebugNode { trace_node_idx: 1, ..Default::default() }];

        let mut arena = CallTraceArena::default();
        {
            let root = &mut arena.nodes_mut()[0];
            root.trace.steps = vec![step(0)];
            root.ordering = vec![TraceMemberOrder::Step(0)];
        }

        flatten_call_trace(arena, &mut flattened);

        assert_eq!(flattened[1].trace_node_idx, 2);
    }

    #[test]
    fn flatten_annotates_parent_step_for_precompile_child_calls() {
        let mut arena = CallTraceArena::default();

        {
            let root = &mut arena.nodes_mut()[0];
            root.trace.steps =
                vec![step_with_op(0, OpCode::STATICCALL), step_with_op(1, OpCode::STOP)];
            root.ordering = vec![
                TraceMemberOrder::Step(0),
                TraceMemberOrder::Call(0),
                TraceMemberOrder::Step(1),
            ];
            root.children.push(1);
        }

        arena.nodes_mut().push(CallTraceNode {
            parent: Some(0),
            idx: 1,
            trace: CallTrace {
                decoded: Some(Box::new(DecodedCallTrace {
                    label: Some("PRECOMPILES".to_string()),
                    call_data: Some(DecodedCallData {
                        signature: "sha256(bytes)".to_string(),
                        args: vec!["0x68656c6c6f".to_string()],
                    }),
                    return_data: Some(
                        "0x2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
                            .to_string(),
                    ),
                })),
                ..Default::default()
            },
            ordering: Vec::new(),
            ..Default::default()
        });

        let mut flattened = Vec::new();
        flatten_call_trace(arena, &mut flattened);

        let Some(DecodedTraceStep::Line(notice)) = flattened[0].steps[0].decoded.as_deref() else {
            panic!("missing precompile notice");
        };
        assert_eq!(
            notice,
            "precompile: PRECOMPILES::sha256(0x68656c6c6f) -> 0x2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn flatten_marks_known_precompile_child_calls_without_decoded_trace() {
        let mut arena = CallTraceArena::default();

        {
            let root = &mut arena.nodes_mut()[0];
            root.trace.steps =
                vec![step_with_op(0, OpCode::STATICCALL), step_with_op(1, OpCode::STOP)];
            root.ordering = vec![
                TraceMemberOrder::Step(0),
                TraceMemberOrder::Call(0),
                TraceMemberOrder::Step(1),
            ];
            root.children.push(1);
        }

        arena.nodes_mut().push(known_sha256_precompile_node());

        let mut flattened = Vec::new();
        flatten_call_trace(arena, &mut flattened);

        let Some(DecodedTraceStep::Line(notice)) = flattened[0].steps[0].decoded.as_deref() else {
            panic!("missing precompile notice");
        };
        assert_eq!(
            notice,
            "precompile: sha256 @ 0x0000000000000000000000000000000000000002 input=0x68656c6c6f output=0x2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn flatten_annotates_marked_precompile_child_calls_with_decoded_label() {
        let arena = arena_with_child_after_staticcall(decoded_fee_manager_trace(Some(true)));

        let mut flattened = Vec::new();
        flatten_call_trace(arena, &mut flattened);

        assert_eq!(
            assert_precompile_notice(&flattened[0].steps[0]),
            "precompile: FeeManager::userTokens(0x0000000000000000000000000000000000000000) -> 0x0000000000000000000000000000000000000000"
        );
    }

    #[test]
    fn flatten_annotates_chain_labeled_precompile_child_calls_with_decoded_label() {
        let address = Address::from([0x42; 20]);
        let arena = arena_with_child_after_staticcall(decoded_fee_manager_trace(None));
        let precompile_labels = AddressHashMap::from_iter([(address, "FeeManager".to_string())]);

        let mut flattened = Vec::new();
        flatten_call_trace_with_precompiles(arena, &mut flattened, &precompile_labels);

        assert_eq!(
            assert_precompile_notice(&flattened[0].steps[0]),
            "precompile: FeeManager::userTokens(0x0000000000000000000000000000000000000000) -> 0x0000000000000000000000000000000000000000"
        );
    }

    #[test]
    fn flatten_marks_marked_precompile_child_calls_without_decoded_trace() {
        let arena = arena_with_child_after_staticcall(CallTrace {
            address: Address::from([0x99; 20]),
            maybe_precompile: Some(true),
            data: Bytes::from_static(&[0x12, 0x34]),
            output: Bytes::from_static(&[0x56]),
            ..Default::default()
        });

        let mut flattened = Vec::new();
        flatten_call_trace(arena, &mut flattened);

        assert_eq!(
            assert_precompile_notice(&flattened[0].steps[0]),
            "precompile: 0x9999999999999999999999999999999999999999 input=0x1234 output=0x56"
        );
    }

    #[test]
    fn flatten_skips_decoded_label_without_precompile_marker() {
        let arena = arena_with_child_after_staticcall(decoded_fee_manager_trace(None));

        let mut flattened = Vec::new();
        flatten_call_trace(arena, &mut flattened);

        assert_no_precompile_notice(&flattened[0].steps[0]);
    }

    #[test]
    fn flatten_preserves_existing_decoded_step_metadata_on_precompile_child_calls() {
        let mut arena = CallTraceArena::default();

        {
            let root = &mut arena.nodes_mut()[0];
            let mut call_step = step_with_op(0, OpCode::STATICCALL);
            call_step.decoded = Some(Box::new(DecodedTraceStep::InternalCall(
                DecodedInternalCall {
                    func_name: "DebugMe::foo".to_string(),
                    args: Some(vec!["1".to_string()]),
                    return_data: Some(vec!["2".to_string()]),
                },
                1,
            )));
            root.trace.steps = vec![call_step, step_with_op(1, OpCode::STOP)];
            root.ordering = vec![
                TraceMemberOrder::Step(0),
                TraceMemberOrder::Call(0),
                TraceMemberOrder::Step(1),
            ];
            root.children.push(1);
        }
        arena.nodes_mut().push(known_sha256_precompile_node());

        let mut flattened = Vec::new();
        flatten_call_trace(arena, &mut flattened);

        let Some(DecodedTraceStep::InternalCall(decoded, end_step)) =
            flattened[0].steps[0].decoded.as_deref()
        else {
            panic!("expected existing internal call metadata to be preserved");
        };
        assert_eq!(decoded.func_name, "DebugMe::foo");
        assert_eq!(*end_step, 1);
    }

    #[test]
    fn flatten_skips_precompile_notice_without_preceding_step() {
        let mut arena = CallTraceArena::default();

        {
            let root = &mut arena.nodes_mut()[0];
            root.trace.steps = vec![step_with_op(0, OpCode::STOP)];
            root.ordering = vec![TraceMemberOrder::Call(0), TraceMemberOrder::Step(0)];
            root.children.push(1);
        }
        arena.nodes_mut().push(known_sha256_precompile_node());

        let mut flattened = Vec::new();
        flatten_call_trace(arena, &mut flattened);

        assert_no_precompile_notice(&flattened[0].steps[0]);
    }

    #[test]
    fn flatten_skips_precompile_notice_after_non_call_step() {
        let mut arena = CallTraceArena::default();

        {
            let root = &mut arena.nodes_mut()[0];
            root.trace.steps = vec![step_with_op(0, OpCode::STOP), step_with_op(1, OpCode::STOP)];
            root.ordering = vec![
                TraceMemberOrder::Step(0),
                TraceMemberOrder::Call(0),
                TraceMemberOrder::Step(1),
            ];
            root.children.push(1);
        }
        arena.nodes_mut().push(known_sha256_precompile_node());

        let mut flattened = Vec::new();
        flatten_call_trace(arena, &mut flattened);

        assert_no_precompile_notice(&flattened[0].steps[0]);
    }
}
