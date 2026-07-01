//! Conversion from geth `callTracer` output into a [`CallTraceArena`].
//!
//! This lets traces fetched over RPC (via `debug_traceCall` / `debug_traceTransaction` with the
//! `callTracer`) be decoded and rendered with the same machinery used for locally executed traces.
//! `callTracer` does not record opcode-level steps, so [`CallTrace::steps`] is left empty;
//! everything the call-tree view needs (calls, value, gas, logs, revert reasons) is preserved.

use alloy_primitives::{Bytes, LogData};
use alloy_rpc_types::trace::geth::{CallFrame, CallLogFrame};
use foundry_evm::traces::{
    CallKind, CallLog, CallTrace, CallTraceArena, CallTraceNode, TraceMemberOrder,
};
use revm::interpreter::InstructionResult;

/// Builds a [`CallTraceArena`] from a geth `callTracer` [`CallFrame`] tree.
pub fn call_frame_to_arena(root: &CallFrame) -> CallTraceArena {
    let mut arena = CallTraceArena::default();
    let nodes = arena.nodes_mut();
    nodes.clear();
    push_frame(nodes, root, None, 0);
    arena
}

/// Pushes `frame` and all of its children into `nodes`, returning the index of the pushed node.
fn push_frame(
    nodes: &mut Vec<CallTraceNode>,
    frame: &CallFrame,
    parent: Option<usize>,
    depth: usize,
) -> usize {
    let idx = nodes.len();

    let success = frame.error.is_none() && frame.revert_reason.is_none();
    let status = Some(status_from_frame(frame));

    // `callTracer` reports an unclassified halt (invalid opcode, a provider-specific quirk) only in
    // the `error` string. When the frame failed but returned no data, surface that string (or the
    // decoded `revert_reason`, preferred) as the output so the renderer shows it instead of a
    // coarse `EvmError: Revert`.
    let mut output = frame.output.clone().unwrap_or_default();
    if output.is_empty()
        && !success
        && let Some(text) = frame.revert_reason.as_deref().or(frame.error.as_deref())
    {
        output = Bytes::copy_from_slice(text.as_bytes());
    }

    let trace = CallTrace {
        depth,
        success,
        caller: frame.from,
        address: frame.to.unwrap_or_default(),
        maybe_precompile: None,
        selfdestruct_address: None,
        selfdestruct_refund_target: None,
        selfdestruct_transferred_value: None,
        kind: call_kind(&frame.typ),
        value: frame.value.unwrap_or_default(),
        data: frame.input.clone(),
        output,
        gas_used: frame.gas_used.saturating_to(),
        gas_limit: frame.gas.saturating_to(),
        gas_refund_counter: 0,
        status,
        steps: Vec::new(),
        decoded: None,
    };

    let logs = frame.logs.iter().map(call_log).collect::<Vec<_>>();

    nodes.push(CallTraceNode {
        parent,
        children: Vec::new(),
        idx,
        trace,
        logs,
        ordering: Vec::new(),
    });

    let mut children = Vec::with_capacity(frame.calls.len());
    for child in &frame.calls {
        children.push(push_frame(nodes, child, Some(idx), depth + 1));
    }

    // Reconstruct the interleaving of logs and child calls in linear time. A log's `position` is
    // the number of child calls emitted before it, so bucketing the logs by position places each
    // one after that many calls. `TraceMemberOrder::Call`/`Log` index into the node's local
    // `children`/`logs` vectors. A position past the last call is clamped to the end so the log is
    // never dropped.
    let num_calls = children.len();
    let mut logs_by_position: Vec<Vec<usize>> = vec![Vec::new(); num_calls + 1];
    for (li, log) in frame.logs.iter().enumerate() {
        let position = (log.position.unwrap_or(0) as usize).min(num_calls);
        logs_by_position[position].push(li);
    }
    let mut ordering = Vec::with_capacity(num_calls + frame.logs.len());
    for (i, logs_at_position) in logs_by_position.iter().enumerate() {
        for &li in logs_at_position {
            ordering.push(TraceMemberOrder::Log(li));
        }
        if i < num_calls {
            ordering.push(TraceMemberOrder::Call(i));
        }
    }

    nodes[idx].children = children;
    nodes[idx].ordering = ordering;
    idx
}

/// Maps a `callTracer` frame to the [`InstructionResult`] used for the rendered `[status]` label.
///
/// `callTracer` only exposes a coarse, human-readable `error` string (plus an optional
/// `revert_reason`), not a machine status code, so we recognise the two halts geth and reth report
/// reliably (an explicit revert and running out of gas) and fall back to
/// [`InstructionResult::Revert`] for anything else. `push_frame` preserves the frame's `error` /
/// `revert_reason` text in the trace output, and the call is coloured by [`CallTrace::success`], so
/// an imperfect status never hides a failure or the original error message.
fn status_from_frame(frame: &CallFrame) -> InstructionResult {
    if frame.error.is_none() && frame.revert_reason.is_none() {
        return InstructionResult::Return;
    }
    if frame.revert_reason.is_some() {
        return InstructionResult::Revert;
    }
    match frame.error.as_deref() {
        Some(err) if err.contains("out of gas") => InstructionResult::OutOfGas,
        // "execution reverted" and any other unclassified halt render as a revert.
        _ => InstructionResult::Revert,
    }
}

/// Maps a geth `callTracer` call type string to a [`CallKind`].
fn call_kind(typ: &str) -> CallKind {
    match typ {
        "STATICCALL" => CallKind::StaticCall,
        "DELEGATECALL" => CallKind::DelegateCall,
        "CALLCODE" => CallKind::CallCode,
        "AUTHCALL" => CallKind::AuthCall,
        "CREATE" => CallKind::Create,
        "CREATE2" => CallKind::Create2,
        // "CALL", "SELFDESTRUCT" and anything unknown render as a plain call.
        _ => CallKind::Call,
    }
}

/// Maps a geth `callTracer` log frame to a [`CallLog`].
fn call_log(log: &CallLogFrame) -> CallLog {
    CallLog {
        address: log.address.unwrap_or_default(),
        raw_log: LogData::new_unchecked(
            log.topics.clone().unwrap_or_default(),
            log.data.clone().unwrap_or_default(),
        ),
        decoded: None,
        position: log.position.unwrap_or_default(),
        index: log.index.unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{U256, address, b256, bytes};

    /// A nested `callTracer` frame (root CALL -> child STATICCALL) with a log on the root,
    /// mirroring a real `debug_traceCall` response, must convert into a well-formed two-node
    /// arena.
    #[test]
    fn converts_nested_call_frame() {
        let frame = CallFrame {
            from: address!("1111111111111111111111111111111111111111"),
            to: Some(address!("2222222222222222222222222222222222222222")),
            gas: U256::from(100_000u64),
            gas_used: U256::from(21_000u64),
            input: bytes!("dead"),
            output: Some(bytes!("beef")),
            value: Some(U256::from(7u64)),
            typ: "CALL".to_string(),
            logs: vec![CallLogFrame {
                address: Some(address!("2222222222222222222222222222222222222222")),
                topics: Some(vec![]),
                data: Some(bytes!("00")),
                position: Some(1),
                index: Some(0),
            }],
            calls: vec![CallFrame {
                from: address!("2222222222222222222222222222222222222222"),
                to: Some(address!("3333333333333333333333333333333333333333")),
                gas: U256::from(50_000u64),
                gas_used: U256::from(5_000u64),
                input: bytes!("cafe"),
                typ: "STATICCALL".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        };

        let arena = call_frame_to_arena(&frame);
        let nodes = arena.nodes();
        assert_eq!(nodes.len(), 2, "root + one child");

        let root = &nodes[0];
        assert_eq!(root.parent, None);
        assert_eq!(root.children, vec![1]);
        assert_eq!(root.trace.kind, CallKind::Call);
        assert_eq!(root.trace.caller, frame.from);
        assert_eq!(root.trace.value, U256::from(7u64));
        assert_eq!(root.trace.gas_used, 21_000);
        assert!(root.trace.success);
        assert_eq!(root.logs.len(), 1);

        // The log has position 1, so it must be ordered after the single child call.
        assert_eq!(root.ordering, vec![TraceMemberOrder::Call(0), TraceMemberOrder::Log(0)]);

        let child = &nodes[1];
        assert_eq!(child.parent, Some(0));
        assert_eq!(child.trace.depth, 1);
        assert_eq!(child.trace.kind, CallKind::StaticCall);
    }

    /// `callTracer` error strings must map onto the status used for the rendered `[status]` label:
    /// a clean call returns, an explicit revert and a `revert_reason` map to `Revert`, an
    /// out-of-gas halt maps to `OutOfGas`, and any other halt falls back to `Revert`.
    #[test]
    fn maps_frame_status() {
        let ok = CallFrame { typ: "CALL".to_string(), ..Default::default() };
        assert_eq!(status_from_frame(&ok), InstructionResult::Return);

        let reverted = CallFrame {
            typ: "CALL".to_string(),
            error: Some("execution reverted".to_string()),
            revert_reason: Some("boom".to_string()),
            ..Default::default()
        };
        assert_eq!(status_from_frame(&reverted), InstructionResult::Revert);

        let oog = CallFrame {
            typ: "CALL".to_string(),
            error: Some("out of gas".to_string()),
            ..Default::default()
        };
        assert_eq!(status_from_frame(&oog), InstructionResult::OutOfGas);

        let other = CallFrame {
            typ: "CALL".to_string(),
            error: Some("invalid opcode: opcode 0xfe not defined".to_string()),
            ..Default::default()
        };
        assert_eq!(status_from_frame(&other), InstructionResult::Revert);
    }

    /// An unclassified halt with no return data (e.g. an invalid opcode) must keep its original
    /// `error` string as the trace output, so the renderer surfaces it instead of a coarse
    /// `EvmError: Revert`.
    #[test]
    fn surfaces_error_string_in_output() {
        let frame = CallFrame {
            from: address!("1111111111111111111111111111111111111111"),
            to: Some(address!("2222222222222222222222222222222222222222")),
            typ: "CALL".to_string(),
            error: Some("invalid opcode: opcode 0xfe not defined".to_string()),
            ..Default::default()
        };

        let arena = call_frame_to_arena(&frame);
        let root = &arena.nodes()[0];

        assert!(!root.trace.success);
        assert_eq!(
            core::str::from_utf8(&root.trace.output[..]).unwrap(),
            "invalid opcode: opcode 0xfe not defined"
        );
    }

    /// A log whose `position` points past the last child call must be clamped to the end rather
    /// than dropped, and a `position` of zero must order the log before the first call.
    #[test]
    fn clamps_out_of_range_log_position() {
        let frame = CallFrame {
            from: address!("1111111111111111111111111111111111111111"),
            to: Some(address!("2222222222222222222222222222222222222222")),
            typ: "CALL".to_string(),
            logs: vec![
                CallLogFrame { position: Some(0), index: Some(0), ..Default::default() },
                CallLogFrame { position: Some(5), index: Some(1), ..Default::default() },
            ],
            calls: vec![CallFrame { typ: "CALL".to_string(), ..Default::default() }],
            ..Default::default()
        };

        let arena = call_frame_to_arena(&frame);
        let root = &arena.nodes()[0];

        assert_eq!(root.logs.len(), 2, "no log dropped");
        // position 0 -> before the only call; position 5 -> clamped to after it.
        assert_eq!(
            root.ordering,
            vec![TraceMemberOrder::Log(0), TraceMemberOrder::Call(0), TraceMemberOrder::Log(1),]
        );
    }

    /// A log whose `position` falls strictly between two child calls must be ordered between them.
    /// The single-child cases above only exercise before-first and after-last, so an off-by-one in
    /// the `Call(i)` / `Log(li)` indexing would otherwise go unnoticed.
    #[test]
    fn orders_log_between_two_children() {
        let frame = CallFrame {
            from: address!("1111111111111111111111111111111111111111"),
            to: Some(address!("2222222222222222222222222222222222222222")),
            typ: "CALL".to_string(),
            // position 1 -> one child emitted before the log, so it lands between the two children.
            logs: vec![CallLogFrame { position: Some(1), index: Some(0), ..Default::default() }],
            calls: vec![
                CallFrame { typ: "CALL".to_string(), ..Default::default() },
                CallFrame { typ: "CALL".to_string(), ..Default::default() },
            ],
            ..Default::default()
        };

        let arena = call_frame_to_arena(&frame);
        let root = &arena.nodes()[0];

        assert_eq!(arena.nodes().len(), 3, "root + two children");
        assert_eq!(root.children, vec![1, 2]);
        assert_eq!(
            root.ordering,
            vec![TraceMemberOrder::Call(0), TraceMemberOrder::Log(0), TraceMemberOrder::Call(1),]
        );
    }

    /// `call_kind` must map every geth `callTracer` call-type string to the right `CallKind`, and
    /// treat anything unknown as a plain call. The conversion tests only exercise
    /// `CALL`/`STATICCALL`, so a swapped or dropped arm would otherwise go unnoticed.
    #[test]
    fn maps_call_kind() {
        assert_eq!(call_kind("CALL"), CallKind::Call);
        assert_eq!(call_kind("STATICCALL"), CallKind::StaticCall);
        assert_eq!(call_kind("DELEGATECALL"), CallKind::DelegateCall);
        assert_eq!(call_kind("CALLCODE"), CallKind::CallCode);
        assert_eq!(call_kind("AUTHCALL"), CallKind::AuthCall);
        assert_eq!(call_kind("CREATE"), CallKind::Create);
        assert_eq!(call_kind("CREATE2"), CallKind::Create2);
        // "SELFDESTRUCT" and any unknown type render as a plain call.
        assert_eq!(call_kind("SELFDESTRUCT"), CallKind::Call);
        assert_eq!(call_kind("NOT_A_REAL_TYPE"), CallKind::Call);
    }

    /// `call_log` must map each `callTracer` log field to the right place. Distinct topics, data,
    /// position and index catch a swapped or dropped field (e.g. topics/data or position/index).
    #[test]
    fn maps_call_log_fields() {
        let frame_log = CallLogFrame {
            address: Some(address!("3333333333333333333333333333333333333333")),
            topics: Some(vec![
                b256!("0x00000000000000000000000000000000000000000000000000000000000000aa"),
                b256!("0x00000000000000000000000000000000000000000000000000000000000000bb"),
            ]),
            data: Some(bytes!("dead")),
            position: Some(2),
            index: Some(5),
        };

        let log = call_log(&frame_log);
        assert_eq!(log.address, address!("3333333333333333333333333333333333333333"));
        assert_eq!(
            log.raw_log.topics(),
            &[
                b256!("0x00000000000000000000000000000000000000000000000000000000000000aa"),
                b256!("0x00000000000000000000000000000000000000000000000000000000000000bb"),
            ]
        );
        assert_eq!(log.raw_log.data, bytes!("dead"));
        assert_eq!(log.position, 2);
        assert_eq!(log.index, 5);
    }
}
