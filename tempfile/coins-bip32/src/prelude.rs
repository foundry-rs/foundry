pub use crate::derived::{DerivedKey, DerivedPubkey, DerivedXPriv, DerivedXPub};
pub use crate::enc::{MainnetEncoder, TestnetEncoder, XKeyEncoder};
pub use crate::path::KeyDerivation;
pub use crate::primitives::*;
pub use crate::xkeys::{Parent, XPriv, XPub};
pub use crate::Bip32Error;

#[cfg(any(feature = "mainnet", feature = "testnet"))]
pub use crate::defaults::*;

/// Re-exported signer traits
pub use k256;
pub use k256::{
    ecdsa::{
        signature::{DigestSigner as _, DigestVerifier as _},
        RecoveryId, Signature, SigningKey, VerifyingKey,
    },
    elliptic_curve::sec1::ToEncodedPoint as _,
};

/// shortcut for easy usage
pub fn fingerprint_of(k: &VerifyingKey) -> KeyFingerprint {
    use coins_core::hashes::Digest;
    let digest = coins_core::hashes::Hash160::digest(k.to_sec1_bytes());
    let mut fingerprint = [0u8; 4];
    fingerprint.copy_from_slice(&digest[..4]);
    fingerprint.into()
}
