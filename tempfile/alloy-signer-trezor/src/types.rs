//! Helpers for interacting with the Ethereum Trezor App.
//!
//! [Official Docs](https://docs.trezor.io/trezor-firmware/index.html)

use alloy_primitives::hex;
use std::fmt;
use thiserror::Error;

/// Trezor wallet type.
#[derive(Clone, Debug)]
pub enum DerivationType {
    /// Trezor Live-generated HD path
    TrezorLive(usize),
    /// Any other path.
    ///
    /// **Warning**: Trezor by default forbids custom derivation paths;
    /// run `trezorctl set safety-checks prompt` to enable them.
    Other(String),
}

impl fmt::Display for DerivationType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        match self {
            Self::TrezorLive(index) => write!(f, "m/44'/60'/{index}'/0/0"),
            Self::Other(inner) => f.write_str(inner),
        }
    }
}

#[derive(Debug, Error)]
/// Error when using the Trezor transport
pub enum TrezorError {
    /// Underlying Trezor transport error.
    #[error(transparent)]
    Client(#[from] trezor_client::error::Error),
    /// Thrown when converting from a hex string.
    #[error(transparent)]
    Hex(#[from] hex::FromHexError),
    /// Thrown when converting a semver requirement.
    #[error(transparent)]
    Semver(#[from] semver::Error),
    /// Signature Error
    #[error(transparent)]
    SignatureError(#[from] alloy_primitives::SignatureError),
    /// Thrown when trying to sign an EIP-712 struct with an incompatible Trezor Ethereum app
    /// version.
    #[error("Trezor Ethereum app requires at least version {0:?}")]
    UnsupportedFirmwareVersion(String),
    /// Need to provide a chain ID for EIP-155 signing.
    #[error("missing Trezor signer chain ID")]
    MissingChainId,
    /// Could not retrieve device features.
    #[error("could not retrieve device features")]
    Features,
}

impl From<TrezorError> for alloy_signer::Error {
    fn from(error: TrezorError) -> Self {
        Self::other(error)
    }
}
