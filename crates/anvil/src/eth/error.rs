//! Aggregated error type for this module

use crate::eth::pool::transactions::PoolTransaction;
use alloy_evm::overrides::StateOverrideError;
use alloy_primitives::{B256, Bytes, SignatureError};
use alloy_rpc_types::BlockNumberOrTag;
use alloy_signer::Error as SignerError;
use alloy_transport::TransportError;
use anvil_core::eth::wallet::WalletError;
use anvil_rpc::{
    error::{ErrorCode, RpcError},
    response::ResponseResult,
};
use foundry_evm::{backend::DatabaseError, decode::RevertDecoder};
use op_revm::OpTransactionError;
use revm::{
    context_interface::result::{EVMError, InvalidHeader, InvalidTransaction},
    interpreter::InstructionResult,
};
use serde::Serialize;
use tokio::time::Duration;

pub(crate) type Result<T> = std::result::Result<T, BlockchainError>;

#[derive(Debug, thiserror::Error)]
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
    #[error("Failed to decode receipt")]
    FailedToDecodeReceipt,
    #[error("Failed to decode state")]
    FailedToDecodeStateDump,
    #[error("Prevrandao not in th EVM's environment after merge")]
    PrevrandaoNotSet,
    #[error(transparent)]
    SignatureError(#[from] SignatureError),
    #[error(transparent)]
    SignerError(#[from] SignerError),
    #[error("Rpc Endpoint not implemented")]
    RpcUnimplemented,
    #[error("Rpc error {0:?}")]
    RpcError(RpcError),
    #[error(transparent)]
    InvalidTransaction(#[from] InvalidTransactionError),
    #[error(transparent)]
    FeeHistory(#[from] FeeHistoryError),
    #[error(transparent)]
    AlloyForkProvider(#[from] TransportError),
    #[error("EVM error {0:?}")]
    EvmError(InstructionResult),
    #[error("Evm override error: {0}")]
    EvmOverrideError(String),
    #[error("Invalid url {0:?}")]
    InvalidUrl(String),
    #[error("Internal error: {0:?}")]
    Internal(String),
    #[error("BlockOutOfRangeError: block height is {0} but requested was {1}")]
    BlockOutOfRange(u64, u64),
    #[error("Resource not found")]
    BlockNotFound,
    /// Thrown when a requested transaction is not found
    #[error("transaction not found")]
    TransactionNotFound,
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
    #[error(
        "EIP-1559 style fee params (maxFeePerGas or maxPriorityFeePerGas) received but they are not supported by the current hardfork.\n\nYou can use them by running anvil with '--hardfork london' or later."
    )]
    EIP1559TransactionUnsupportedAtHardfork,
    #[error(
        "Access list received but is not supported by the current hardfork.\n\nYou can use it by running anvil with '--hardfork berlin' or later."
    )]
    EIP2930TransactionUnsupportedAtHardfork,
    #[error(
        "EIP-4844 fields received but is not supported by the current hardfork.\n\nYou can use it by running anvil with '--hardfork cancun' or later."
    )]
    EIP4844TransactionUnsupportedAtHardfork,
    #[error(
        "EIP-7702 fields received but is not supported by the current hardfork.\n\nYou can use it by running anvil with '--hardfork prague' or later."
    )]
    EIP7702TransactionUnsupportedAtHardfork,
    #[error(
        "op-stack deposit tx received but is not supported.\n\nYou can use it by running anvil with '--optimism'."
    )]
    DepositTransactionUnsupported,
    #[error("UnknownTransactionType not supported ")]
    UnknownTransactionType,
    #[error("Excess blob gas not set.")]
    ExcessBlobGasNotSet,
    #[error("{0}")]
    Message(String),
    #[error("Transaction {hash} was added to the mempool but wasn't confirmed within {duration:?}")]
    TransactionConfirmationTimeout {
        /// Hash of the transaction that timed out
        hash: B256,
        /// Duration that was waited before timing out
        duration: Duration,
    },
    #[error("Failed to parse transaction request: missing required fields")]
    MissingRequiredFields,
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
            EVMError::Custom(err) => Self::Message(err),
        }
    }
}

impl<T> From<EVMError<T, OpTransactionError>> for BlockchainError
where
    T: Into<Self>,
{
    fn from(err: EVMError<T, OpTransactionError>) -> Self {
        match err {
            EVMError::Transaction(err) => match err {
                OpTransactionError::Base(err) => InvalidTransactionError::from(err).into(),
                OpTransactionError::DepositSystemTxPostRegolith => {
                    Self::DepositTransactionUnsupported
                }
                OpTransactionError::HaltedDepositPostRegolith => {
                    Self::DepositTransactionUnsupported
                }
            },
            EVMError::Header(err) => match err {
                InvalidHeader::ExcessBlobGasNotSet => Self::ExcessBlobGasNotSet,
                InvalidHeader::PrevrandaoNotSet => Self::PrevrandaoNotSet,
            },
            EVMError::Database(err) => err.into(),
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

impl<E> From<StateOverrideError<E>> for BlockchainError
where
    E: Into<Self>,
{
    fn from(value: StateOverrideError<E>) -> Self {
        match value {
            StateOverrideError::InvalidBytecode(err) => Self::StateOverrideError(err.to_string()),
            StateOverrideError::BothStateAndStateDiff(addr) => Self::StateOverrideError(format!(
                "state and state_diff can't be used together for account {addr}",
            )),
            StateOverrideError::Database(err) => err.into(),
        }
    }
}

/// Errors that can occur in the transaction pool
#[derive(Debug, thiserror::Error)]
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
#[derive(Debug, thiserror::Error)]
pub enum FeeHistoryError {
    #[error("requested block range is out of bounds")]
    InvalidBlockRange,
    #[error("could not find newest block number requested: {0}")]
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
    #[error("intrinsic gas too high -- {}",.0.detail)]
    GasTooHigh(ErrDetail),
    /// Thrown to ensure no one is able to specify a transaction with a tip higher than the total
    /// fee cap.
    #[error("max priority fee per gas higher than max fee per gas")]
    TipAboveFeeCap,
    /// Thrown post London if the transaction's fee is less than the base fee of the block
    #[error("max fee per gas less than block base fee")]
    FeeCapTooLow,
    /// Thrown during estimate if caller has insufficient funds to cover the tx.
    #[error("Out of gas: gas required exceeds allowance: {0:?}")]
    BasicOutOfGas(u128),
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
    /// Thrown when an access list is used before the berlin hard fork.
    #[error("Access lists are not supported before the Berlin hardfork")]
    AccessListNotSupported,
    /// Thrown when the block's `blob_gas_price` is greater than tx-specified
    /// `max_fee_per_blob_gas` after Cancun.
    #[error("Block `blob_gas_price` is greater than tx-specified `max_fee_per_blob_gas`")]
    BlobFeeCapTooLow,
    /// Thrown when we receive a tx with `blob_versioned_hashes` and we're not on the Cancun hard
    /// fork.
    #[error("Block `blob_versioned_hashes` is not supported before the Cancun hardfork")]
    BlobVersionedHashesNotSupported,
    /// Thrown when `max_fee_per_blob_gas` is not supported for blocks before the Cancun hardfork.
    #[error("`max_fee_per_blob_gas` is not supported for blocks before the Cancun hardfork.")]
    MaxFeePerBlobGasNotSupported,
    /// Thrown when there are no `blob_hashes` in the transaction, and it is an EIP-4844 tx.
    #[error("`blob_hashes` are required for EIP-4844 transactions")]
    NoBlobHashes,
    #[error("too many blobs in one transaction, have: {0}, max: {1}")]
    TooManyBlobs(usize, usize),
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
    #[error("Transaction gas limit is greater than the block gas limit, gas_limit: {0}, cap: {1}")]
    TxGasLimitGreaterThanCap(u64, u64),
    /// Forwards error from the revm
    #[error(transparent)]
    Revm(revm::context_interface::result::InvalidTransaction),
    /// Deposit transaction error post regolith
    #[error("op-deposit failure post regolith")]
    DepositTxErrorPostRegolith,
}

impl From<InvalidTransaction> for InvalidTransactionError {
    fn from(err: InvalidTransaction) -> Self {
        match err {
            InvalidTransaction::InvalidChainId => Self::InvalidChainId,
            InvalidTransaction::PriorityFeeGreaterThanMaxFee => Self::TipAboveFeeCap,
            InvalidTransaction::GasPriceLessThanBasefee => Self::FeeCapTooLow,
            InvalidTransaction::CallerGasLimitMoreThanBlock => {
                Self::GasTooHigh(ErrDetail { detail: String::from("CallerGasLimitMoreThanBlock") })
            }
            InvalidTransaction::CallGasCostMoreThanGasLimit { .. } => {
                Self::GasTooHigh(ErrDetail { detail: String::from("CallGasCostMoreThanGasLimit") })
            }
            InvalidTransaction::GasFloorMoreThanGasLimit { .. } => {
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
            InvalidTransaction::TooManyBlobs { have, max } => Self::TooManyBlobs(have, max),
            InvalidTransaction::AuthorizationListNotSupported => {
                Self::AuthorizationListNotSupported
            }
            InvalidTransaction::TxGasLimitGreaterThanCap { gas_limit, cap } => {
                Self::TxGasLimitGreaterThanCap(gas_limit, cap)
            }

            InvalidTransaction::AuthorizationListInvalidFields
            | InvalidTransaction::Eip1559NotSupported
            | InvalidTransaction::Eip2930NotSupported
            | InvalidTransaction::Eip4844NotSupported
            | InvalidTransaction::Eip7702NotSupported
            | InvalidTransaction::EmptyAuthorizationList
            | InvalidTransaction::Eip7873NotSupported
            | InvalidTransaction::Eip7873MissingTarget
            | InvalidTransaction::MissingChainId => Self::Revm(err),
        }
    }
}

impl From<OpTransactionError> for InvalidTransactionError {
    fn from(value: OpTransactionError) -> Self {
        match value {
            OpTransactionError::Base(err) => err.into(),
            OpTransactionError::DepositSystemTxPostRegolith
            | OpTransactionError::HaltedDepositPostRegolith => Self::DepositTxErrorPostRegolith,
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
                BlockchainError::TransactionConfirmationTimeout { .. } => {
                    RpcError::internal_error_with("Transaction confirmation timeout")
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
                            // geth returns this error code on reverts, See <https://eips.ethereum.org/EIPS/eip-1474#specification>
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
                err @ BlockchainError::EvmOverrideError(_) => {
                    RpcError::invalid_params(err.to_string())
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
                err @ BlockchainError::TransactionNotFound => RpcError {
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
                err @ BlockchainError::MissingRequiredFields => {
                    RpcError::invalid_params(err.to_string())
                }
            }
            .into(),
        }
    }
}
