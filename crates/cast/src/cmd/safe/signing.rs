use alloy_dyn_abi::TypedData;
use alloy_primitives::{Address, B256, hex};
use alloy_signer::Signer;
use eyre::Result;
use serde_json::json;
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) async fn sign_delegate(
    signer: &foundry_wallets::WalletSigner,
    delegate: Address,
    chain_id: u64,
) -> Result<String> {
    let totp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() / 3600;
    let typed_data: TypedData = serde_json::from_value(json!({
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
    }))?;
    let signature = signer.sign_dynamic_typed_data(&typed_data).await?;
    Ok(normalize_signature(&signature.as_bytes(), false))
}

pub(super) async fn sign_safe_hash(
    signer: &foundry_wallets::WalletSigner,
    safe_tx_hash: B256,
) -> Result<String> {
    let signature = signer.sign_message(safe_tx_hash.as_slice()).await?;
    Ok(normalize_signature(&signature.as_bytes(), true))
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
}
