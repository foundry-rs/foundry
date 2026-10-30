use alloy_primitives::Address;
use evm2::registry::HandlerError;

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
/// Transaction validation errors keep the wording users saw before the evm2 migration.
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
    eyre::eyre!("transaction validation error: {reason}")
}
