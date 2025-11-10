use alloy_primitives::hex::FromHexError;
use alloy_signer::k256::ecdsa;
use alloy_signer_ledger::LedgerError;
use alloy_signer_local::LocalSignerError;
use alloy_signer_trezor::TrezorError;

#[cfg(feature = "aws-kms")]
use alloy_signer_aws::AwsSignerError;

#[cfg(feature = "gcp-kms")]
use alloy_signer_gcp::GcpSignerError;

#[cfg(feature = "turnkey")]
use alloy_signer_turnkey::TurnkeySignerError;

use crate::wallet_browser::error::BrowserWalletError;

#[derive(Debug, thiserror::Error)]
pub enum PrivateKeyError {
    #[error("Failed to create wallet from private key. Private key is invalid hex: {0}")]
    InvalidHex(#[from] FromHexError),
    #[error(
        "Failed to create wallet from private key. Invalid private key. But env var {0} exists. Is the `$` anchor missing?"
    )]
    ExistsAsEnvVar(String),
}

#[derive(Debug, thiserror::Error)]
pub enum WalletSignerError {
    #[error(transparent)]
    Local(#[from] LocalSignerError),
    #[error("Failed to decrypt keystore: incorrect password")]
    IncorrectKeystorePassword,
    #[error(transparent)]
    Ledger(#[from] LedgerError),
    #[error(transparent)]
    Trezor(#[from] TrezorError),
    #[error(transparent)]
    #[cfg(feature = "aws-kms")]
    Aws(#[from] Box<AwsSignerError>),
    #[error(transparent)]
    #[cfg(feature = "gcp-kms")]
    Gcp(#[from] Box<GcpSignerError>),
    #[error(transparent)]
    #[cfg(feature = "turnkey")]
    Turnkey(#[from] TurnkeySignerError),
    #[error(transparent)]
    Browser(#[from] BrowserWalletError),
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

    pub fn gcp_unsupported() -> Self {
        Self::UnsupportedSigner("Google Cloud KMS")
    }

    pub fn turnkey_unsupported() -> Self {
        Self::UnsupportedSigner("Turnkey")
    }

    pub fn browser_unsupported() -> Self {
        Self::UnsupportedSigner("Browser Wallet")
    }
}
