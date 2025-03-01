//! Error variants when validating an execution payload.

use alloy_primitives::{Bytes, B256, U256};

/// Error that can occur when handling payloads.
#[derive(Debug, derive_more::Display)]
pub enum PayloadError {
    /// Invalid payload extra data.
    #[display("invalid payload extra data: {_0}")]
    ExtraData(Bytes),
    /// Invalid payload base fee.
    #[display("invalid payload base fee: {_0}")]
    BaseFee(U256),
    /// Invalid payload blob gas used.
    #[display("invalid payload blob gas used: {_0}")]
    BlobGasUsed(U256),
    /// Invalid payload excess blob gas.
    #[display("invalid payload excess blob gas: {_0}")]
    ExcessBlobGas(U256),
    /// withdrawals present in pre-shanghai payload.
    #[display("withdrawals present in pre-shanghai payload")]
    PreShanghaiBlockWithWithdrawals,
    /// withdrawals missing in post-shanghai payload.
    #[display("withdrawals missing in post-shanghai payload")]
    PostShanghaiBlockWithoutWithdrawals,
    /// blob transactions present in pre-cancun payload.
    #[display("blob transactions present in pre-cancun payload")]
    PreCancunBlockWithBlobTransactions,
    /// blob gas used present in pre-cancun payload.
    #[display("blob gas used present in pre-cancun payload")]
    PreCancunBlockWithBlobGasUsed,
    /// excess blob gas present in pre-cancun payload.
    #[display("excess blob gas present in pre-cancun payload")]
    PreCancunBlockWithExcessBlobGas,
    /// cancun fields present in pre-cancun payload.
    #[display("cancun fields present in pre-cancun payload")]
    PreCancunWithCancunFields,
    /// blob transactions missing in post-cancun payload.
    #[display("blob transactions missing in post-cancun payload")]
    PostCancunBlockWithoutBlobTransactions,
    /// blob gas used missing in post-cancun payload.
    #[display("blob gas used missing in post-cancun payload")]
    PostCancunBlockWithoutBlobGasUsed,
    /// excess blob gas missing in post-cancun payload.
    #[display("excess blob gas missing in post-cancun payload")]
    PostCancunBlockWithoutExcessBlobGas,
    /// cancun fields missing in post-cancun payload.
    #[display("cancun fields missing in post-cancun payload")]
    PostCancunWithoutCancunFields,
    /// blob transactions present in pre-prague payload.
    #[display("eip 7702 transactions present in pre-prague payload")]
    PrePragueBlockWithEip7702Transactions,
    /// requests present in pre-prague payload.
    #[display("requests present in pre-prague payload")]
    PrePragueBlockRequests,
    /// Invalid payload block hash.
    #[display("block hash mismatch: want {consensus}, got {execution}")]
    BlockHash {
        /// The block hash computed from the payload.
        execution: B256,
        /// The block hash provided with the payload.
        consensus: B256,
    },
    /// Expected blob versioned hashes do not match the given transactions.
    #[display("expected blob versioned hashes do not match the given transactions")]
    InvalidVersionedHashes,
    /// Encountered decoding error.
    #[display("{_0}")]
    Decode(alloy_rlp::Error),
}

impl core::error::Error for PayloadError {}

impl From<alloy_rlp::Error> for PayloadError {
    fn from(value: alloy_rlp::Error) -> Self {
        Self::Decode(value)
    }
}

impl PayloadError {
    /// Returns `true` if the error is caused by a block hash mismatch.
    #[inline]
    pub const fn is_block_hash_mismatch(&self) -> bool {
        matches!(self, Self::BlockHash { .. })
    }

    /// Returns `true` if the error is caused by invalid block hashes (Cancun).
    #[inline]
    pub const fn is_invalid_versioned_hashes(&self) -> bool {
        matches!(self, Self::InvalidVersionedHashes)
    }
}

/// Various errors that can occur when validating a payload or forkchoice update.
///
/// This is intended for the [PayloadStatusEnum::Invalid](crate::PayloadStatusEnum) variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, derive_more::Display)]
pub enum PayloadValidationError {
    /// Thrown when a forkchoice update's head links to a previously rejected payload.
    #[display("links to previously rejected block")]
    LinksToRejectedPayload,
    /// Thrown when a new payload contains a wrong block number.
    #[display("invalid block number")]
    InvalidBlockNumber,
    /// Thrown when a new payload contains a wrong state root
    #[display("invalid merkle root: (remote: {remote:?} local: {local:?})")]
    InvalidStateRoot {
        /// The state root of the payload we received from remote (CL)
        remote: B256,
        /// The state root of the payload that we computed locally.
        local: B256,
    },
}

impl core::error::Error for PayloadValidationError {}
