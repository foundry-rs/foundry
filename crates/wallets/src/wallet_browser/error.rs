use alloy_signer::Error as SignerError;

#[derive(Debug, thiserror::Error)]
pub enum BrowserWalletError {
    #[error("{operation} request timed out")]
    Timeout { operation: &'static str },

    #[error("{operation} rejected: {reason}")]
    Rejected { operation: &'static str, reason: String },

    #[error("Wallet not connected")]
    NotConnected,

    #[error("Server error: {0}")]
    ServerError(String),
}

impl From<BrowserWalletError> for SignerError {
    fn from(err: BrowserWalletError) -> Self {
        Self::other(err)
    }
}

impl From<SignerError> for BrowserWalletError {
    fn from(err: SignerError) -> Self {
        Self::ServerError(err.to_string())
    }
}
