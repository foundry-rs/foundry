//! Utility functions for working with Ethereum signatures.

use alloy_primitives::{keccak256, Address};
use elliptic_curve::sec1::ToEncodedPoint;
use k256::ecdsa::{SigningKey, VerifyingKey};

/// Converts an ECDSA private key to its corresponding Ethereum Address.
#[inline]
pub fn secret_key_to_address(secret_key: &SigningKey) -> Address {
    public_key_to_address(secret_key.verifying_key())
}

/// Converts an ECDSA public key to its corresponding Ethereum address.
#[inline]
pub fn public_key_to_address(pubkey: &VerifyingKey) -> Address {
    let affine = pubkey.as_ref();
    let encoded = affine.to_encoded_point(false);
    raw_public_key_to_address(&encoded.as_bytes()[1..])
}

/// Convert a raw, uncompressed public key to its corresponding Ethereum address.
///
/// ### Warning
///
/// This method **does not** verify that the public key is valid. It is the
/// caller's responsibility to pass a valid public key. Passing an invalid
/// public key will produce an unspendable output.
///
/// # Panics
///
/// This function panics if the input is not **exactly** 64 bytes.
#[inline]
#[track_caller]
pub fn raw_public_key_to_address(pubkey: &[u8]) -> Address {
    assert_eq!(pubkey.len(), 64, "raw public key must be 64 bytes");
    let digest = keccak256(pubkey);
    Address::from_slice(&digest[12..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::hex;

    // Only tests for correctness, no edge cases. Uses examples from https://docs.ethers.org/v5/api/utils/address/#utils-computeAddress
    #[test]
    fn test_public_key_to_address() {
        let addr = "0Ac1dF02185025F65202660F8167210A80dD5086".parse::<Address>().unwrap();

        // Compressed
        let pubkey = VerifyingKey::from_sec1_bytes(
            &hex::decode("0376698beebe8ee5c74d8cc50ab84ac301ee8f10af6f28d0ffd6adf4d6d3b9b762")
                .unwrap(),
        )
        .unwrap();
        assert_eq!(public_key_to_address(&pubkey), addr);

        // Uncompressed
        let pubkey= VerifyingKey::from_sec1_bytes(&hex::decode("0476698beebe8ee5c74d8cc50ab84ac301ee8f10af6f28d0ffd6adf4d6d3b9b762d46ca56d3dad2ce13213a6f42278dabbb53259f2d92681ea6a0b98197a719be3").unwrap()).unwrap();
        assert_eq!(public_key_to_address(&pubkey), addr);
    }

    #[test]
    fn test_raw_public_key_to_address() {
        let addr = "0Ac1dF02185025F65202660F8167210A80dD5086".parse::<Address>().unwrap();

        let pubkey_bytes = hex::decode("76698beebe8ee5c74d8cc50ab84ac301ee8f10af6f28d0ffd6adf4d6d3b9b762d46ca56d3dad2ce13213a6f42278dabbb53259f2d92681ea6a0b98197a719be3").unwrap();

        assert_eq!(raw_public_key_to_address(&pubkey_bytes), addr);
    }

    #[test]
    #[should_panic]
    fn test_raw_public_key_to_address_panics() {
        raw_public_key_to_address(&[]);
    }
}
