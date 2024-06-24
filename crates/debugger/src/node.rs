use alloy_primitives::{Address, Bytes};
use foundry_evm_traces::CallKind;
use revm_inspectors::tracing::types::CallTraceStep;
use serde::{Deserialize, Serialize};

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
    /// The debug steps.
    pub steps: Vec<CallTraceStep>,
}

impl DebugNode {
    /// Creates a new debug node.
    pub fn new(
        address: Address,
        kind: CallKind,
        steps: Vec<CallTraceStep>,
        calldata: Bytes,
    ) -> Self {
        Self { address, kind, steps, calldata }
    }
}
