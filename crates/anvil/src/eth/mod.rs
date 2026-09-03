use alloy_rpc_types::{
    TransactionRequest,
    simulate::{SimBlock, SimulatePayload},
};
use alloy_serde::WithOtherFields;

pub mod api;
pub mod backend;
pub mod error;
pub mod fees;
pub(crate) mod macros;
pub mod miner;
pub mod otterscan;
pub mod pool;
pub mod sign;
pub mod util;

pub use api::EthApi;

pub(crate) fn preserve_simulation_request_fields(
    request: SimulatePayload,
) -> SimulatePayload<WithOtherFields<TransactionRequest>> {
    SimulatePayload {
        block_state_calls: request
            .block_state_calls
            .into_iter()
            .map(|block| SimBlock {
                block_overrides: block.block_overrides,
                state_overrides: block.state_overrides,
                calls: block.calls.into_iter().map(WithOtherFields::new).collect(),
            })
            .collect(),
        trace_transfers: request.trace_transfers,
        validation: request.validation,
        return_full_transactions: request.return_full_transactions,
    }
}
