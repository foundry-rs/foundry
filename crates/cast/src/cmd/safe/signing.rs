use alloy_dyn_abi::TypedData;
use alloy_primitives::{Address, B256, hex};
use alloy_signer::Signer;
use eyre::Result;
use foundry_wallets::WalletSigner;
use serde_json::json;
use std::time::{SystemTime, UNIX_EPOCH};

const DELEGATE_TOTP_PERIOD_SECS: u64 = 60 * 60;

pub(super) async fn sign_delegate(
    signer: &WalletSigner,
    delegate: Address,
    chain_id: u64,
) -> Result<String> {
    let totp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() / DELEGATE_TOTP_PERIOD_SECS;
    let typed_data = delegate_typed_data(delegate, chain_id, totp)?;
    sign_delegate_typed_data(signer, &typed_data, matches!(signer, WalletSigner::Trezor(_))).await
}

pub(super) async fn sign_safe_hash(signer: &WalletSigner, safe_tx_hash: B256) -> Result<String> {
    let signature = signer.sign_message(safe_tx_hash.as_slice()).await?;
    Ok(normalize_signature(&signature.as_bytes(), true))
}

fn delegate_typed_data(delegate: Address, chain_id: u64, totp: u64) -> Result<TypedData> {
    Ok(serde_json::from_value(json!({
        "types": {
            "EIP712Domain": [
                { "name": "name", "type": "string" },
                { "name": "version", "type": "string" },
                { "name": "chainId", "type": "uint256" }
            ],
            "Delegate": [
                { "name": "delegateAddress", "type": "address" },
                { "name": "totp", "type": "uint256" }
            ]
        },
        "primaryType": "Delegate",
        "domain": {
            "name": "Safe Transaction Service",
            "version": "1.0",
            "chainId": chain_id
        },
        "message": {
            "delegateAddress": delegate.to_checksum(None),
            "totp": totp
        }
    }))?)
}

async fn sign_delegate_typed_data(
    signer: &WalletSigner,
    typed_data: &TypedData,
    safe_eth_sign: bool,
) -> Result<String> {
    let signature = if safe_eth_sign {
        signer.sign_message(typed_data.eip712_signing_hash()?.as_slice()).await?
    } else {
        signer.sign_dynamic_typed_data(typed_data).await?
    };
    Ok(normalize_signature(&signature.as_bytes(), safe_eth_sign))
}

fn normalize_signature(signature: &[u8], safe_eth_sign: bool) -> String {
    let mut signature = signature.to_vec();
    let v = &mut signature[64];
    if *v < 27 {
        *v += 27;
    }
    if safe_eth_sign {
        *v += 4;
    }
    hex::encode_prefixed(signature)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::Signature;

    #[test]
    fn normalizes_safe_eth_sign_v() {
        let mut signature = [0u8; 65];
        signature[64] = 1;
        assert!(normalize_signature(&signature, true).ends_with("20"));

        signature[64] = 27;
        assert!(normalize_signature(&signature, true).ends_with("1f"));
    }

    #[test]
    fn normalizes_typed_data_v() {
        let mut signature = [0u8; 65];
        signature[64] = 0;
        assert!(normalize_signature(&signature, false).ends_with("1b"));
    }

    #[tokio::test]
    async fn signs_delegate_typed_data_with_supported_safe_signatures() -> Result<()> {
        let typed_data = delegate_typed_data(Address::repeat_byte(0x11), 1, 1)?;
        let signer = WalletSigner::from_private_key(&B256::repeat_byte(1))?;
        let signing_hash = typed_data.eip712_signing_hash()?;

        for safe_eth_sign in [false, true] {
            let signature = sign_delegate_typed_data(&signer, &typed_data, safe_eth_sign).await?;
            let mut bytes = hex::decode(signature.strip_prefix("0x").unwrap())?;
            if safe_eth_sign {
                assert!(matches!(bytes[64], 31 | 32));
                bytes[64] -= 4;
            } else {
                assert!(matches!(bytes[64], 27 | 28));
            }
            let signature = Signature::from_raw(&bytes)?;
            let recovered = if safe_eth_sign {
                signature.recover_address_from_msg(signing_hash.as_slice())?
            } else {
                signature.recover_address_from_prehash(&signing_hash)?
            };
            assert_eq!(recovered, signer.address());
        }
        Ok(())
    }
}
