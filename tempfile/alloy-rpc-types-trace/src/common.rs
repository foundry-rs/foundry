//! Types used by tracing backends.

use alloy_primitives::TxHash;
use serde::{Deserialize, Serialize};

/// The result of a single transaction trace.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TraceResult<Ok, Err> {
    /// Untagged success variant
    Success {
        /// Trace results produced by the tracer
        result: Ok,
        /// transaction hash
        #[serde(skip_serializing_if = "Option::is_none", rename = "txHash")]
        #[doc(alias = "transaction_hash")]
        tx_hash: Option<TxHash>,
    },
    /// Untagged error variant
    Error {
        /// Trace failure produced by the tracer
        error: Err,
        /// transaction hash
        #[serde(skip_serializing_if = "Option::is_none", rename = "txHash")]
        #[doc(alias = "transaction_hash")]
        tx_hash: Option<TxHash>,
    },
}

impl<Ok, Err> TraceResult<Ok, Err> {
    /// Returns the hash of the transaction that was traced.
    #[doc(alias = "transaction_hash")]
    pub const fn tx_hash(&self) -> Option<TxHash> {
        *match self {
            Self::Success { tx_hash, .. } | Self::Error { tx_hash, .. } => tx_hash,
        }
    }

    /// Returns a reference to the result if it is a success variant.
    pub const fn success(&self) -> Option<&Ok> {
        match self {
            Self::Success { result, .. } => Some(result),
            Self::Error { .. } => None,
        }
    }

    /// Returns a reference to the error if it is an error variant.
    pub const fn error(&self) -> Option<&Err> {
        match self {
            Self::Error { error, .. } => Some(error),
            Self::Success { .. } => None,
        }
    }

    /// Checks if the result is a success.
    pub const fn is_success(&self) -> bool {
        matches!(self, Self::Success { .. })
    }

    /// Checks if the result is an error.
    pub const fn is_error(&self) -> bool {
        matches!(self, Self::Error { .. })
    }

    /// Creates a new success trace result.
    pub const fn new_success(result: Ok, tx_hash: Option<TxHash>) -> Self {
        Self::Success { result, tx_hash }
    }

    /// Creates a new error trace result.
    pub const fn new_error(error: Err, tx_hash: Option<TxHash>) -> Self {
        Self::Error { error, tx_hash }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use similar_asserts::assert_eq;

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    struct OkResult {
        message: String,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    struct ErrResult {
        code: i32,
    }

    #[test]
    fn test_trace_result_getters() {
        let tx_hash = Some(TxHash::ZERO);

        let success_result: TraceResult<OkResult, ErrResult> =
            TraceResult::new_success(OkResult { message: "Success".to_string() }, tx_hash);

        assert!(success_result.is_success());
        assert!(!success_result.is_error());
        assert_eq!(success_result.tx_hash(), tx_hash);
        assert_eq!(success_result.success(), Some(&OkResult { message: "Success".to_string() }));
        assert_eq!(success_result.error(), None);

        let error_result: TraceResult<OkResult, ErrResult> =
            TraceResult::new_error(ErrResult { code: 404 }, tx_hash);

        assert!(!error_result.is_success());
        assert!(error_result.is_error());
        assert_eq!(error_result.tx_hash(), tx_hash);
        assert_eq!(error_result.success(), None);
        assert_eq!(error_result.error(), Some(&ErrResult { code: 404 }));
    }
}
