pub use alloy_eip5792::*;

#[derive(Debug, thiserror::Error)]
pub enum WalletError {
    /// The transaction value is not 0.
    ///
    /// The value should be 0 to prevent draining the sequencer.
    #[error("transaction value must be zero for delegated transactions")]
    ValueNotZero,
    /// The from field is set on the transaction.
    ///
    /// Requests with the from field are rejected, since it is implied that it will always be the
    /// sequencer.
    #[error("transaction 'from' field should not be set for delegated transactions")]
    FromSet,
    /// The nonce field is set on the transaction.
    ///
    /// Requests with the nonce field set are rejected, as this is managed by the sequencer.
    #[error("transaction nonce should not be set for delegated transactions")]
    NonceSet,
    /// An authorization item was invalid.
    ///
    /// The item is invalid if it tries to delegate an account to a contract that is not
    /// whitelisted.
    #[error("invalid authorization address: contract is not whitelisted for delegation")]
    InvalidAuthorization,
    /// The to field of the transaction was invalid.
    ///
    /// The destination is invalid if:
    ///
    /// - There is no bytecode at the destination, or
    /// - The bytecode is not an EIP-7702 delegation designator, or
    /// - The delegation designator points to a contract that is not whitelisted
    #[error("transaction destination is not a valid delegated account")]
    IllegalDestination,
    /// The transaction request was invalid.
    ///
    /// This is likely an internal error, as most of the request is built by the sequencer.
    #[error("invalid transaction request format")]
    InvalidTransactionRequest,
    /// An internal error occurred.
    #[error("internal server error occurred")]
    InternalError,
}
