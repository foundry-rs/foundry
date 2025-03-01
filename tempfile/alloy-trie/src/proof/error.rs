use alloy_primitives::{Bytes, B256};
use core::fmt;
use nybbles::Nibbles;

/// Error during proof verification.
#[derive(PartialEq, Eq, Debug)]
pub enum ProofVerificationError {
    /// State root does not match the expected.
    RootMismatch {
        /// Computed state root.
        got: B256,
        /// State root provided to verify function.
        expected: B256,
    },
    /// The node value does not match at specified path.
    ValueMismatch {
        /// Path at which error occurred.
        path: Nibbles,
        /// Value in the proof.
        got: Option<Bytes>,
        /// Expected value.
        expected: Option<Bytes>,
    },
    /// Encountered unexpected empty root node.
    UnexpectedEmptyRoot,
    /// Error during RLP decoding of trie node.
    Rlp(alloy_rlp::Error),
}

/// Enable Error trait implementation when core is stabilized.
/// <https://github.com/rust-lang/rust/issues/103765>
#[cfg(feature = "std")]
impl std::error::Error for ProofVerificationError {
    fn source(&self) -> ::core::option::Option<&(dyn std::error::Error + 'static)> {
        #[allow(deprecated)]
        match self {
            Self::Rlp { 0: transparent } => {
                std::error::Error::source(transparent as &dyn std::error::Error)
            }
            _ => None,
        }
    }
}

impl fmt::Display for ProofVerificationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RootMismatch { got, expected } => {
                write!(f, "root mismatch. got: {got}. expected: {expected}")
            }
            Self::ValueMismatch { path, got, expected } => {
                write!(f, "value mismatch at path {path:?}. got: {got:?}. expected: {expected:?}")
            }
            Self::UnexpectedEmptyRoot => {
                write!(f, "unexpected empty root node")
            }
            Self::Rlp(error) => fmt::Display::fmt(error, f),
        }
    }
}

impl From<alloy_rlp::Error> for ProofVerificationError {
    fn from(source: alloy_rlp::Error) -> Self {
        Self::Rlp(source)
    }
}
