//! Zone RPC auth wire format, mirrored from tempoxyz/zones at a1c15e9f.
//!
//! Source: <https://github.com/tempoxyz/zones/tree/a1c15e9f002a5efd150f56c5076fb35519288396/crates/rpc/src/auth/token.rs>

use alloy_primitives::{B256, hex, keccak256};
use alloy_signer::Signer;
use eyre::Result;
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) const HEADER: &str = "x-authorization-token";

/// Sign a short-lived token with the same raw-digest signature as `cast wallet sign --no-hash`.
pub(super) async fn sign_token(
    signer: &impl Signer,
    zone_id: u32,
    chain_id: u64,
) -> Result<String> {
    let issued = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let (fields, digest) = token_fields(zone_id, chain_id, issued, issued + 600);
    let signature = signer.sign_hash(&digest).await?;
    let mut token = signature.as_bytes().to_vec();
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
    use alloy_primitives::Signature;
    use alloy_signer_local::PrivateKeySigner;

    #[tokio::test]
    async fn token_recovers_wallet_from_raw_digest() {
        let signer = PrivateKeySigner::random();
        let token = hex::decode(sign_token(&signer, 42, 1337).await.unwrap()).unwrap();
        assert_eq!(token.len(), 94);
        let fields = &token[65..];
        assert_eq!(&fields[..13], &hex::decode("000000002a0000000000000539").unwrap());
        let issued = u64::from_be_bytes(fields[13..21].try_into().unwrap());
        let expires = u64::from_be_bytes(fields[21..29].try_into().unwrap());
        assert_eq!(expires - issued, 600);
        let (_, digest) = token_fields(42, 1337, issued, expires);
        let signature = Signature::try_from(&token[..65]).unwrap();
        assert_eq!(signature.recover_address_from_prehash(&digest).unwrap(), signer.address());
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
            "0xf827387a933f40dfedece81ba4933feaef89e98a269f52f4f54dda2f1dac4171"
                .parse::<B256>()
                .unwrap()
        );
    }
}
