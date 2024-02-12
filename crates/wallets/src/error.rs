use ethers_signers::{AwsSignerError, LedgerError, TrezorError, WalletError};
use hex::FromHexError;

#[derive(Debug, thiserror::Error)]
pub enum PrivateKeyError {
    #[error("Failed to create wallet from private key. Private key is invalid hex: {0}")]
    InvalidHex(#[from] FromHexError),
    #[error("Failed to create wallet from private key. Invalid private key. But env var {0} exists. Is the `$` anchor missing?")]
    ExistsAsEnvVar(String),
}

#[derive(Debug, thiserror::Error)]
pub enum WalletSignerError {
    #[error(transparent)]
    Local(#[from] WalletError),
    #[error(transparent)]
    Ledger(#[from] LedgerError),
    #[error(transparent)]
    Trezor(#[from] TrezorError),
    #[error(transparent)]
    Aws(#[from] AwsSignerError),
}
