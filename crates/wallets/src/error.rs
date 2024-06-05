use alloy_signer::k256::ecdsa;
use alloy_signer_ledger::LedgerError;
use alloy_signer_trezor::TrezorError;
use alloy_signer_wallet::WalletError;
use hex::FromHexError;

#[cfg(feature = "aws-kms")]
use alloy_signer_aws::AwsSignerError;

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
    #[cfg(feature = "aws-kms")]
    Aws(#[from] AwsSignerError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    InvalidHex(#[from] FromHexError),
    #[error(transparent)]
    Ecdsa(#[from] ecdsa::Error),
    #[error("foundry was not built with support for {0} signer")]
    UnsupportedSigner(&'static str),
}

impl WalletSignerError {
    pub fn aws_unsupported() -> Self {
        Self::UnsupportedSigner("AWS KMS")
    }
}
