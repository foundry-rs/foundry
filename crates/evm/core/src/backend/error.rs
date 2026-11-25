use alloy_primitives::Address;
use evm2::registry::HandlerError;
use foundry_common::provider::{RpcFailure, rpc_failure};

pub use foundry_fork_db::{DatabaseError, DatabaseResult};

pub type BackendResult<T> = Result<T, BackendError>;

/// Errors of [`Backend`](super::Backend) operations.
#[derive(Debug, thiserror::Error)]
#[expect(missing_docs)]
pub enum BackendError {
    #[error("cheatcodes are not enabled for {0}; see `vm.allowCheatcodes(address)`")]
    NoCheats(Address),
    #[error(transparent)]
    Database(#[from] DatabaseError),
    #[error(transparent)]
    State(#[from] evm2::DatabaseError),
}

/// Returns a report for a transaction evm2 rejected or failed to execute, wrapped in an
/// "EVM error" context.
pub fn handler_error(err: HandlerError) -> eyre::Report {
    transaction_error(err).wrap_err("EVM error")
}

/// Returns a report for a transaction evm2 rejected or failed to execute.
///
/// Transaction validation errors use Foundry's own wording instead of evm2's.
pub fn transaction_error(err: HandlerError) -> eyre::Report {
    let reason = match &err {
        HandlerError::InvalidNonce { expected, got } if got > expected => {
            format!("nonce {got} too high, expected {expected}")
        }
        HandlerError::InvalidNonce { expected, got } => {
            format!("nonce {got} too low, expected {expected}")
        }
        HandlerError::InvalidChainId { .. } => "invalid chain ID".to_string(),
        HandlerError::MissingChainId => "missing chain ID".to_string(),
        HandlerError::IntrinsicGasTooLow { required, got } => {
            format!("call gas cost ({required}) exceeds the gas limit ({got})")
        }
        HandlerError::RejectCallerWithCode => {
            "reject transactions from senders with deployed code".to_string()
        }
        HandlerError::GasLimitMoreThanBlock { .. } => {
            "caller gas limit exceeds the block gas limit".to_string()
        }
        HandlerError::TxGasLimitGreaterThanCap { gas_limit, cap } => {
            format!("transaction gas limit ({gas_limit}) is greater than the cap ({cap})")
        }
        HandlerError::CreateInitCodeSizeLimit { .. } => "create initcode size limit".to_string(),
        HandlerError::FeeCapLessThanBaseFee { .. } => "gas price is less than basefee".to_string(),
        HandlerError::EmptyAuthorizationList => "empty authorization list".to_string(),
        HandlerError::BlobFeeCapLessThanBlobBaseFee { max_fee_per_blob_gas, blob_base_fee } => {
            format!(
                "blob gas price ({blob_base_fee}) is greater than max fee per blob gas \
                 ({max_fee_per_blob_gas})"
            )
        }
        HandlerError::TooManyBlobs { have, max } => {
            format!("too many blobs, have {have}, max {max}")
        }
        HandlerError::PriorityFeeGreaterThanMaxFee => {
            "priority fee is greater than max fee".to_string()
        }
        HandlerError::InsufficientFunds { .. }
        | HandlerError::NonceOverflow
        | HandlerError::EmptyBlobs
        | HandlerError::BlobVersionNotSupported => err.to_string(),
        _ => return eyre::Report::new(err),
    };
    eyre::Report::new(TransactionValidationError { reason, error: err })
}

/// A transaction evm2 rejected, with Foundry's wording of the reason. The evm2 error stays
/// available to callers that downcast the report.
#[derive(Debug, thiserror::Error)]
#[error("transaction validation error: {reason}")]
pub struct TransactionValidationError {
    /// Foundry's wording of the rejection.
    pub reason: String,
    /// The evm2 rejection.
    pub error: HandlerError,
}

/// A fork's endpoint has no block at a requested number, such as the one the fork starts from on a
/// load-balanced endpoint whose backend has not seen that block yet.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct ForkBlockNotFound(pub String);

/// Returns how an RPC request in `error`'s chain failed, including requests a fork's database
/// made during execution.
pub fn rpc_failure_in(error: &(dyn std::error::Error + 'static)) -> Option<RpcFailure> {
    for error in std::iter::successors(Some(error), |error| error.source()) {
        if error.is::<ForkBlockNotFound>() {
            return Some(RpcFailure { answered: true, ..Default::default() });
        }
        // These wrappers hide the fork database error from the source chain.
        let state = error
            .downcast_ref::<evm2::DatabaseError>()
            .or_else(|| match error.downcast_ref() {
                Some(BackendError::State(error)) => Some(error),
                _ => None,
            })
            .or_else(|| match error.downcast_ref() {
                Some(evm2::ExecutionError::Database(error)) => Some(error),
                _ => None,
            });
        let database = state
            .and_then(|error| error.downcast_ref::<DatabaseError>())
            .or_else(|| error.downcast_ref::<DatabaseError>())
            .or_else(|| match error.downcast_ref() {
                Some(BackendError::Database(error)) => Some(error),
                _ => None,
            });
        if let Some(database) = database {
            return match database {
                DatabaseError::GetAccount(.., error)
                | DatabaseError::GetStorage(.., error)
                | DatabaseError::GetFullBlock(_, error)
                | DatabaseError::AnyRequest(error) => {
                    Some(rpc_failure((**error).as_ref()).unwrap_or_default())
                }
                // Without a transport error, the endpoint answered, such as with no transaction or
                // no ancestor block.
                DatabaseError::GetTransaction(_, error) | DatabaseError::GetBlockHash(_, error) => {
                    Some(
                        rpc_failure((**error).as_ref())
                            .unwrap_or(RpcFailure { answered: true, ..Default::default() }),
                    )
                }
                DatabaseError::BlockNotFound(_) => {
                    Some(RpcFailure { answered: true, ..Default::default() })
                }
                DatabaseError::MissingCode(_) | DatabaseError::Recv(_) | DatabaseError::Send(_) => {
                    None
                }
            };
        }
    }
    rpc_failure(error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn missing_ancestor_block_is_answered() {
        let error =
            DatabaseError::GetBlockHash(1, Arc::new(eyre::eyre!("anchored block 1 not found")));
        assert_eq!(rpc_failure_in(&error).unwrap().kind(), "rpc.error");
    }
}
