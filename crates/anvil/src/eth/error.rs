//! Aggregated error type for this module

use crate::eth::pool::transactions::PoolTransaction;
use alloy_primitives::{Bytes, SignatureError};
use alloy_rpc_types::BlockNumberOrTag;
use alloy_signer::Error as SignerError;
use alloy_transport::TransportError;
use anvil_core::eth::wallet::WalletError;
use anvil_rpc::{
    error::{ErrorCode, RpcError},
    response::ResponseResult,
};
use foundry_evm::{
    backend::DatabaseError,
    decode::RevertDecoder,
    revm::{
        interpreter::InstructionResult,
        primitives::{EVMError, InvalidHeader},
    },
};
use serde::Serialize;

pub(crate) type Result<T> = std::result::Result<T, BlockchainError>;

#[derive(Debug, thiserror::Error)]
pub enum BlockchainError {
    #[error(transparent)]
    Pool(#[from] PoolError),
    #[error("No signer available. Make sure you have configured accounts properly in the node.")]
    NoSignerAvailable,
    #[error("Chain ID not available. Make sure you have properly configured the network.")]
    ChainIdNotAvailable,
    #[error("Invalid transaction fee parameters: `max_priority_fee_per_gas` ({0}) is greater than `max_fee_per_gas` ({1}). The priority fee must be less than or equal to the max fee.")]
    InvalidFeeInput(u128, u128),
    #[error("Transaction data is empty. Ensure your transaction contains valid data.")]
    EmptyRawTransactionData,
    #[error("Failed to decode signed transaction. The transaction format might be invalid or corrupted.")]
    FailedToDecodeSignedTransaction,
    #[error("Failed to decode transaction. The transaction format might be invalid or corrupted.")]
    FailedToDecodeTransaction,
    #[error("Failed to decode receipt. The receipt format might be invalid or corrupted.")]
    FailedToDecodeReceipt,
    #[error("Failed to decode state. The state dump format might be invalid or corrupted.")]
    FailedToDecodeStateDump,
    #[error("Prevrandao not set in the EVM environment after merge. This is an internal issue with the post-merge configuration.")]
    PrevrandaoNotSet,
    #[error(transparent)]
    SignatureError(#[from] SignatureError),
    #[error(transparent)]
    SignerError(#[from] SignerError),
    #[error("RPC Endpoint not implemented. This feature is not available in the current version.")]
    RpcUnimplemented,
    #[error("RPC error {0:?}")]
    RpcError(RpcError),
    #[error(transparent)]
    InvalidTransaction(#[from] InvalidTransactionError),
    #[error(transparent)]
    FeeHistory(#[from] FeeHistoryError),
    #[error(transparent)]
    AlloyForkProvider(#[from] TransportError),
    #[error("EVM execution error: {0:?}. Check your transaction parameters and contract code.")]
    EvmError(InstructionResult),
    #[error("Invalid URL format: '{0}'. Please provide a valid URL.")]
    InvalidUrl(String),
    #[error("Internal error: {0:?}. Please report this issue to the Anvil maintainers.")]
    Internal(String),
    #[error("Block out of range error: current block height is {0} but requested block was {1}. Make sure you're requesting an existing block.")]
    BlockOutOfRange(u64, u64),
    #[error("Block not found. The requested block does not exist or has not been processed yet.")]
    BlockNotFound,
    #[error("Required data unavailable. The requested information might be pruned or not synced yet.")]
    DataUnavailable,
    #[error("Trie error: {0}. This is likely an issue with the internal state storage.")]
    TrieError(String),
    #[error("{0}")]
    UintConversion(&'static str),
    #[error("State override error: {0}. Check your state override parameters.")]
    StateOverrideError(String),
    #[error("Timestamp error: {0}. Make sure the timestamp is valid and within acceptable bounds.")]
    TimestampError(String),
    #[error(transparent)]
    DatabaseError(#[from] DatabaseError),
    #[error("EIP-1559 style fee params (maxFeePerGas or maxPriorityFeePerGas) received but they are not supported by the current hardfork.\n\nYou can use them by running anvil with '--hardfork london' or later, e.g.: anvil --hardfork london")]
    EIP1559TransactionUnsupportedAtHardfork,
    #[error("Access list received but is not supported by the current hardfork.\n\nYou can use it by running anvil with '--hardfork berlin' or later, e.g.: anvil --hardfork berlin")]
    EIP2930TransactionUnsupportedAtHardfork,
    #[error("EIP-4844 blob fields received but they are not supported by the current hardfork.\n\nYou can use them by running anvil with '--hardfork cancun' or later, e.g.: anvil --hardfork cancun")]
    EIP4844TransactionUnsupportedAtHardfork,
    #[error("EIP-7702 fields received but they are not supported by the current hardfork.\n\nYou can use them by running anvil with '--hardfork prague' or later, e.g.: anvil --hardfork prague")]
    EIP7702TransactionUnsupportedAtHardfork,
    #[error("Optimism deposit transaction received but optimism mode is not enabled.\n\nYou can enable it by running anvil with '--optimism', e.g.: anvil --optimism")]
    DepositTransactionUnsupported,
    #[error("Unknown transaction type not supported. Make sure you're using a transaction type supported by the current network configuration.")]
    UnknownTransactionType,
    #[error("Excess blob gas not set. This is required for EIP-4844 transactions on Cancun hardfork.")]
    ExcessBlobGasNotSet,
    #[error("{0}")]
    Message(String),
}

impl From<eyre::Report> for BlockchainError {
    fn from(err: eyre::Report) -> Self {
        Self::Message(err.to_string())
    }
}

impl From<RpcError> for BlockchainError {
    fn from(err: RpcError) -> Self {
        Self::RpcError(err)
    }
}

impl<T> From<EVMError<T>> for BlockchainError
where
    T: Into<Self>,
{
    fn from(err: EVMError<T>) -> Self {
        match err {
            EVMError::Transaction(err) => InvalidTransactionError::from(err).into(),
            EVMError::Header(err) => match err {
                InvalidHeader::ExcessBlobGasNotSet => Self::ExcessBlobGasNotSet,
                InvalidHeader::PrevrandaoNotSet => Self::PrevrandaoNotSet,
            },
            EVMError::Database(err) => err.into(),
            EVMError::Precompile(err) => Self::Message(err),
            EVMError::Custom(err) => Self::Message(err),
        }
    }
}

impl From<WalletError> for BlockchainError {
    fn from(value: WalletError) -> Self {
        match value {
            WalletError::ValueNotZero => Self::Message("tx value not zero".to_string()),
            WalletError::FromSet => Self::Message("tx from field is set".to_string()),
            WalletError::NonceSet => Self::Message("tx nonce is set".to_string()),
            WalletError::InvalidAuthorization => {
                Self::Message("invalid authorization address".to_string())
            }
            WalletError::IllegalDestination => Self::Message(
                "the destination of the transaction is not a delegated account".to_string(),
            ),
            WalletError::InternalError => Self::Message("internal error".to_string()),
            WalletError::InvalidTransactionRequest => {
                Self::Message("invalid tx request".to_string())
            }
        }
    }
}

/// Errors that can occur in the transaction pool
#[derive(Debug, thiserror::Error)]
pub enum PoolError {
    #[error("Transaction with cyclic dependent transactions detected. Ensure your transactions don't have circular dependencies.")]
    CyclicTransaction,
    /// Thrown if a replacement transaction's gas price is below the already imported transaction
    #[error("Transaction {0:?} has insufficient gas price to replace existing transaction. Increase the gas price to replace the pending transaction.")]
    ReplacementUnderpriced(Box<PoolTransaction>),
    #[error("Transaction {0:?} has already been imported into the pool. No need to resubmit.")]
    AlreadyImported(Box<PoolTransaction>),
}

/// Errors that can occur with `eth_feeHistory`
#[derive(Debug, thiserror::Error)]
pub enum FeeHistoryError {
    #[error("Requested block range is out of bounds. Make sure you're requesting blocks within the available history.")]
    InvalidBlockRange,
    #[error("Could not find the newest block requested: {0}. The block might not exist or hasn't been processed yet.")]
    BlockNotFound(BlockNumberOrTag),
}

#[derive(Debug)]
pub struct ErrDetail {
    pub detail: String,
}

/// An error due to invalid transaction
#[derive(Debug, thiserror::Error)]
pub enum InvalidTransactionError {
    /// returned if the nonce of a transaction is lower than the one present in the local chain.
    #[error("Transaction nonce too low. The account's current nonce is higher than the transaction's nonce.")]
    NonceTooLow,
    /// returned if the nonce of a transaction is higher than the next one expected based on the
    /// local chain.
    #[error("Transaction nonce too high. Use a nonce that matches the account's current nonce.")]
    NonceTooHigh,
    /// Returned if the nonce of a transaction is too high
    /// Incrementing the nonce would lead to invalid state (overflow)
    #[error("Transaction nonce has reached the maximum value and cannot be incremented further.")]
    NonceMaxValue,
    /// thrown if the transaction sender doesn't have enough funds for a transfer
    #[error("Insufficient funds for transfer. The account doesn't have enough balance to complete this transaction.")]
    InsufficientFundsForTransfer,
    /// thrown if creation transaction provides the init code bigger than init code size limit.
    #[error("Maximum initialization code size exceeded. Reduce the size of your contract's initialization code.")]
    MaxInitCodeSizeExceeded,
    /// Represents the inability to cover max cost + value (account balance too low).
    #[error("Insufficient funds for gas * price + value. Ensure the account has enough balance to cover all costs.")]
    InsufficientFunds,
    /// Thrown when calculating gas usage
    #[error("Gas uint64 overflow occurred during calculation. Try reducing the gas parameters.")]
    GasUintOverflow,
    /// returned if the transaction is specified to use less gas than required to start the
    /// invocation.
    #[error("Intrinsic gas too low. The transaction requires more gas to execute the basic operations.")]
    GasTooLow,
    /// returned if the transaction gas exceeds the limit
    #[error("Intrinsic gas too high -- {}",.0.detail)]
    GasTooHigh(ErrDetail),
    /// Thrown to ensure no one is able to specify a transaction with a tip higher than the total
    /// fee cap.
    #[error("Max priority fee per gas is higher than max fee per gas. The priority fee cannot exceed the total fee cap.")]
    TipAboveFeeCap,
    /// Thrown post London if the transaction's fee is less than the base fee of the block
    #[error("Max fee per gas less than block base fee. Increase your max fee to at least the current base fee.")]
    FeeCapTooLow,
    /// Thrown during estimate if caller has insufficient funds to cover the tx.
    #[error("Out of gas: gas required ({0:?}) exceeds allowance. Increase the gas limit for this transaction.")]
    BasicOutOfGas(u128),
    /// Thrown if executing a transaction failed during estimate/call
    #[error("Execution reverted: {0:?}. Check your contract code and transaction parameters.")]
    Revert(Option<Bytes>),
    /// Thrown if the sender of a transaction is a contract.
    #[error("Sender is not an externally owned account (EOA). Contracts cannot send transactions directly.")]
    SenderNoEOA,
    /// Thrown when a tx was signed with a different chain_id
    #[error("Invalid chain ID for signer. Make sure you're signing the transaction for the correct network.")]
    InvalidChainId,
    /// Thrown when a legacy tx was signed for a different chain
    #[error("Incompatible EIP-155 transaction, signed for another chain. Ensure you're using the correct network configuration.")]
    IncompatibleEIP155,
    /// Thrown when an access list is used before the berlin hard fork.
    #[error("Access lists are not supported before the Berlin hardfork. Use '--hardfork berlin' or later when running Anvil.")]
    AccessListNotSupported,
    /// Thrown when the block's `blob_gas_price` is greater than tx-specified
    /// `max_fee_per_blob_gas` after Cancun.
    #[error("Block `blob_gas_price` is greater than tx-specified `max_fee_per_blob_gas`. Increase your max fee per blob gas.")]
    BlobFeeCapTooLow,
    /// Thrown when we receive a tx with `blob_versioned_hashes` and we're not on the Cancun hard
    /// fork.
    #[error("Block `blob_versioned_hashes` is not supported before the Cancun hardfork. Use '--hardfork cancun' when running Anvil.")]
    BlobVersionedHashesNotSupported,
    /// Thrown when `max_fee_per_blob_gas` is not supported for blocks before the Cancun hardfork.
    #[error("`max_fee_per_blob_gas` is not supported for blocks before the Cancun hardfork. Use '--hardfork cancun' when running Anvil.")]
    MaxFeePerBlobGasNotSupported,
    /// Thrown when there are no `blob_hashes` in the transaction, and it is an EIP-4844 tx.
    #[error("`blob_hashes` are required for EIP-4844 transactions. Ensure your transaction includes blob hashes.")]
    NoBlobHashes,
    #[error("Too many blobs in one transaction, have: {0}. Reduce the number of blobs in your transaction.")]
    TooManyBlobs(usize),
    /// Thrown when there's a blob validation error
    #[error(transparent)]
    BlobTransactionValidationError(#[from] alloy_consensus::BlobTransactionValidationError),
    /// Thrown when Blob transaction is a create transaction. `to` must be present.
    #[error("Blob transaction can't be a create transaction. `to` must be present.")]
    BlobCreateTransaction,
    /// Thrown when Blob transaction contains a versioned hash with an incorrect version.
    #[error("Blob transaction contains a versioned hash with an incorrect version")]
    BlobVersionNotSupported,
    /// Thrown when there are no `blob_hashes` in the transaction.
    #[error("There should be at least one blob in a Blob transaction.")]
    EmptyBlobs,
    /// Thrown when an access list is used before the berlin hard fork.
    #[error("EIP-7702 authorization lists are not supported before the Prague hardfork")]
    AuthorizationListNotSupported,
    /// Forwards error from the revm
    #[error(transparent)]
    Revm(revm::primitives::InvalidTransaction),
}

impl From<revm::primitives::InvalidTransaction> for InvalidTransactionError {
    fn from(err: revm::primitives::InvalidTransaction) -> Self {
        use revm::primitives::InvalidTransaction;
        match err {
            InvalidTransaction::InvalidChainId => Self::InvalidChainId,
            InvalidTransaction::PriorityFeeGreaterThanMaxFee => Self::TipAboveFeeCap,
            InvalidTransaction::GasPriceLessThanBasefee => Self::FeeCapTooLow,
            InvalidTransaction::CallerGasLimitMoreThanBlock => {
                Self::GasTooHigh(ErrDetail { detail: String::from("CallerGasLimitMoreThanBlock") })
            }
            InvalidTransaction::CallGasCostMoreThanGasLimit => {
                Self::GasTooHigh(ErrDetail { detail: String::from("CallGasCostMoreThanGasLimit") })
            }
            InvalidTransaction::GasFloorMoreThanGasLimit => {
                Self::GasTooHigh(ErrDetail { detail: String::from("CallGasCostMoreThanGasLimit") })
            }
            InvalidTransaction::RejectCallerWithCode => Self::SenderNoEOA,
            InvalidTransaction::LackOfFundForMaxFee { .. } => Self::InsufficientFunds,
            InvalidTransaction::OverflowPaymentInTransaction => Self::GasUintOverflow,
            InvalidTransaction::NonceOverflowInTransaction => Self::NonceMaxValue,
            InvalidTransaction::CreateInitCodeSizeLimit => Self::MaxInitCodeSizeExceeded,
            InvalidTransaction::NonceTooHigh { .. } => Self::NonceTooHigh,
            InvalidTransaction::NonceTooLow { .. } => Self::NonceTooLow,
            InvalidTransaction::AccessListNotSupported => Self::AccessListNotSupported,
            InvalidTransaction::BlobGasPriceGreaterThanMax => Self::BlobFeeCapTooLow,
            InvalidTransaction::BlobVersionedHashesNotSupported => {
                Self::BlobVersionedHashesNotSupported
            }
            InvalidTransaction::MaxFeePerBlobGasNotSupported => Self::MaxFeePerBlobGasNotSupported,
            InvalidTransaction::BlobCreateTransaction => Self::BlobCreateTransaction,
            InvalidTransaction::BlobVersionNotSupported => Self::BlobVersionNotSupported,
            InvalidTransaction::EmptyBlobs => Self::EmptyBlobs,
            InvalidTransaction::TooManyBlobs { have } => Self::TooManyBlobs(have),
            InvalidTransaction::AuthorizationListNotSupported => {
                Self::AuthorizationListNotSupported
            }
            InvalidTransaction::AuthorizationListInvalidFields |
            InvalidTransaction::OptimismError(_) |
            InvalidTransaction::EofCrateShouldHaveToAddress |
            InvalidTransaction::EmptyAuthorizationList => Self::Revm(err),
        }
    }
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
            error!(%err, "Failed serialize rpc response");
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
                    error!(%err, "txpool error");
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
                        if let Some(reason) = data
                            .as_ref()
                            .and_then(|data| RevertDecoder::new().maybe_decode(data, None))
                        {
                            msg = format!("{msg}: {reason}");
                        }
                        RpcError {
                            // geth returns this error code on reverts, See <https://github.com/ethereum/wiki/wiki/JSON-RPC-Error-Codes-Improvement-Proposal>
                            code: ErrorCode::ExecutionError,
                            message: msg.into(),
                            data: serde_json::to_value(data).ok(),
                        }
                    }
                    InvalidTransactionError::GasTooLow => {
                        // <https://eips.ethereum.org/EIPS/eip-1898>
                        RpcError {
                            code: ErrorCode::ServerError(-32000),
                            message: err.to_string().into(),
                            data: None,
                        }
                    }
                    InvalidTransactionError::GasTooHigh(_) => {
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
                BlockchainError::FailedToDecodeReceipt => {
                    RpcError::invalid_params("Failed to decode receipt")
                }
                BlockchainError::FailedToDecodeStateDump => {
                    RpcError::invalid_params("Failed to decode state dump")
                }
                BlockchainError::SignerError(err) => RpcError::invalid_params(err.to_string()),
                BlockchainError::SignatureError(err) => RpcError::invalid_params(err.to_string()),
                BlockchainError::RpcUnimplemented => {
                    RpcError::internal_error_with("Not implemented")
                }
                BlockchainError::PrevrandaoNotSet => RpcError::internal_error_with(err.to_string()),
                BlockchainError::RpcError(err) => err,
                BlockchainError::InvalidFeeInput => RpcError::invalid_params(
                    "Invalid input: `max_priority_fee_per_gas` greater than `max_fee_per_gas`",
                ),
                BlockchainError::AlloyForkProvider(err) => {
                    error!(target: "backend", %err, "fork provider error");
                    match err {
                        TransportError::ErrorResp(err) => RpcError {
                            code: ErrorCode::from(err.code),
                            message: err.message,
                            data: err.data.and_then(|data| serde_json::to_value(data).ok()),
                        },
                        err => RpcError::internal_error_with(format!("Fork Error: {err:?}")),
                    }
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
                err @ BlockchainError::EIP4844TransactionUnsupportedAtHardfork => {
                    RpcError::invalid_params(err.to_string())
                }
                err @ BlockchainError::EIP7702TransactionUnsupportedAtHardfork => {
                    RpcError::invalid_params(err.to_string())
                }
                err @ BlockchainError::DepositTransactionUnsupported => {
                    RpcError::invalid_params(err.to_string())
                }
                err @ BlockchainError::ExcessBlobGasNotSet => {
                    RpcError::invalid_params(err.to_string())
                }
                err @ BlockchainError::Message(_) => RpcError::internal_error_with(err.to_string()),
                err @ BlockchainError::UnknownTransactionType => {
                    RpcError::invalid_params(err.to_string())
                }
            }
            .into(),
        }
    }
}
