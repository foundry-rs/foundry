//! Builder style functions for `trace_call`

use crate::parity::TraceType;
use alloy_primitives::map::HashSet;
use alloy_rpc_types_eth::{
    request::TransactionRequest, state::StateOverride, BlockId, BlockOverrides,
};
use serde::{Deserialize, Serialize};

/// Container type for `trace_call` arguments
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct TraceCallRequest {
    /// call request object
    pub call: TransactionRequest,
    /// trace types
    pub trace_types: HashSet<TraceType>,
    /// Optional: blockId
    pub block_id: Option<BlockId>,
    /// Optional: StateOverride
    pub state_overrides: Option<StateOverride>,
    /// Optional: BlockOverrides
    pub block_overrides: Option<Box<BlockOverrides>>,
}

impl TraceCallRequest {
    /// Returns a new [`TraceCallRequest`] given a [`TransactionRequest`] and [`HashSet<TraceType>`]
    pub fn new(call: TransactionRequest) -> Self {
        Self {
            call,
            trace_types: HashSet::default(),
            block_id: None,
            state_overrides: None,
            block_overrides: None,
        }
    }

    /// Sets the [`BlockId`]
    /// Note: this is optional
    pub const fn with_block_id(mut self, block_id: BlockId) -> Self {
        self.block_id = Some(block_id);
        self
    }

    /// Sets the [`StateOverride`]
    /// Note: this is optional
    pub fn with_state_override(mut self, state_overrides: StateOverride) -> Self {
        self.state_overrides = Some(state_overrides);
        self
    }

    /// Sets the [`BlockOverrides`]
    /// Note: this is optional
    pub fn with_block_overrides(mut self, block_overrides: Box<BlockOverrides>) -> Self {
        self.block_overrides = Some(block_overrides);
        self
    }

    /// Inserts a single trace type.
    pub fn with_trace_type(mut self, trace_type: TraceType) -> Self {
        self.trace_types.insert(trace_type);
        self
    }

    /// Inserts multiple trace types from an iterator.
    pub fn with_trace_types<I: IntoIterator<Item = TraceType>>(mut self, trace_types: I) -> Self {
        self.trace_types.extend(trace_types);
        self
    }

    /// Inserts [`TraceType::Trace`]
    pub fn with_trace(self) -> Self {
        self.with_trace_type(TraceType::Trace)
    }

    /// Inserts [`TraceType::VmTrace`]
    pub fn with_vm_trace(self) -> Self {
        self.with_trace_type(TraceType::VmTrace)
    }

    /// Inserts [`TraceType::StateDiff`]
    pub fn with_statediff(self) -> Self {
        self.with_trace_type(TraceType::StateDiff)
    }
}
