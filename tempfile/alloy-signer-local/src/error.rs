use alloy_primitives::hex;
use k256::ecdsa;
use thiserror::Error;

/// Error thrown by [`LocalSigner`](crate::LocalSigner).
#[derive(Debug, Error)]
pub enum LocalSignerError {
    /// [`ecdsa`] error.
    #[error(transparent)]
    EcdsaError(#[from] ecdsa::Error),
    /// [`hex`](mod@hex) error.
    #[error(transparent)]
    HexError(#[from] hex::FromHexError),
    /// [`std::io`] error.
    #[error(transparent)]
    IoError(#[from] std::io::Error),

    /// [`coins_bip32`] error.
    #[error(transparent)]
    #[cfg(feature = "mnemonic")]
    Bip32Error(#[from] coins_bip32::Bip32Error),
    /// [`coins_bip39`] error.
    #[error(transparent)]
    #[cfg(feature = "mnemonic")]
    Bip39Error(#[from] coins_bip39::MnemonicError),
    /// [`MnemonicBuilder`](super::mnemonic::MnemonicBuilder) error.
    #[error(transparent)]
    #[cfg(feature = "mnemonic")]
    MnemonicBuilderError(#[from] super::mnemonic::MnemonicBuilderError),

    /// [`eth_keystore`] error.
    #[cfg(feature = "keystore")]
    #[error(transparent)]
    EthKeystoreError(#[from] eth_keystore::KeystoreError),
}
