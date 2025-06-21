use alloy_signer::Error as SignerError;
use std::fmt;

#[derive(Debug)]
pub enum BrowserWalletError {
    ServerNotRunning,
    ConnectionFailed(String),
    TransactionTimeout,
    SigningTimeout,
    TransactionRejected(String),
    SigningRejected(String),
    InvalidResponse(String),
    ServerError(String),
    NoWalletConnected,
    ChainMismatch { expected: u64, actual: u64 },
}

impl fmt::Display for BrowserWalletError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ServerNotRunning => {
                write!(f, "Browser wallet server is not running. Please start it first.")
            }
            Self::ConnectionFailed(msg) => write!(f, "Failed to connect wallet: {msg}"),
            Self::TransactionTimeout => write!(f, "Transaction request timed out"),
            Self::SigningTimeout => write!(f, "Message signing request timed out"),
            Self::TransactionRejected(msg) => write!(f, "Transaction rejected: {msg}"),
            Self::SigningRejected(msg) => write!(f, "Message signing rejected: {msg}"),
            Self::InvalidResponse(msg) => write!(f, "Invalid response from server: {msg}"),
            Self::ServerError(msg) => write!(f, "Server error: {msg}"),
            Self::NoWalletConnected => {
                write!(f, "No wallet connected. Please connect a wallet first.")
            }
            Self::ChainMismatch { expected, actual } => {
                write!(
                    f,
                    "Chain mismatch: expected chain {expected}, but wallet is on chain {actual}"
                )
            }
        }
    }
}

impl std::error::Error for BrowserWalletError {}

impl From<BrowserWalletError> for SignerError {
    fn from(err: BrowserWalletError) -> Self {
        SignerError::other(err)
    }
}

impl From<SignerError> for BrowserWalletError {
    fn from(err: SignerError) -> Self {
        BrowserWalletError::InvalidResponse(err.to_string())
    }
}
