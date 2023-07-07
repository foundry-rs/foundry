//! Errors when working with wallets

use hex::FromHexError;

#[derive(Debug, thiserror::Error)]
pub enum PrivateKeyError {
    #[error("Failed to create wallet from private key. Private key is invalid hex: {0}")]
    InvalidHex(#[from] FromHexError),
    #[error("Failed to create wallet from private key. Invalid private key. But env var {0} exists. Is the `$` anchor missing?")]
    ExistsAsEnvVar(String),
}
