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
use forge::revm::{self, primitives::EVMError};
use foundry_common::SELECTOR_LEN;
use foundry_evm::{executor::backend::DatabaseError, revm::interpreter::InstructionResult};
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
    #[error("Failed to decode state")]
    FailedToDecodeStateDump,
    #[error("Prevrandao not in th EVM's environment after merge")]
    PrevrandaoNotSet,
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
    EvmError(InstructionResult),
    #[error("Invalid url {0:?}")]
    InvalidUrl(String),
    #[error("Internal error: {0:?}")]
    Internal(String),
    #[error("BlockOutOfRangeError: block height is {0} but requested was {1}")]
    BlockOutOfRange(u64, u64),
    #[error("Resource not found")]
    BlockNotFound,
    #[error("Required data unavailable")]
    DataUnavailable,
    #[error("Trie error: {0}")]
    TrieError(String),
    #[error("{0}")]
    UintConversion(&'static str),
    #[error("State override error: {0}")]
    StateOverrideError(String),
    #[error("Timestamp error: {0}")]
    TimestampError(String),
    #[error(transparent)]
    DatabaseError(#[from] DatabaseError),
    #[error("EIP-1559 style fee params (maxFeePerGas or maxPriorityFeePerGas) received but they are not supported by the current hardfork.\n\nYou can use them by running anvil with '--hardfork london' or later.")]
    EIP1559TransactionUnsupportedAtHardfork,
    #[error("Access list received but is not supported by the current hardfork.\n\nYou can use it by running anvil with '--hardfork berlin' or later.")]
    EIP2930TransactionUnsupportedAtHardfork,
}

impl From<RpcError> for BlockchainError {
    fn from(err: RpcError) -> Self {
        BlockchainError::RpcError(err)
    }
}

impl<T> From<EVMError<T>> for BlockchainError
where
    T: Into<BlockchainError>,
{
    fn from(err: EVMError<T>) -> Self {
        match err {
            EVMError::Transaction(err) => InvalidTransactionError::from(err).into(),
            EVMError::PrevrandaoNotSet => BlockchainError::PrevrandaoNotSet,
            EVMError::Database(err) => err.into(),
        }
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
    /// returned if the nonce of a transaction is lower than the one present in the local chain.
    #[error("nonce too low")]
    NonceTooLow,
    /// returned if the nonce of a transaction is higher than the next one expected based on the
    /// local chain.
    #[error("Nonce too high")]
    NonceTooHigh,
    /// Returned if the nonce of a transaction is too high
    /// Incrementing the nonce would lead to invalid state (overflow)
    #[error("nonce has max value")]
    NonceMaxValue,
    /// thrown if the transaction sender doesn't have enough funds for a transfer
    #[error("insufficient funds for transfer")]
    InsufficientFundsForTransfer,
    /// thrown if creation transaction provides the init code bigger than init code size limit.
    #[error("max initcode size exceeded")]
    MaxInitCodeSizeExceeded,
    /// Represents the inability to cover max cost + value (account balance too low).
    #[error("Insufficient funds for gas * price + value")]
    InsufficientFunds,
    /// Thrown when calculating gas usage
    #[error("gas uint64 overflow")]
    GasUintOverflow,
    /// returned if the transaction is specified to use less gas than required to start the
    /// invocation.
    #[error("intrinsic gas too low")]
    GasTooLow,
    /// returned if the transaction gas exceeds the limit
    #[error("intrinsic gas too high")]
    GasTooHigh,
    /// Thrown to ensure no one is able to specify a transaction with a tip higher than the total
    /// fee cap.
    #[error("max priority fee per gas higher than max fee per gas")]
    TipAboveFeeCap,
    /// Thrown post London if the transaction's fee is less than the base fee of the block
    #[error("max fee per gas less than block base fee")]
    FeeCapTooLow,
    /// Thrown during estimate if caller has insufficient funds to cover the tx.
    #[error("Out of gas: gas required exceeds allowance: {0:?}")]
    BasicOutOfGas(U256),
    /// Thrown if executing a transaction failed during estimate/call
    #[error("execution reverted: {0:?}")]
    Revert(Option<Bytes>),
    /// Thrown if the sender of a transaction is a contract.
    #[error("sender not an eoa")]
    SenderNoEOA,
    /// Thrown when a tx was signed with a different chain_id
    #[error("invalid chain id for signer")]
    InvalidChainId,
    /// Thrown when a legacy tx was signed for a different chain
    #[error("Incompatible EIP-155 transaction, signed for another chain")]
    IncompatibleEIP155,
}

impl From<revm::primitives::InvalidTransaction> for InvalidTransactionError {
    fn from(err: revm::primitives::InvalidTransaction) -> Self {
        use revm::primitives::InvalidTransaction;
        match err {
            InvalidTransaction::InvalidChainId => InvalidTransactionError::InvalidChainId,
            InvalidTransaction::GasMaxFeeGreaterThanPriorityFee => {
                InvalidTransactionError::TipAboveFeeCap
            }
            InvalidTransaction::GasPriceLessThanBasefee => InvalidTransactionError::FeeCapTooLow,
            InvalidTransaction::CallerGasLimitMoreThanBlock => InvalidTransactionError::GasTooHigh,
            InvalidTransaction::CallGasCostMoreThanGasLimit => InvalidTransactionError::GasTooHigh,
            InvalidTransaction::RejectCallerWithCode => InvalidTransactionError::SenderNoEOA,
            InvalidTransaction::LackOfFundForGasLimit { .. } => {
                InvalidTransactionError::InsufficientFunds
            }
            InvalidTransaction::OverflowPaymentInTransaction => {
                InvalidTransactionError::GasUintOverflow
            }
            InvalidTransaction::NonceOverflowInTransaction => {
                InvalidTransactionError::NonceMaxValue
            }
            InvalidTransaction::CreateInitcodeSizeLimit => {
                InvalidTransactionError::MaxInitCodeSizeExceeded
            }
            InvalidTransaction::NonceTooHigh { .. } => InvalidTransactionError::NonceTooHigh,
            InvalidTransaction::NonceTooLow { .. } => InvalidTransactionError::NonceTooLow,
        }
    }
}

/// Returns the revert reason from the `revm::TransactOut` data, if it's an abi encoded String.
///
/// **Note:** it's assumed the `out` buffer starts with the call's signature
pub(crate) fn decode_revert_reason(out: impl AsRef<[u8]>) -> Option<String> {
    let out = out.as_ref();
    if out.len() < SELECTOR_LEN {
        return None
    }
    String::decode(&out[SELECTOR_LEN..]).ok()
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
                            msg = format!("{msg}: {reason}");
                        }
                        RpcError {
                            // geth returns this error code on reverts, See <https://github.com/ethereum/wiki/wiki/JSON-RPC-Error-Codes-Improvement-Proposal>
                            code: ErrorCode::ExecutionError,
                            message: msg.into(),
                            data: serde_json::to_value(data).ok(),
                        }
                    }
                    InvalidTransactionError::GasTooLow | InvalidTransactionError::GasTooHigh => {
                        // <https://eips.ethereum.org/EIPS/eip-1898>
                        RpcError {
                            code: ErrorCode::ServerError(-32000),
                            message: err.to_string().into(),
                            data: None,
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
                BlockchainError::FailedToDecodeStateDump => {
                    RpcError::invalid_params("Failed to decode state dump")
                }
                BlockchainError::SignatureError(err) => RpcError::invalid_params(err.to_string()),
                BlockchainError::WalletError(err) => RpcError::invalid_params(err.to_string()),
                BlockchainError::RpcUnimplemented => {
                    RpcError::internal_error_with("Not implemented")
                }
                BlockchainError::PrevrandaoNotSet => RpcError::internal_error_with(err.to_string()),
                BlockchainError::RpcError(err) => err,
                BlockchainError::InvalidFeeInput => RpcError::invalid_params(
                    "Invalid input: `max_priority_fee_per_gas` greater than `max_fee_per_gas`",
                ),
                BlockchainError::ForkProvider(err) => {
                    error!("fork provider error: {:?}", err);
                    RpcError::internal_error_with(format!("Fork Error: {err:?}"))
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
                err @ BlockchainError::DataUnavailable => {
                    RpcError::internal_error_with(err.to_string())
                }
                err @ BlockchainError::TrieError(_) => {
                    RpcError::internal_error_with(err.to_string())
                }
                BlockchainError::UintConversion(err) => RpcError::invalid_params(err),
                err @ BlockchainError::StateOverrideError(_) => {
                    RpcError::invalid_params(err.to_string())
                }
                err @ BlockchainError::TimestampError(_) => {
                    RpcError::invalid_params(err.to_string())
                }
                BlockchainError::DatabaseError(err) => {
                    RpcError::internal_error_with(err.to_string())
                }
                err @ BlockchainError::EIP1559TransactionUnsupportedAtHardfork => {
                    RpcError::invalid_params(err.to_string())
                }
                err @ BlockchainError::EIP2930TransactionUnsupportedAtHardfork => {
                    RpcError::invalid_params(err.to_string())
                }
            }
            .into(),
        }
    }
}
