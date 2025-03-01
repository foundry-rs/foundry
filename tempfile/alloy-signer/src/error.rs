use alloy_primitives::{hex, ChainId};
use k256::ecdsa;
use std::fmt;
use thiserror::Error;

/// Result type alias for [`Error`](enum@Error).
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Generic error type for [`Signer`](crate::Signer) implementations.
#[derive(Debug, Error)]
pub enum Error {
    /// This operation is not supported by the signer.
    #[error("operation `{0}` is not supported by the signer")]
    UnsupportedOperation(UnsupportedSignerOperation),
    /// Mismatch between provided transaction chain ID and signer chain ID.
    #[error(
        "transaction-provided chain ID ({tx}) does not match the signer's chain ID ({signer})"
    )]
    TransactionChainIdMismatch {
        /// The signer's chain ID.
        signer: ChainId,
        /// The chain ID provided by the transaction.
        tx: ChainId,
    },
    /// [`alloy_dyn_abi`] error.
    #[error(transparent)]
    #[cfg(feature = "eip712")]
    DynAbiError(#[from] alloy_dyn_abi::Error),
    /// [`ecdsa`] error.
    #[error(transparent)]
    Ecdsa(#[from] ecdsa::Error),
    /// [`hex`](mod@hex) error.
    #[error(transparent)]
    HexError(#[from] hex::FromHexError),
    /// Signature error.
    #[error(transparent)]
    SignatureError(#[from] alloy_primitives::SignatureError),
    /// Generic error.
    #[error(transparent)]
    Other(#[from] Box<dyn std::error::Error + Send + Sync + 'static>),
}

impl Error {
    /// Constructs a new [`Other`](Self::Other) error.
    #[cold]
    pub fn other(error: impl Into<Box<dyn std::error::Error + Send + Sync + 'static>>) -> Self {
        Self::Other(error.into())
    }

    /// Returns `true` if the error is [`UnsupportedOperation`](Self::UnsupportedOperation).
    #[inline]
    pub const fn is_unsupported(&self) -> bool {
        matches!(self, Self::UnsupportedOperation(_))
    }

    /// Returns the [`UnsupportedSignerOperation`] if the error is
    /// [`UnsupportedOperation`](Self::UnsupportedOperation).
    #[inline]
    pub const fn unsupported(&self) -> Option<UnsupportedSignerOperation> {
        match self {
            Self::UnsupportedOperation(op) => Some(*op),
            _ => None,
        }
    }
}

/// An unsupported signer operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum UnsupportedSignerOperation {
    /// `sign_hash` is not supported by the signer.
    SignHash,
    /// `sign_message` is not supported by the signer.
    SignMessage,
    /// `sign_transaction` is not supported by the signer.
    SignTransaction,
    /// `sign_typed_data` is not supported by the signer.
    SignTypedData,
}

impl fmt::Display for UnsupportedSignerOperation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_str().fmt(f)
    }
}

impl UnsupportedSignerOperation {
    /// Returns the string representation of the operation.
    #[inline]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::SignHash => "sign_hash",
            Self::SignMessage => "sign_message",
            Self::SignTransaction => "sign_transaction",
            Self::SignTypedData => "sign_typed_data",
        }
    }
}
