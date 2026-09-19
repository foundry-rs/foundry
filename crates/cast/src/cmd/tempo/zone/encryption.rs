//! Client-side deposit ECIES, mirrored from tempoxyz/zones at a1c15e9f.
//!
//! Source: <https://github.com/tempoxyz/zones/tree/a1c15e9f002a5efd150f56c5076fb35519288396/crates/precompiles/src/ecies.rs>

use super::abi::DepositPayload;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce, aead::Aead};
use alloy_primitives::{Address, B256, U256};
use eyre::{Result, eyre};
use hkdf::Hkdf;
use k256::{
    AffinePoint, ProjectivePoint, PublicKey, SecretKey, elliptic_curve::sec1::ToEncodedPoint,
};
use sha2::Sha256;

/// Encrypt `[recipient(20) | memo(32) | padding(12)]` for the sequencer.
pub(super) fn encrypt_deposit(
    x: B256,
    parity: u8,
    to: Address,
    memo: B256,
    sender: Address,
    portal: Address,
    key_index: U256,
) -> Result<DepositPayload> {
    let mut compressed = [0u8; 33];
    compressed[0] = match parity {
        0 | 1 => parity + 2,
        2 | 3 => parity,
        _ => return Err(eyre!("invalid sequencer encryption key parity")),
    };
    compressed[1..].copy_from_slice(x.as_slice());
    let public = PublicKey::from_sec1_bytes(&compressed)
        .map_err(|_| eyre!("invalid sequencer encryption public key"))?;
    let ephemeral = SecretKey::random(&mut rand_08::thread_rng());
    let encoded = ephemeral.public_key().to_encoded_point(true);
    let ephemeral_x = B256::from_slice(encoded.x().unwrap());
    let shared = AffinePoint::from(
        ProjectivePoint::from(*public.as_affine()) * *ephemeral.to_nonzero_scalar(),
    );
    let shared = shared.to_encoded_point(true);
    let mut info = [0u8; 104];
    info[..20].copy_from_slice(portal.as_slice());
    info[20..52].copy_from_slice(&key_index.to_be_bytes::<32>());
    info[52..84].copy_from_slice(ephemeral_x.as_slice());
    info[84..].copy_from_slice(sender.as_slice());
    let mut key = [0u8; 32];
    Hkdf::<Sha256>::new(Some(b"ecies-aes-key"), shared.x().unwrap())
        .expand(&info, &mut key)
        .map_err(|_| eyre!("deposit key derivation failed"))?;
    let mut plaintext = [0u8; 64];
    plaintext[..20].copy_from_slice(to.as_slice());
    plaintext[20..52].copy_from_slice(memo.as_slice());
    let nonce: [u8; 12] = rand_08::random();
    let mut encrypted = Aes256Gcm::new((&key).into())
        .encrypt(Nonce::from_slice(&nonce), plaintext.as_ref())
        .map_err(|_| eyre!("deposit encryption failed"))?;
    let tag = encrypted.split_off(64);
    Ok(DepositPayload {
        ephemeralPubkeyX: ephemeral_x,
        ephemeralPubkeyYParity: encoded.as_bytes()[0],
        ciphertext: encrypted.into(),
        nonce: nonce.into(),
        tag: tag.as_slice().try_into()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deposit_roundtrip_and_context_binding() {
        let sequencer = SecretKey::from_slice(&[7; 32]).unwrap();
        let public = sequencer.public_key().to_encoded_point(true);
        let x = B256::from_slice(public.x().unwrap());
        let to = Address::repeat_byte(0xbb);
        let memo = B256::repeat_byte(0xcc);
        let sender = Address::repeat_byte(0xdd);
        let portal = Address::repeat_byte(0xaa);
        let index = U256::from(42);
        let payload =
            encrypt_deposit(x, public.as_bytes()[0], to, memo, sender, portal, index).unwrap();
        assert_eq!(payload.ciphertext.len(), 64);
        let mut ephemeral = [0u8; 33];
        ephemeral[0] = payload.ephemeralPubkeyYParity;
        ephemeral[1..].copy_from_slice(payload.ephemeralPubkeyX.as_slice());
        let ephemeral = PublicKey::from_sec1_bytes(&ephemeral).unwrap();
        let shared = AffinePoint::from(
            ProjectivePoint::from(*ephemeral.as_affine()) * *sequencer.to_nonzero_scalar(),
        );
        let shared = shared.to_encoded_point(true);
        // Zones' decryption context is packed, not ABI-padded.
        let mut info = [
            portal.as_slice(),
            &index.to_be_bytes::<32>(),
            payload.ephemeralPubkeyX.as_slice(),
            sender.as_slice(),
        ]
        .concat();
        let mut ciphertext = payload.ciphertext.to_vec();
        ciphertext.extend_from_slice(payload.tag.as_slice());
        let mut key = [0u8; 32];
        Hkdf::<Sha256>::new(Some(b"ecies-aes-key"), shared.x().unwrap())
            .expand(&info, &mut key)
            .unwrap();
        let cipher = Aes256Gcm::new((&key).into());
        let plaintext = cipher
            .decrypt(Nonce::from_slice(payload.nonce.as_slice()), ciphertext.as_slice())
            .unwrap();
        assert_eq!(&plaintext[..20], to.as_slice());
        assert_eq!(&plaintext[20..52], memo.as_slice());
        assert_eq!(&plaintext[52..], &[0; 12]);
        // Changing any bound field must fail authentication.
        for offset in [0, 20, 52, 84] {
            info[offset] ^= 1;
            Hkdf::<Sha256>::new(Some(b"ecies-aes-key"), shared.x().unwrap())
                .expand(&info, &mut key)
                .unwrap();
            assert!(
                Aes256Gcm::new((&key).into())
                    .decrypt(Nonce::from_slice(payload.nonce.as_slice()), ciphertext.as_slice())
                    .is_err()
            );
            info[offset] ^= 1;
        }
        let second =
            encrypt_deposit(x, public.as_bytes()[0], to, memo, sender, portal, index).unwrap();
        assert_ne!(payload.ephemeralPubkeyX, second.ephemeralPubkeyX);
        assert_ne!(payload.nonce, second.nonce);
    }

    #[test]
    fn rejects_invalid_encryption_keys() {
        for parity in [0, 1, 2, 3, 4, 255] {
            assert!(
                encrypt_deposit(
                    B256::repeat_byte(0xff),
                    parity,
                    Address::ZERO,
                    B256::ZERO,
                    Address::ZERO,
                    Address::ZERO,
                    U256::ZERO
                )
                .is_err()
            );
        }
    }
}
