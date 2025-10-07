//! Host functions overrides for Ethereum address recovery.
//!
//! The host functions overriding is required because of impersonation feature.
//! Impersonation is used for sending/executing transactions with a signer
//! where its private key is unknown. Sent transactions require a signature
//! that is obtained with sender's private key and the pallet-revive
//! runtime logic can recover the signer's ethereum address from the signature
//! and the rest of the transaction bytes. The runtime recovers the signer's
//! address by running `secp256k1_ecdsa_recover` to recover the signer's public
//! key, and `keccak_256` to hash the public key to a 20 byes Ethereum address.
//! These functions are exposed by `sp-io` and used as host functions by the runtime.
//! This module implements tweaked versions of the host functions from `sp-io`, which
//! can recognize fake signatures used for impersonated transactions, and can recover
//! the signer address from them, while expecting those fake signatures to be built in
//! a certain way ([0; 12] + sender's Ethereum address + [0; 33]).
//!
//! The tweaked host functions are especially useful in the context of overriding the
//! same `sp-io` host functions in the wasm executor type.

use polkadot_sdk::sp_io::{self, EcdsaVerifyError};
use sp_runtime_interface::{
    pass_by::{
        AllocateAndReturnByCodec, AllocateAndReturnPointer, PassFatPointerAndRead,
        PassPointerAndRead,
    },
    runtime_interface,
};

// The host functions in this module expect transactions
// with fake signatures conforming the format checked in this function.
fn is_impersonated(sig: &[u8]) -> bool {
    sig[..12] == [0; 12] && sig[32..64] == [0; 32]
}

#[runtime_interface]
pub trait Crypto {
    #[version(1)]
    fn secp256k1_ecdsa_recover(
        sig: PassPointerAndRead<&[u8; 65], 65>,
        msg: PassPointerAndRead<&[u8; 32], 32>,
    ) -> AllocateAndReturnByCodec<Result<[u8; 64], EcdsaVerifyError>> {
        if is_impersonated(sig) {
            trace!(
                target = "host_fn_overrides",
                name = "secp256k1_ecdsa_recover - version 1",
                "impersonation for: {:?}",
                &sig[12..32]
            );
            let mut res = [0u8; 64];
            res[12..32].copy_from_slice(&sig[12..32]);
            Ok(res)
        } else {
            sp_io::crypto::secp256k1_ecdsa_recover(sig, msg)
        }
    }

    #[version(2)]
    fn secp256k1_ecdsa_recover(
        sig: PassPointerAndRead<&[u8; 65], 65>,
        msg: PassPointerAndRead<&[u8; 32], 32>,
    ) -> AllocateAndReturnByCodec<Result<[u8; 64], EcdsaVerifyError>> {
        if is_impersonated(sig) {
            trace!(
                target = "host_fn_overrides",
                name = "secp256k1_ecdsa_recover - version 2",
                "impersonation for: {:?}",
                &sig[12..32]
            );
            let mut res = [0u8; 64];
            res[12..32].copy_from_slice(&sig[12..32]);
            Ok(res)
        } else {
            sp_io::crypto::secp256k1_ecdsa_recover(sig, msg)
        }
    }
}

#[runtime_interface]
pub trait Hashing {
    fn keccak_256(data: PassFatPointerAndRead<&[u8]>) -> AllocateAndReturnPointer<[u8; 32], 32> {
        if data.len() == 64 && is_impersonated(data) {
            trace!(
                target = "host_fn_overrides",
                name = "keccak_256",
                "impersonation for: {:?}",
                &data[12..32]
            );
            let mut res = [0; 32];
            res.copy_from_slice(&data[0..32]);
            res
        } else {
            sp_io::hashing::keccak_256(data)
        }
    }
}

/// Provides host function that overrides ETH address recovery from
/// signature in the scope of impersonation.
pub type SenderAddressRecoveryOverride = self::crypto::HostFunctions;
/// Provides host function that override hashing functions in the
/// scope of impersonation.
pub type PublicKeyToHashOverride = self::hashing::HostFunctions;
