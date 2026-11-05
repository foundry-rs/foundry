//! Native Tempo precompiles cannot be replaced with account bytecode.
//!
//! Empty and unchanged writes are rejected to protect native account markers.

use super::*;
use alloy_provider::MULTICALL3_ADDRESS;
use anvil::eth::error::BlockchainError;
use foundry_evm::hardfork::EthereumHardfork;
use tempo_precompiles::{
    SIGNATURE_VERIFIER_ADDRESS, STORAGE_CREDITS_ADDRESS, VALIDATOR_CONFIG_ADDRESS,
    VALIDATOR_CONFIG_V2_ADDRESS, ZONE_FACTORY_ADDRESS, ZONE_VERIFIER_ADDRESS,
};

const UNINITIALIZED_TIP20: Address = address!("0x20c0000000000000000000006a37da5c996874be");

#[tokio::test(flavor = "multi_thread")]
async fn test_anvil_set_code_rejects_native_token() {
    let (api, handle) = spawn(NodeConfig::test_tempo().with_no_mining(true)).await;
    let provider = handle.http_provider();
    let original_code = provider.get_code_at(PATH_USD).await.unwrap();
    assert!(!original_code.is_empty());
    assert!(provider.get_code_at(UNINITIALIZED_TIP20).await.unwrap().is_empty());
    let before: Bytes = provider.raw_request("anvil_dumpState".into(), ()).await.unwrap();

    assert_set_code_rejected(&provider, PATH_USD, "TIP20Token", original_code).await;
    for address in [PATH_USD, UNINITIALIZED_TIP20] {
        for code in [bytes!("60006000f3"), Bytes::new()] {
            assert_set_code_rejected(&provider, address, "TIP20Token", code).await;
        }
    }
    let BlockchainError::RpcError(error) =
        api.anvil_set_code(PATH_USD, Bytes::new()).await.unwrap_err()
    else {
        panic!("expected native precompile error");
    };
    assert_eq!(error.code.code(), -32602);
    assert_eq!(error.message, rejection_message(PATH_USD, "TIP20Token"));
    assert_eq!(error.data, None);

    let after: Bytes = provider.raw_request("anvil_dumpState".into(), ()).await.unwrap();
    assert_eq!(after, before);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_anvil_set_code_rejects_native_precompiles() {
    let (_api, handle) = spawn(
        NodeConfig::test_tempo()
            .with_hardfork(Some(TempoHardfork::T13.into()))
            .with_no_mining(true),
    )
    .await;
    let provider = handle.http_provider();
    let before: Bytes = provider.raw_request("anvil_dumpState".into(), ()).await.unwrap();
    for (address, name) in [
        (TIP20_FACTORY_ADDRESS, "TIP20Factory"),
        (TIP403_REGISTRY_ADDRESS, "TIP403Registry"),
        (TIP_FEE_MANAGER_ADDRESS, "TipFeeManager"),
        (STABLECOIN_DEX_ADDRESS, "StablecoinDEX"),
        (NONCE_PRECOMPILE_ADDRESS, "NonceManager"),
        (VALIDATOR_CONFIG_ADDRESS, "ValidatorConfig"),
        (VALIDATOR_CONFIG_V2_ADDRESS, "ValidatorConfigV2"),
        (ACCOUNT_KEYCHAIN_ADDRESS, "AccountKeychain"),
        (SIGNATURE_VERIFIER_ADDRESS, "SignatureVerifier"),
        (ADDRESS_REGISTRY_ADDRESS, "AddressRegistry"),
        (TIP20_CHANNEL_RESERVE_ADDRESS, "TIP20ChannelReserve"),
        (RECEIVE_POLICY_GUARD_ADDRESS, "ReceivePolicyGuard"),
        (STORAGE_CREDITS_ADDRESS, "StorageCredits"),
        (CURRENT_COMMITTEE_ADDRESS, "CurrentCommittee"),
        (ZONE_FACTORY_ADDRESS, "ZoneFactory"),
        (ZONE_VERIFIER_ADDRESS, "ZoneVerifier"),
    ] {
        assert_set_code_rejected(&provider, address, name, bytes!("00")).await;
        assert_set_code_rejected(&provider, address, name, Bytes::new()).await;
    }
    let after: Bytes = provider.raw_request("anvil_dumpState".into(), ()).await.unwrap();
    assert_eq!(after, before);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_anvil_set_code_respects_native_activation() {
    for (address, name, before, active) in [
        (ADDRESS_REGISTRY_ADDRESS, "AddressRegistry", TempoHardfork::T2, TempoHardfork::T3),
        (SIGNATURE_VERIFIER_ADDRESS, "SignatureVerifier", TempoHardfork::T2, TempoHardfork::T3),
        (
            TIP20_CHANNEL_RESERVE_ADDRESS,
            "TIP20ChannelReserve",
            TempoHardfork::T4,
            TempoHardfork::T5,
        ),
        (RECEIVE_POLICY_GUARD_ADDRESS, "ReceivePolicyGuard", TempoHardfork::T5, TempoHardfork::T6),
        (STORAGE_CREDITS_ADDRESS, "StorageCredits", TempoHardfork::T6, TempoHardfork::T7),
        (CURRENT_COMMITTEE_ADDRESS, "CurrentCommittee", TempoHardfork::T7, TempoHardfork::T8),
        (ZONE_FACTORY_ADDRESS, "ZoneFactory", TempoHardfork::T9, TempoHardfork::T10),
        (ZONE_VERIFIER_ADDRESS, "ZoneVerifier", TempoHardfork::T12, TempoHardfork::T13),
    ] {
        for hardfork in [before, active] {
            let (_api, handle) = spawn(
                NodeConfig::test_tempo().with_hardfork(Some(hardfork.into())).with_no_mining(true),
            )
            .await;
            let provider = handle.http_provider();
            if hardfork == before {
                assert_set_code_allowed(&provider, address).await;
            } else {
                assert_set_code_rejected(&provider, address, name, bytes!("00")).await;
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_anvil_set_code_preserves_bytecode_contracts() {
    let (_api, handle) = spawn(NodeConfig::test_tempo().with_no_mining(true)).await;
    let provider = handle.http_provider();
    assert!(!provider.get_code_at(MULTICALL3_ADDRESS).await.unwrap().is_empty());
    for address in [Address::random(), MULTICALL3_ADDRESS, Address::with_last_byte(1)] {
        assert_set_code_allowed(&provider, address).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_anvil_set_code_preserves_ethereum_behavior() {
    let (_api, handle) = spawn(NodeConfig::test().with_no_mining(true)).await;
    let provider = handle.http_provider();
    for address in
        [PATH_USD, UNINITIALIZED_TIP20, TIP20_FACTORY_ADDRESS, Address::with_last_byte(1)]
    {
        assert_set_code_allowed(&provider, address).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_anvil_set_code_rejects_native_with_ethereum_hardfork_override() {
    let (_api, handle) = spawn(
        NodeConfig::test_tempo()
            .with_hardfork(Some(EthereumHardfork::Prague.into()))
            .with_no_mining(true),
    )
    .await;
    let provider = handle.http_provider();
    assert_set_code_rejected(&provider, PATH_USD, "TIP20Token", bytes!("00")).await;
    assert_set_code_rejected(&provider, TIP20_FACTORY_ADDRESS, "TIP20Factory", Bytes::new()).await;
}

async fn assert_set_code_rejected(
    provider: &RetryProvider,
    address: Address,
    name: &str,
    code: Bytes,
) {
    let error =
        provider.raw_request::<_, ()>("anvil_setCode".into(), (address, code)).await.unwrap_err();
    assert_eq!(
        serde_json::to_value(error.as_error_resp().unwrap()).unwrap(),
        serde_json::json!({
            "code": -32602,
            "message": rejection_message(address, name),
            "data": null,
        }),
    );
}

async fn assert_set_code_allowed(provider: &RetryProvider, address: Address) {
    for code in [bytes!("602a5f5260205ff3"), Bytes::new()] {
        provider.raw_request::<_, ()>("anvil_setCode".into(), (address, &code)).await.unwrap();
        assert_eq!(provider.get_code_at(address).await.unwrap(), code);
    }
}

fn rejection_message(address: Address, name: &str) -> String {
    format!(
        "cannot set code for native Tempo precompile {name} at {address}: calls are handled natively and do not execute account bytecode; code writes are rejected to preserve native account markers, including TIP20 initialization"
    )
}
