//! Zone RPC auth wire format, mirrored from tempoxyz/zones at a1c15e9f.
//!
//! Source: <https://github.com/tempoxyz/zones/tree/a1c15e9f002a5efd150f56c5076fb35519288396/crates/rpc/src/auth/token.rs>
//!
//! The signature is a Tempo transaction signature over the token digest. tempoxyz/tempo's zone
//! RPC accepts configurable account (`0x05`) signatures from #9045; see `authenticate_token` in
//! <https://github.com/tempoxyz/tempo/blob/c8b89f9b6d9f011a8a277ab431183b2b105c4af7/crates/zones/rpc/src/server.rs>.

use super::signer::ZoneSigner;
use alloy_primitives::{B256, hex, keccak256};
use eyre::Result;
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) const HEADER: &str = "x-authorization-token";

/// Sign a short-lived token as `signer`, see [`ZoneSigner::sign_token_digest`].
pub(super) async fn sign_token(signer: &ZoneSigner, zone_id: u32, chain_id: u64) -> Result<String> {
    let issued = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let (fields, digest) = token_fields(zone_id, chain_id, issued, issued + 600);
    let mut token = signer.sign_token_digest(digest).await?.to_vec();
    token.extend_from_slice(&fields);
    Ok(hex::encode(token))
}

fn token_fields(zone_id: u32, chain_id: u64, issued: u64, expires: u64) -> ([u8; 29], B256) {
    let mut message = [0u8; 61];
    message[..12].copy_from_slice(b"TempoZoneRPC");
    message[33..37].copy_from_slice(&zone_id.to_be_bytes());
    message[37..45].copy_from_slice(&chain_id.to_be_bytes());
    message[45..53].copy_from_slice(&issued.to_be_bytes());
    message[53..61].copy_from_slice(&expires.to_be_bytes());
    (message[32..].try_into().unwrap(), keccak256(message))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{Signature, b256};
    use alloy_signer::Signer;
    use alloy_signer_local::PrivateKeySigner;
    use foundry_common::tempo::configurable_account::{
        ConfigurableAccountSigner, single_owner_config,
    };
    use foundry_wallets::WalletSigner;
    use tempo_primitives::transaction::{
        ConfigurableAccountSignature, PrimitiveSignature, tt_signature::TempoSignature,
    };

    #[tokio::test]
    async fn token_recovers_wallet_from_raw_digest() {
        let signer = PrivateKeySigner::random();
        let address = signer.address();
        let signer = ZoneSigner::Wallet(WalletSigner::Local(signer));
        let token = hex::decode(sign_token(&signer, 42, 1337).await.unwrap()).unwrap();
        assert_eq!(token.len(), 94);
        let fields = &token[65..];
        assert_eq!(&fields[..13], &hex::decode("000000002a0000000000000539").unwrap());
        let issued = u64::from_be_bytes(fields[13..21].try_into().unwrap());
        let expires = u64::from_be_bytes(fields[21..29].try_into().unwrap());
        assert_eq!(expires - issued, 600);
        let (_, digest) = token_fields(42, 1337, issued, expires);
        let signature = Signature::try_from(&token[..65]).unwrap();
        assert_eq!(signature.recover_address_from_prehash(&digest).unwrap(), address);
    }

    /// Repeats the signature checks of tempoxyz/tempo's zone RPC at c8b89f9 (`authenticate_token`
    /// in crates/zones/rpc/src/server.rs) on a configurable account token, with Tempo's own
    /// types, and pins the owners' digest.
    #[tokio::test]
    async fn configurable_account_token_matches_zone_rpc_checks() {
        let owner = PrivateKeySigner::random();
        let config = single_owner_config(owner.address());
        let account = config.derive_account().unwrap();
        let signer = ZoneSigner::ConfigurableAccount(
            ConfigurableAccountSigner::new(
                account,
                config.clone(),
                [WalletSigner::Local(owner.clone()).into()],
                "",
            )
            .unwrap(),
        );
        let token = hex::decode(sign_token(&signer, 42, 1337).await.unwrap()).unwrap();

        // `<signature><29 fixed bytes>`, with a `0x05` signature.
        let (signature, fields) = token.split_at(token.len() - 29);
        assert_eq!(signature[0], 0x05);
        let issued = u64::from_be_bytes(fields[13..21].try_into().unwrap());
        let (expected_fields, digest) = token_fields(42, 1337, issued, issued + 600);
        assert_eq!(fields, expected_fields);

        let parsed = TempoSignature::from_bytes(signature).unwrap();
        assert!(!parsed.is_legacy_keychain());
        assert_eq!(parsed.recover_signer(&digest).unwrap(), account);
        let TempoSignature::ConfigurableAccount(approval) = &parsed else {
            panic!("a configurable account signature")
        };
        // Unregistered, the owners derive the account; registered, they match its commitment.
        approval.validate_account_commitment(B256::ZERO).unwrap();
        approval.validate_account_commitment(config.commitment().unwrap()).unwrap();
        assert!(approval.validate_account_commitment(B256::repeat_byte(1)).is_err());

        // The owner approves keccak256(domain || token digest || account || version).
        let mut preimage = b"tempo:configurable-account:signature".to_vec();
        preimage.extend_from_slice(digest.as_slice());
        preimage.extend_from_slice(account.as_slice());
        preimage.extend_from_slice(&0u64.to_be_bytes());
        let approval = owner.sign_hash(&keccak256(preimage)).await.unwrap();
        let expected = ConfigurableAccountSignature::try_new(
            account,
            config,
            vec![PrimitiveSignature::Secp256k1(approval)],
        )
        .unwrap();
        assert_eq!(signature, TempoSignature::ConfigurableAccount(expected).to_bytes().as_ref());
    }

    #[test]
    fn matches_zones_wire_vector() {
        let (fields, digest) = token_fields(
            0x0102_0304,
            0x0506_0708_090a_0b0c,
            0x0d0e_0f10_1112_1314,
            0x1516_1718_191a_1b1c,
        );
        assert_eq!(
            hex::encode(fields),
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c"
        );
        assert_eq!(
            digest,
            b256!("0xf827387a933f40dfedece81ba4933feaef89e98a269f52f4f54dda2f1dac4171")
        );
    }
}
