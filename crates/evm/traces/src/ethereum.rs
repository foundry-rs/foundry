//! Transfer native execution traces into the current Foundry renderer.

use crate::{InstructionResult, SparsedTraceArena};
use evm2::interpreter::InstrStop;
use evm2_inspectors::tracing::TracingInspector;

#[cfg(feature = "revm")]
use crate::{
    CallKind, CallLog, CallTrace, CallTraceArena, CallTraceNode, CallTraceStep, OpCode,
    StorageChange, StorageChangeReason, TraceMemberOrder,
};
#[cfg(feature = "revm")]
use evm2_inspectors::tracing::types as native;

/// Transfers an EVM2 call tree without execution or RPC serialization.
#[cfg(feature = "revm")]
pub fn native_arena(inspector: &TracingInspector) -> SparsedTraceArena {
    // Preserve native node indices, ordering and exact halt reasons. The current renderer still
    // consumes legacy trace types; this boundary performs no execution or RPC serialization.
    let mut arena = CallTraceArena::default();
    *arena.nodes_mut() = inspector
        .traces()
        .nodes()
        .iter()
        .map(|node| {
            let trace = &node.trace;
            CallTraceNode {
                parent: node.parent,
                children: node.children.clone(),
                idx: node.idx,
                trace: CallTrace {
                    depth: trace.depth,
                    success: trace.success,
                    caller: trace.caller,
                    address: trace.address,
                    maybe_precompile: trace.maybe_precompile,
                    selfdestruct_address: trace.selfdestruct_address,
                    selfdestruct_refund_target: trace.selfdestruct_refund_target,
                    selfdestruct_transferred_value: trace.selfdestruct_transferred_value,
                    kind: match trace.kind {
                        native::CallKind::Call => CallKind::Call,
                        native::CallKind::StaticCall => CallKind::StaticCall,
                        native::CallKind::CallCode => CallKind::CallCode,
                        native::CallKind::DelegateCall => CallKind::DelegateCall,
                        native::CallKind::Create => CallKind::Create,
                        native::CallKind::Create2 => CallKind::Create2,
                        native::CallKind::AuthCall => CallKind::AuthCall,
                    },
                    value: trace.value,
                    data: trace.data.clone(),
                    output: trace.output.clone(),
                    bytecode: trace.bytecode.clone(),
                    gas_used: trace.gas_used,
                    gas_limit: trace.gas_limit,
                    gas_refund_counter: trace.gas_refund_counter,
                    status: trace.status.map(render_status),
                    steps: trace.steps.iter().map(render_step).collect(),
                    step_deltas: Vec::new(),
                    decoded: None,
                },
                logs: node
                    .logs
                    .iter()
                    .map(|log| CallLog {
                        address: log.address,
                        raw_log: log.raw_log.clone(),
                        decoded: None,
                        position: log.position,
                        index: log.index,
                    })
                    .collect(),
                ordering: node
                    .ordering
                    .iter()
                    .map(|item| match *item {
                        native::TraceMemberOrder::Call(index) => TraceMemberOrder::Call(index),
                        native::TraceMemberOrder::Log(index) => TraceMemberOrder::Log(index),
                        native::TraceMemberOrder::Step(index) => TraceMemberOrder::Step(index),
                    })
                    .collect(),
            }
        })
        .collect();
    SparsedTraceArena { arena, ignored: Default::default(), diagnostics: Default::default() }
}

#[cfg(feature = "revm")]
fn render_step(step: &native::CallTraceStep) -> CallTraceStep {
    CallTraceStep {
        pc: step.pc,
        op: OpCode::new_or_unknown(step.op.get()),
        stack: step.stack.clone(),
        push_stack: step.push_stack.clone(),
        // Memory capture and opcode debugging are enabled once the renderer uses native types.
        memory: None,
        returndata: step.returndata.clone(),
        gas_remaining: step.gas_remaining,
        gas_refund_counter: step.gas_refund_counter,
        gas_used: step.gas_used,
        gas_cost: step.gas_cost,
        state_gas_cost: step.state_gas_cost,
        state_gas_reservoir: step.state_gas_reservoir,
        state_gas_spent: step.state_gas_spent,
        storage_change: step.storage_change.as_ref().map(|change| {
            Box::new(StorageChange {
                key: change.key,
                value: change.value,
                had_value: change.had_value,
                reason: match change.reason {
                    native::StorageChangeReason::SLOAD => StorageChangeReason::SLOAD,
                    native::StorageChangeReason::SSTORE => StorageChangeReason::SSTORE,
                },
            })
        }),
        status: step.status.map(render_status),
        immediate_bytes: step.immediate_bytes.clone(),
        decoded: None,
    }
}

/// Maps an EVM2 halt reason for the current renderer.
#[cfg(feature = "revm")]
pub fn render_status(status: InstrStop) -> InstructionResult {
    match status {
        InstrStop::Stop => InstructionResult::Stop,
        InstrStop::Return => InstructionResult::Return,
        InstrStop::SelfDestruct => InstructionResult::SelfDestruct,
        InstrStop::Revert => InstructionResult::Revert,
        InstrStop::CallTooDeep => InstructionResult::CallTooDeep,
        InstrStop::OutOfFunds => InstructionResult::OutOfFunds,
        InstrStop::OutOfGas => InstructionResult::OutOfGas,
        InstrStop::MemoryOOG => InstructionResult::MemoryOOG,
        InstrStop::MemoryLimitOOG => InstructionResult::MemoryLimitOOG,
        InstrStop::PrecompileOOG => InstructionResult::PrecompileOOG,
        InstrStop::InvalidOperandOOG => InstructionResult::InvalidOperandOOG,
        InstrStop::ReentrancySentryOOG => InstructionResult::ReentrancySentryOOG,
        InstrStop::CallNotAllowedInsideStatic => InstructionResult::CallNotAllowedInsideStatic,
        InstrStop::StateChangeDuringStaticCall => InstructionResult::StateChangeDuringStaticCall,
        InstrStop::OpcodeNotFound => InstructionResult::OpcodeNotFound,
        InstrStop::InvalidFEOpcode => InstructionResult::InvalidFEOpcode,
        InstrStop::InvalidJump => InstructionResult::InvalidJump,
        InstrStop::NotActivated => InstructionResult::NotActivated,
        InstrStop::StackUnderflow => InstructionResult::StackUnderflow,
        InstrStop::StackOverflow => InstructionResult::StackOverflow,
        InstrStop::OutOfOffset => InstructionResult::OutOfOffset,
        InstrStop::CreateCollision => InstructionResult::CreateCollision,
        InstrStop::PrecompileError => InstructionResult::PrecompileError,
        InstrStop::NonceOverflow => InstructionResult::NonceOverflow,
        InstrStop::CreateContractSizeLimit => InstructionResult::CreateContractSizeLimit,
        InstrStop::CreateContractStartingWithEF => InstructionResult::CreateContractStartingWithEF,
        InstrStop::CreateInitCodeSizeLimit => InstructionResult::CreateInitCodeSizeLimit,
        InstrStop::FatalExternalError => InstructionResult::FatalExternalError,
        InstrStop::InvalidImmediateEncoding => InstructionResult::InvalidImmediateEncoding,
        _ => unreachable!("unmapped EVM2 instruction status: {status:?}"),
    }
}

/// Retains the native call tree for the native renderer.
#[cfg(not(feature = "revm"))]
pub fn native_arena(inspector: &TracingInspector) -> SparsedTraceArena {
    SparsedTraceArena {
        arena: inspector.traces().clone(),
        ignored: Default::default(),
        diagnostics: Default::default(),
    }
}

/// Retains the native execution halt reason.
#[cfg(not(feature = "revm"))]
pub const fn render_status(status: InstrStop) -> InstructionResult {
    status
}

#[cfg(all(test, not(feature = "revm")))]
mod tests {
    use super::*;
    use crate::{CallTraceStep, OpCode};

    #[test]
    fn native_arena_preserves_memory_and_halt_reason() {
        let mut inspector = TracingInspector::default();
        let trace = &mut inspector.traces_mut().nodes_mut()[0].trace;
        trace.status = Some(InstrStop::InvalidJump);
        trace.steps.push(CallTraceStep {
            op: OpCode::MSTORE,
            memory: Some(serde_json::from_value(serde_json::json!("0x010203")).unwrap()),
            status: Some(InstrStop::OutOfGas),
            pc: 0,
            stack: None,
            push_stack: None,
            returndata: Default::default(),
            gas_remaining: 100,
            gas_refund_counter: 0,
            gas_used: 3,
            gas_cost: 3,
            state_gas_cost: None,
            state_gas_reservoir: None,
            state_gas_spent: 0,
            storage_change: None,
            immediate_bytes: None,
            decoded: None,
        });
        let arena = native_arena(&inspector);
        assert_eq!(
            serde_json::to_value(&arena.arena).unwrap(),
            serde_json::to_value(inspector.traces()).unwrap(),
        );
    }
}
