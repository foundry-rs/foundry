//! Aggregated error type for this module

use alloy_consensus::crypto::RecoveryError;
use alloy_primitives::{B256, Bytes, SignatureError, TxHash};
use alloy_rpc_types::BlockNumberOrTag;
use alloy_signer::Error as SignerError;
use alloy_transport::TransportError;
use anvil_core::eth::wallet::WalletError;
use anvil_rpc::{
    error::{ErrorCode, RpcError},
    response::ResponseResult,
};
use evm2::{interpreter::InstrStop, registry::HandlerError};
use foundry_evm_core::decode::RevertDecoder;
use foundry_fork_db::DatabaseError;
use serde::Serialize;
use tempo_evm::TempoInvalidTransaction;
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
    #[error("both gasPrice and (maxFeePerGas or maxPriorityFeePerGas) specified")]
    ConflictingFeeFields,
    #[error("Transaction data is empty")]
    EmptyRawTransactionData,
    #[error("Failed to decode signed transaction")]
    FailedToDecodeSignedTransaction,
    #[error("Failed to decode transaction")]
    FailedToDecodeTransaction,
    #[error("Failed to decode receipt")]
    FailedToDecodeReceipt,
    #[error("Cannot EIP-2718 encode transaction type 0x{0:x}")]
    UnsupportedTransactionEncoding(u8),
    #[error("Failed to decode state")]
    FailedToDecodeStateDump,
    #[error(transparent)]
    SignatureError(#[from] SignatureError),
    #[error(transparent)]
    RecoveryError(#[from] RecoveryError),
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
    EvmError(InstrStop),
    #[error("Evm override error: {0}")]
    EvmOverrideError(String),
    #[error("Invalid url {0:?}")]
    InvalidUrl(String),
    #[error("unsupported fork network for chain {chain_id}: {reason}")]
    UnsupportedForkNetwork { chain_id: u64, reason: &'static str },
    #[error("Internal error: {0:?}")]
    Internal(String),
    #[error("BlockOutOfRangeError: block height is {0} but requested was {1}")]
    BlockOutOfRange(u64, u64),
    #[error("Resource not found")]
    BlockNotFound,
    #[error("unknown block")]
    UnknownBlock,
    /// Thrown when a requested transaction is not found
    #[error("transaction not found")]
    TransactionNotFound,
    #[error("Required data unavailable")]
    DataUnavailable,
    /// Thrown when replaying a mined block requires a parent state that is no longer stored.
    #[error("historical state needed to replay block {0} is not available")]
    HistoricalStateUnavailable(u64),
    #[error("Trie error: {0}")]
    TrieError(String),
    #[error("{0}")]
    UintConversion(&'static str),
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
    // Base is an OP-stack chain and uses the same deposit envelope, so only the hint that names
    // the flags able to enable it varies with the compiled-in families.
    #[cfg_attr(
        all(feature = "base", feature = "optimism"),
        error(
            "op-stack deposit tx received but is not supported.\n\nYou can use it by running anvil with '--optimism' or '--network base'."
        )
    )]
    #[cfg_attr(
        all(feature = "base", not(feature = "optimism")),
        error(
            "op-stack deposit tx received but is not supported.\n\nYou can use it by running anvil with '--network base'."
        )
    )]
    #[cfg_attr(
        not(feature = "base"),
        error(
            "op-stack deposit tx received but is not supported.\n\nYou can use it by running anvil with '--optimism'."
        )
    )]
    DepositTransactionUnsupported,
    #[error(
        "tempo transaction received but is not supported.\n\nYou can use it by running anvil with '--tempo'."
    )]
    TempoTransactionUnsupported,
    #[cfg(feature = "base")]
    #[error("Base transactions require native Base execution; run Anvil with --network base")]
    BaseTransactionUnsupported,
    #[error("Unknown transaction type not supported")]
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
    #[error("Invalid transaction request: {0}")]
    InvalidTransactionRequest(String),
    #[cfg(feature = "base")]
    #[error("EIP-8130 transaction rejected: {0}")]
    Eip8130TransactionRejected(String),
    #[error("filter not found")]
    FilterNotFound,
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

impl From<evm2::DatabaseError> for BlockchainError {
    fn from(err: evm2::DatabaseError) -> Self {
        // Remote fork failures keep their typed error so RPC callers see the fork error.
        match err.downcast_ref::<DatabaseError>().and_then(clone_database_error) {
            Some(err) => Self::DatabaseError(err),
            None => Self::Message(err.to_string()),
        }
    }
}

/// Clones a fork database error, which does not implement `Clone`.
pub(crate) fn clone_database_error(err: &DatabaseError) -> Option<DatabaseError> {
    Some(match err {
        DatabaseError::MissingCode(hash) => DatabaseError::MissingCode(*hash),
        DatabaseError::BlockNumberOverflow(number) => DatabaseError::BlockNumberOverflow(*number),
        DatabaseError::GetAccount(address, err) => DatabaseError::GetAccount(*address, err.clone()),
        DatabaseError::GetStorage(address, slot, err) => {
            DatabaseError::GetStorage(*address, *slot, err.clone())
        }
        DatabaseError::GetBlockHash(number, err) => {
            DatabaseError::GetBlockHash(*number, err.clone())
        }
        DatabaseError::GetFullBlock(block, err) => DatabaseError::GetFullBlock(*block, err.clone()),
        DatabaseError::BlockNotFound(block) => DatabaseError::BlockNotFound(*block),
        DatabaseError::GetTransaction(hash, err) => {
            DatabaseError::GetTransaction(*hash, err.clone())
        }
        DatabaseError::AnyRequest(err) => DatabaseError::AnyRequest(err.clone()),
        DatabaseError::Recv(err) => DatabaseError::Recv(*err),
        DatabaseError::Send(err) => DatabaseError::Send(err.clone()),
    })
}

impl From<HandlerError> for BlockchainError {
    fn from(err: HandlerError) -> Self {
        match err {
            HandlerError::Database(err) => err.into(),
            HandlerError::Fatal(err) => Self::Message(err.to_string()),
            HandlerError::External(err) => {
                #[cfg(feature = "optimism")]
                if let Some(err) = err.downcast_ref::<op_evm2::OpTxError>() {
                    return match err {
                        op_evm2::OpTxError::MissingEnvelopedTx => {
                            Self::InvalidTransaction(InvalidTransactionError::MissingEnvelopedTx)
                        }
                        op_evm2::OpTxError::DepositSystemTxPostRegolith => {
                            Self::DepositTransactionUnsupported
                        }
                    };
                }
                if let Some(err) = err.downcast_ref::<TempoInvalidTransaction>() {
                    return Self::Message(format!("tempo transaction error: {err}"));
                }
                Self::InvalidTransaction(InvalidTransactionError::Rejected(err.to_string()))
            }
            HandlerError::UnsupportedTransactionType(ty) => {
                #[cfg(feature = "base")]
                if ty == base_common_consensus::EIP8130_TX_TYPE_ID {
                    return Self::BaseTransactionUnsupported;
                }
                Self::InvalidTransaction(InvalidTransactionError::Rejected(format!(
                    "unsupported transaction type 0x{ty:02x}"
                )))
            }
            err => Self::InvalidTransaction(err.into()),
        }
    }
}

impl From<WalletError> for BlockchainError {
    fn from(value: WalletError) -> Self {
        Self::Message(value.to_string())
    }
}

/// Errors that can occur in the transaction pool
#[derive(Debug, thiserror::Error)]
pub enum PoolError {
    #[error("Transaction with cyclic dependent transactions")]
    CyclicTransaction,
    /// Thrown if a replacement transaction's gas price is below the already imported transaction
    #[error("Tx: [{0:?}] insufficient gas price to replace existing transaction")]
    ReplacementUnderpriced(TxHash),
    #[error("Tx: [{0:?}] already Imported")]
    AlreadyImported(TxHash),
}

/// Errors that can occur with `eth_feeHistory`
#[derive(Debug, thiserror::Error)]
pub enum FeeHistoryError {
    #[error("requested block range is out of bounds")]
    InvalidBlockRange,
    #[error("reward percentiles must be strictly increasing and between 0 and 100")]
    InvalidRewardPercentiles,
    #[error("could not find block number requested: {0}")]
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
    /// Thrown when the block's `blob_gas_price` is greater than tx-specified
    /// `max_fee_per_blob_gas` after Cancun.
    #[error("Block `blob_gas_price` is greater than tx-specified `max_fee_per_blob_gas`")]
    BlobFeeCapTooLow(u128, u128),
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
    /// Thrown when a blob transaction is submitted on a Monad network.
    #[error("EIP-4844 blob transactions are not supported on Monad")]
    MonadBlobTransactionUnsupported,
    /// Thrown when there are no `blob_hashes` in the transaction.
    #[error("There should be at least one blob in a Blob transaction.")]
    EmptyBlobs,
    #[error("Transaction gas limit is greater than the block gas limit, gas_limit: {0}, cap: {1}")]
    TxGasLimitGreaterThanCap(u64, u64),
    /// Any other transaction validation failure reported by evm2.
    #[error("{0}")]
    Rejected(String),
    /// Missing enveloped transaction
    #[error("missing enveloped transaction")]
    MissingEnvelopedTx,
    /// EIP-8130 transaction failed block-inclusion validation.
    #[cfg(feature = "base")]
    #[error("EIP-8130 transaction rejected: {0}")]
    Eip8130(String),
    /// Native ETH value transfers are not allowed in Tempo mode
    #[error("native value transfer not allowed in Tempo mode")]
    TempoNativeValueTransfer,
    /// Tempo transaction valid_before is expired or too close to current time
    #[error("Tempo tx valid_before ({valid_before}) must be > current time + 3s ({min_allowed})")]
    TempoValidBeforeExpired { valid_before: u64, min_allowed: u64 },
    /// Tempo expiring nonce transaction valid_before is too far in the future.
    #[error(
        "Tempo expiring nonce tx valid_before ({valid_before}) must be <= current time + {max_expiry_secs}s ({max_allowed})"
    )]
    TempoValidBeforeTooFar { valid_before: u64, max_expiry_secs: u64, max_allowed: u64 },
    /// Tempo transaction valid_after is too far in the future
    #[error(
        "Tempo tx valid_after ({valid_after}) must be <= current time + {max_valid_after_secs}s ({max_allowed})"
    )]
    TempoValidAfterTooFar { valid_after: u64, max_valid_after_secs: u64, max_allowed: u64 },
    /// Tempo transaction has too many authorizations
    #[error("Tempo tx has too many authorizations ({count}), max allowed is {max}")]
    TempoTooManyAuthorizations { count: usize, max: usize },
}

impl From<HandlerError> for InvalidTransactionError {
    fn from(err: HandlerError) -> Self {
        match err {
            HandlerError::InvalidChainId { .. } => Self::InvalidChainId,
            HandlerError::PriorityFeeGreaterThanMaxFee => Self::TipAboveFeeCap,
            HandlerError::FeeCapLessThanBaseFee { .. } => Self::FeeCapTooLow,
            HandlerError::GasLimitMoreThanBlock { .. } => {
                Self::GasTooHigh(ErrDetail { detail: String::from("CallerGasLimitMoreThanBlock") })
            }
            HandlerError::IntrinsicGasTooLow { .. } => {
                Self::GasTooHigh(ErrDetail { detail: String::from("CallGasCostMoreThanGasLimit") })
            }
            HandlerError::RejectCallerWithCode => Self::SenderNoEOA,
            HandlerError::InsufficientFunds { .. } => Self::InsufficientFunds,
            HandlerError::NonceOverflow => Self::NonceMaxValue,
            HandlerError::CreateInitCodeSizeLimit { .. } => Self::MaxInitCodeSizeExceeded,
            HandlerError::InvalidNonce { expected, got } if got > expected => Self::NonceTooHigh,
            HandlerError::InvalidNonce { .. } => Self::NonceTooLow,
            HandlerError::BlobFeeCapLessThanBlobBaseFee { max_fee_per_blob_gas, blob_base_fee } => {
                Self::BlobFeeCapTooLow(
                    blob_base_fee.saturating_to(),
                    max_fee_per_blob_gas.saturating_to(),
                )
            }
            HandlerError::BlobVersionNotSupported => Self::BlobVersionNotSupported,
            HandlerError::EmptyBlobs => Self::EmptyBlobs,
            HandlerError::TooManyBlobs { have, max } => Self::TooManyBlobs(have, max),
            HandlerError::TxGasLimitGreaterThanCap { gas_limit, cap } => {
                Self::TxGasLimitGreaterThanCap(gas_limit, cap)
            }
            err => Self::Rejected(err.to_string()),
        }
    }
}

/// Helper trait to easily convert results to rpc results
pub(crate) trait ToRpcResponseResult {
    fn to_rpc_result(self) -> ResponseResult;
}

/// Converts a serializable value into a `ResponseResult`
pub fn to_rpc_result<T: Serialize>(val: T) -> ResponseResult {
    match serde_json::value::to_raw_value(&val) {
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
            Err(err) => blockchain_error_to_rpc_result(err),
        }
    }
}

/// Converts a [`BlockchainError`] into a `ResponseResult`.
///
/// This is not generic so that the error mapping is only compiled once.
fn blockchain_error_to_rpc_result(err: BlockchainError) -> ResponseResult {
    match err {
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
        BlockchainError::NoSignerAvailable => RpcError::invalid_params("No Signer available"),
        BlockchainError::ChainIdNotAvailable => RpcError::invalid_params("Chain Id not available"),
        BlockchainError::TransactionConfirmationTimeout { hash, .. } => RpcError {
            code: ErrorCode::ServerError(4),
            message: "Transaction confirmation timeout".into(),
            data: Some(serde_json::Value::String(hash.to_string())),
        },
        BlockchainError::InvalidTransaction(err) => match err {
            InvalidTransactionError::Revert(data) => {
                // this mimics geth revert error
                let mut msg = "execution reverted".to_string();
                if let Some(reason) =
                    data.as_ref().and_then(|data| RevertDecoder::new().maybe_decode(data, None))
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
        BlockchainError::UnsupportedTransactionEncoding(_) => {
            RpcError::internal_error_with(err.to_string())
        }
        BlockchainError::FailedToDecodeStateDump => {
            RpcError::invalid_params("Failed to decode state dump")
        }
        BlockchainError::SignerError(err) => RpcError::invalid_params(err.to_string()),
        BlockchainError::SignatureError(err) => RpcError::invalid_params(err.to_string()),
        BlockchainError::RpcUnimplemented => RpcError::internal_error_with("Not implemented"),
        BlockchainError::RpcError(err) => err,
        BlockchainError::InvalidFeeInput => RpcError::invalid_params(
            "Invalid input: `max_priority_fee_per_gas` greater than `max_fee_per_gas`",
        ),
        err @ BlockchainError::ConflictingFeeFields => RpcError::invalid_params(err.to_string()),
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
        err @ BlockchainError::EvmError(_) => RpcError {
            // VM halts are execution failures, not JSON-RPC server faults. REVERT has a
            // dedicated code/data path above; other halts, such as invalid opcode, do not.
            code: ErrorCode::TransactionRejected,
            message: err.to_string().into(),
            data: None,
        },
        #[cfg(feature = "base")]
        err @ BlockchainError::Eip8130TransactionRejected(_) => RpcError {
            code: ErrorCode::TransactionRejected,
            message: err.to_string().into(),
            data: None,
        },
        err @ BlockchainError::EvmOverrideError(_) => RpcError::invalid_params(err.to_string()),
        err @ BlockchainError::InvalidUrl(_) => RpcError::invalid_params(err.to_string()),
        err @ BlockchainError::UnsupportedForkNetwork { .. } => {
            RpcError::invalid_params(err.to_string())
        }
        BlockchainError::Internal(err) => RpcError::internal_error_with(err),
        err @ BlockchainError::BlockOutOfRange(_, _) => RpcError::invalid_params(err.to_string()),
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
        err @ BlockchainError::DataUnavailable => RpcError::internal_error_with(err.to_string()),
        err @ BlockchainError::HistoricalStateUnavailable(_) => RpcError {
            code: ErrorCode::ServerError(-32000),
            message: err.to_string().into(),
            data: None,
        },
        err @ BlockchainError::TrieError(_) => RpcError::internal_error_with(err.to_string()),
        BlockchainError::UintConversion(err) => RpcError::invalid_params(err),
        err @ BlockchainError::TimestampError(_) => RpcError::invalid_params(err.to_string()),
        BlockchainError::DatabaseError(err) => RpcError::internal_error_with(err.to_string()),
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
        #[cfg(feature = "base")]
        err @ BlockchainError::BaseTransactionUnsupported => {
            RpcError::invalid_params(err.to_string())
        }
        err @ BlockchainError::TempoTransactionUnsupported => {
            RpcError::invalid_params(err.to_string())
        }
        err @ BlockchainError::ExcessBlobGasNotSet => RpcError::invalid_params(err.to_string()),
        err @ BlockchainError::Message(_) => RpcError::internal_error_with(err.to_string()),
        err @ BlockchainError::UnknownTransactionType => RpcError::invalid_params(err.to_string()),
        err @ BlockchainError::InvalidTransactionRequest(_) => {
            RpcError::invalid_params(err.to_string())
        }
        err @ BlockchainError::RecoveryError(_) => RpcError::invalid_params(err.to_string()),
        BlockchainError::FilterNotFound => RpcError {
            code: ErrorCode::ServerError(-32000),
            message: "filter not found".into(),
            data: None,
        },
        err @ BlockchainError::UnknownBlock => RpcError {
            code: ErrorCode::ServerError(-32000),
            message: err.to_string().into(),
            data: None,
        },
    }
    .into()
}
