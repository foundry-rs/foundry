//! Aggregated error type for this module

use crate::eth::pool::transactions::PoolTransaction;
use anvil_rpc::{
    error::{ErrorCode, RpcError},
    response::ResponseResult,
};
use ethers::{
    abi::AbiDecode,
    providers::ProviderError,
    signers::WalletError,
    types::{Bytes, SignatureError, U256},
};
use foundry_evm::revm::Return;
use serde::Serialize;
use tracing::error;

pub(crate) type Result<T> = std::result::Result<T, BlockchainError>;

#[derive(thiserror::Error, Debug)]
pub enum BlockchainError {
    #[error(transparent)]
    Pool(#[from] PoolError),
    #[error("No signer available")]
    NoSignerAvailable,
    #[error("Chain Id not available")]
    ChainIdNotAvailable,
    #[error("Invalid input: `max_priority_fee_per_gas` greater than `max_fee_per_gas`")]
    InvalidFeeInput,
    #[error("Transaction data is empty")]
    EmptyRawTransactionData,
    #[error("Failed to decode signed transaction")]
    FailedToDecodeSignedTransaction,
    #[error("Failed to decode transaction")]
    FailedToDecodeTransaction,
    #[error(transparent)]
    SignatureError(#[from] SignatureError),
    #[error(transparent)]
    WalletError(#[from] WalletError),
    #[error("Rpc Endpoint not implemented")]
    RpcUnimplemented,
    #[error("Rpc error {0:?}")]
    RpcError(RpcError),
    #[error(transparent)]
    InvalidTransaction(#[from] InvalidTransactionError),
    #[error(transparent)]
    FeeHistory(#[from] FeeHistoryError),
    #[error(transparent)]
    ForkProvider(#[from] ProviderError),
    #[error("EVM error {0:?}")]
    EvmError(Return),
    #[error("Invalid url {0:?}")]
    InvalidUrl(String),
    #[error("Internal error: {0:?}")]
    Internal(String),
    #[error("BlockOutOfRangeError: block height is {0} but requested was {1}")]
    BlockOutOfRange(u64, u64),
    #[error("Resource not found")]
    BlockNotFound,
}

impl From<RpcError> for BlockchainError {
    fn from(err: RpcError) -> Self {
        BlockchainError::RpcError(err)
    }
}

/// Errors that can occur in the transaction pool
#[derive(thiserror::Error, Debug)]
pub enum PoolError {
    #[error("Transaction with cyclic dependent transactions")]
    CyclicTransaction,
    /// Thrown if a replacement transaction's gas price is below the already imported transaction
    #[error("Tx: [{0:?}] insufficient gas price to replace existing transaction")]
    ReplacementUnderpriced(Box<PoolTransaction>),
    #[error("Tx: [{0:?}] already Imported")]
    AlreadyImported(Box<PoolTransaction>),
}

/// Errors that can occur with `eth_feeHistory`
#[derive(thiserror::Error, Debug)]
pub enum FeeHistoryError {
    #[error("Requested block range is out of bounds")]
    InvalidBlockRange,
}

/// An error due to invalid transaction
#[derive(thiserror::Error, Debug)]
pub enum InvalidTransactionError {
    /// Represents the inability to cover max cost + value (account balance too low).
    #[error("Insufficient funds for gas * price + value")]
    Payment,
    /// General error when transaction is outdated, nonce too low
    #[error("Transaction is outdated")]
    Outdated,
    /// returned if the nonce of a transaction is higher than the next one expected based on the
    /// local chain.
    #[error("Nonce too high")]
    NonceTooHigh,
    /// returned if the nonce of a transaction is lower than the one present in the local chain.
    #[error("nonce too low")]
    NonceTooLow,
    /// Returned if the nonce of a transaction is too high
    #[error("nonce has max value")]
    NonceMax,
    /// returned if the transaction gas exceeds the limit
    #[error("intrinsic gas too high")]
    GasTooHigh,
    /// returned if the transaction is specified to use less gas than required to start the
    /// invocation.
    #[error("intrinsic gas too low")]
    GasTooLow,

    #[error("execution reverted: {0:?}")]
    Revert(Option<Bytes>),
    /// The transaction would exhaust gas resources of current block.
    ///
    /// But transaction is still valid.
    #[error("Insufficient funds for gas * price + value")]
    ExhaustsGasResources,
    #[error("Out of gas: required gas exceeds allowance: {0:?}")]
    OutOfGas(U256),

    /// Thrown post London if the transaction's fee is less than the base fee of the block
    #[error("max fee per gas less than block base fee")]
    FeeTooLow,
}

/// Returns the revert reason from the `revm::TransactOut` data, if it's an abi encoded String.
///
/// **Note:** it's assumed the `out` buffer starts with the call's signature
fn decode_revert_reason(out: impl AsRef<[u8]>) -> Option<String> {
    let out = out.as_ref();
    if out.len() < 4 {
        return None
    }
    String::decode(&out[4..]).ok()
}

/// Helper trait to easily convert results to rpc results
pub(crate) trait ToRpcResponseResult {
    fn to_rpc_result(self) -> ResponseResult;
}

/// Converts a serializable value into a `ResponseResult`
pub fn to_rpc_result<T: Serialize>(val: T) -> ResponseResult {
    match serde_json::to_value(val) {
        Ok(success) => ResponseResult::Success(success),
        Err(err) => {
            error!("Failed serialize rpc response: {:?}", err);
            ResponseResult::error(RpcError::internal_error())
        }
    }
}

impl<T: Serialize> ToRpcResponseResult for Result<T> {
    fn to_rpc_result(self) -> ResponseResult {
        match self {
            Ok(val) => to_rpc_result(val),
            Err(err) => match err {
                BlockchainError::Pool(err) => {
                    error!("txpool error: {:?}", err);
                    match err {
                        PoolError::CyclicTransaction => {
                            RpcError::transaction_rejected("Cyclic transaction detected")
                        }
                        PoolError::ReplacementUnderpriced(_) => {
                            RpcError::transaction_rejected("replacement transaction underpriced")
                        }
                        PoolError::AlreadyImported(_) => {
                            RpcError::transaction_rejected("transaction already imported")
                        }
                    }
                }
                BlockchainError::NoSignerAvailable => {
                    RpcError::invalid_params("No Signer available")
                }
                BlockchainError::ChainIdNotAvailable => {
                    RpcError::invalid_params("Chain Id not available")
                }
                BlockchainError::InvalidTransaction(err) => match err {
                    InvalidTransactionError::Revert(data) => {
                        // this mimics geth revert error
                        let mut msg = "execution reverted".to_string();
                        if let Some(reason) = data.as_ref().and_then(decode_revert_reason) {
                            msg = format!("{}: {}", msg, reason);
                        }
                        RpcError {
                            // geth returns this error code on reverts, See <https://github.com/ethereum/wiki/wiki/JSON-RPC-Error-Codes-Improvement-Proposal>
                            code: ErrorCode::ExecutionError,
                            message: msg.into(),
                            data: serde_json::to_value(data).ok(),
                        }
                    }
                    _ => RpcError::transaction_rejected(err.to_string()),
                },
                BlockchainError::FeeHistory(err) => RpcError::invalid_params(err.to_string()),
                BlockchainError::EmptyRawTransactionData => {
                    RpcError::invalid_params("Empty transaction data")
                }
                BlockchainError::FailedToDecodeSignedTransaction => {
                    RpcError::invalid_params("Failed to decode transaction")
                }
                BlockchainError::FailedToDecodeTransaction => {
                    RpcError::invalid_params("Failed to decode transaction")
                }
                BlockchainError::SignatureError(err) => RpcError::invalid_params(err.to_string()),
                BlockchainError::WalletError(err) => RpcError::invalid_params(err.to_string()),
                BlockchainError::RpcUnimplemented => {
                    RpcError::internal_error_with("Not implemented")
                }
                BlockchainError::RpcError(err) => err,
                BlockchainError::InvalidFeeInput => RpcError::invalid_params(
                    "Invalid input: `max_priority_fee_per_gas` greater than `max_fee_per_gas`",
                ),
                BlockchainError::ForkProvider(err) => {
                    error!("fork provider error: {:?}", err);
                    RpcError::internal_error_with(format!("Fork Error: {:?}", err))
                }
                err @ BlockchainError::EvmError(_) => {
                    RpcError::internal_error_with(err.to_string())
                }
                err @ BlockchainError::InvalidUrl(_) => RpcError::invalid_params(err.to_string()),
                BlockchainError::Internal(err) => RpcError::internal_error_with(err),
                err @ BlockchainError::BlockOutOfRange(_, _) => {
                    RpcError::invalid_params(err.to_string())
                }
                err @ BlockchainError::BlockNotFound => RpcError {
                    // <https://eips.ethereum.org/EIPS/eip-1898>
                    code: ErrorCode::ServerError(-32001),
                    message: err.to_string().into(),
                    data: None,
                },
            }
            .into(),
        }
    }
}
