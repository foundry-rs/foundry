use alloy_provider::{Provider, ProviderBuilder};
use eyre::WrapErr;
use foundry_evm_hardforks::TempoHardfork;
use serde::Deserialize;
use std::env;

use crate::FoundryTransactionBuilder;
use alloy_chains::{Chain, NamedChain};
use alloy_network::TransactionBuilder;
use alloy_primitives::{Address, Bytes, TxKind, U256, address};
use alloy_provider::mock::Asserter;
use alloy_rpc_types::TransactionRequest;
use alloy_sol_types::{SolCall, SolValue};
use tempo_alloy::{
    TempoNetwork,
    contracts::precompiles::{IFeeManager, IStablecoinDEX, ITIP20, STABLECOIN_DEX_ADDRESS},
    rpc::TempoTransactionRequest,
};
use tempo_primitives::transaction::{Call, SignatureType};

use super::{
    ALPHA_USD_ADDRESS, BETA_USD_ADDRESS, PATH_USD_ADDRESS, THETA_USD_ADDRESS,
    TIP_FEE_MANAGER_ADDRESS, TempoSponsor, known_fee_token_symbol, resolve_and_set_fee_token,
    resolve_fee_token_symbol,
};

#[test]
fn browser_gas_estimation_uses_conservative_webauthn_hint() {
    let request = TempoTransactionRequest {
        inner: TransactionRequest {
            to: Some(TxKind::Create),
            value: Some(U256::from(7)),
            input: vec![0x60, 0x00].into(),
            ..Default::default()
        },
        ..Default::default()
    };
    let estimate_request = request.browser_wallet_gas_estimation_request();

    assert_eq!(estimate_request.key_type, Some(SignatureType::WebAuthn));
    assert_eq!(estimate_request.key_data, Some(Bytes::from_static(&[0x05, 0x78])));
    let json = serde_json::to_value(&estimate_request).unwrap();
    assert_eq!(json["keyType"], "webAuthn");
    assert_eq!(json["keyData"], "0x0578");
    assert_eq!(
        estimate_request.calls,
        vec![Call { to: TxKind::Create, value: U256::from(7), input: vec![0x60, 0x00].into() }]
    );
    assert_eq!(estimate_request.inner.to, None);
    assert_eq!(estimate_request.inner.value, None);
    assert_eq!(estimate_request.inner.input.input(), None);
    assert_eq!(request.key_type, None);
    assert_eq!(request.key_data, None);
    assert_eq!(request.inner.to, Some(TxKind::Create));
}

#[test]
fn browser_gas_estimation_preserves_complete_signer_hints() {
    for key_type in [SignatureType::Secp256k1, SignatureType::P256, SignatureType::WebAuthn] {
        let request = TempoTransactionRequest {
            key_type: Some(key_type),
            key_data: Some(Bytes::from_static(&[0x01])),
            ..Default::default()
        };
        let estimate_request = request.browser_wallet_gas_estimation_request();
        assert_eq!(estimate_request.key_type, request.key_type);
        assert_eq!(estimate_request.key_data, request.key_data);
    }
}

#[test]
fn browser_gas_estimation_fills_missing_webauthn_key_data() {
    let request =
        TempoTransactionRequest { key_type: Some(SignatureType::WebAuthn), ..Default::default() };
    let estimate_request = request.browser_wallet_gas_estimation_request();
    assert_eq!(estimate_request.key_type, request.key_type);
    assert_eq!(estimate_request.key_data, Some(Bytes::from_static(&[0x05, 0x78])));
}

#[test]
fn browser_gas_estimation_replaces_key_data_without_key_type() {
    let request = TempoTransactionRequest {
        key_data: Some(Bytes::from_static(&[0x01])),
        ..Default::default()
    };
    let estimate_request = request.browser_wallet_gas_estimation_request();
    assert_eq!(estimate_request.key_type, Some(SignatureType::WebAuthn));
    assert_eq!(estimate_request.key_data, Some(Bytes::from_static(&[0x05, 0x78])));
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TempoForkSchedule {
    schedule: Vec<ForkInfo>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ForkInfo {
    name: String,
}

async fn check_fork_schedule(rpc_url: &str) -> eyre::Result<()> {
    let provider = ProviderBuilder::new().connect_http(rpc_url.parse()?);
    let schedule: TempoForkSchedule = provider.raw_request("tempo_forkSchedule".into(), ()).await?;
    for fork in &schedule.schedule {
        fork.name.parse::<TempoHardfork>()?;
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork_schedule_parses_configured_rpcs() -> eyre::Result<()> {
    let mut checked_any = false;

    for (network, env_key) in [
        ("mainnet", "TEMPO_MAINNET_RPC_URL"),
        ("testnet", "TEMPO_TESTNET_RPC_URL"),
        ("devnet", "TEMPO_DEVNET_RPC_URL"),
    ] {
        let rpc_url = match env::var(env_key) {
            Ok(url) if !url.is_empty() => url,
            _ => continue,
        };
        checked_any = true;

        check_fork_schedule(&rpc_url)
            .await
            .wrap_err_with(|| format!("[{network}] {env_key}={rpc_url}"))?;
    }

    if !checked_any {
        let _ = crate::sh_eprintln!(
            "Missing Tempo RPC env vars. Skipping Tempo fork schedule compatibility test."
        );
    }

    Ok(())
}

#[tokio::test]
async fn explicit_fee_token_overrides_stored_user_token_when_applied() -> eyre::Result<()> {
    let asserter = Asserter::new();
    let provider =
        ProviderBuilder::new_with_network::<TempoNetwork>().connect_mocked_client(asserter);
    let explicit = Address::repeat_byte(0x42);
    let fee_payer = Address::repeat_byte(0x11);
    let mut tx = TempoTransactionRequest {
        inner: TransactionRequest::default().with_from(fee_payer),
        fee_token: Some(explicit),
        ..Default::default()
    };

    assert_eq!(
        resolve_and_set_fee_token::<TempoNetwork>(
            Some(&provider),
            Some(Chain::from_named(NamedChain::Tempo)),
            &mut tx,
            Some(fee_payer),
        )
        .await?,
        Some(explicit)
    );
    assert_eq!(tx.fee_token, Some(explicit));
    Ok(())
}

#[tokio::test]
async fn default_fee_token_resolution_leaves_transaction_fee_token_unset() -> eyre::Result<()> {
    let explicit = Address::repeat_byte(0x42);
    let mut tx = TempoTransactionRequest { fee_token: Some(explicit), ..Default::default() };

    let resolved = resolve_and_set_fee_token::<TempoNetwork>(
        None,
        Some(Chain::from_named(NamedChain::Tempo)),
        &mut tx,
        None,
    )
    .await?;
    assert_eq!(resolved, Some(explicit));
    assert_eq!(tx.fee_token, Some(explicit));

    let mut tx = TempoTransactionRequest::default();
    let resolved = resolve_and_set_fee_token::<TempoNetwork>(
        None,
        Some(Chain::from_named(NamedChain::Tempo)),
        &mut tx,
        None,
    )
    .await?;
    assert_eq!(resolved, None);
    assert_eq!(tx.fee_token, None);

    let mut tx = TempoTransactionRequest::default();
    let resolved = resolve_and_set_fee_token::<TempoNetwork>(
        None,
        Some(Chain::from_named(NamedChain::Mainnet)),
        &mut tx,
        None,
    )
    .await?;
    assert_eq!(resolved, None);
    assert_eq!(tx.fee_token, None);
    Ok(())
}

#[tokio::test]
async fn send_fee_token_resolution_can_skip_lookup_for_curl_mode() -> eyre::Result<()> {
    let asserter = Asserter::new();
    let provider =
        ProviderBuilder::new_with_network::<TempoNetwork>().connect_mocked_client(asserter.clone());
    let fee_payer = Address::repeat_byte(0x11);
    let mut tx = TempoTransactionRequest::default();

    asserter.push_success(&BETA_USD_ADDRESS.abi_encode());
    assert_eq!(
        resolve_and_set_fee_token::<TempoNetwork>(
            Some(&provider),
            Some(Chain::from_named(NamedChain::Tempo)),
            &mut tx,
            Some(fee_payer),
        )
        .await?,
        Some(BETA_USD_ADDRESS)
    );
    assert_eq!(tx.fee_token, Some(BETA_USD_ADDRESS));

    let mut tx = TempoTransactionRequest::default();
    assert_eq!(
        resolve_and_set_fee_token::<TempoNetwork>(
            None,
            Some(Chain::from_named(NamedChain::Tempo)),
            &mut tx,
            Some(fee_payer),
        )
        .await?,
        None
    );
    assert_eq!(tx.fee_token, None);

    Ok(())
}

#[tokio::test]
async fn unset_user_token_does_not_stamp_default_fee_token() -> eyre::Result<()> {
    let asserter = Asserter::new();
    let provider =
        ProviderBuilder::new_with_network::<TempoNetwork>().connect_mocked_client(asserter.clone());
    let fee_payer = Address::repeat_byte(0x11);
    let mut tx = TempoTransactionRequest::default();

    asserter.push_success(&Address::ZERO.abi_encode());

    assert_eq!(
        resolve_and_set_fee_token::<TempoNetwork>(
            Some(&provider),
            Some(Chain::from_named(NamedChain::Tempo)),
            &mut tx,
            Some(fee_payer),
        )
        .await?,
        None
    );
    assert_eq!(tx.fee_token, None);

    Ok(())
}

#[tokio::test]
async fn contract_creation_does_not_stamp_stored_fee_token() -> eyre::Result<()> {
    let asserter = Asserter::new();
    let provider =
        ProviderBuilder::new_with_network::<TempoNetwork>().connect_mocked_client(asserter);
    let fee_payer = Address::repeat_byte(0x11);
    let mut tx = TempoTransactionRequest {
        inner: TransactionRequest {
            from: Some(fee_payer),
            to: Some(TxKind::Create),
            input: vec![0x60, 0x00].into(),
            ..Default::default()
        },
        ..Default::default()
    };

    assert_eq!(
        resolve_and_set_fee_token::<TempoNetwork>(
            Some(&provider),
            Some(Chain::from_named(NamedChain::Tempo)),
            &mut tx,
            Some(fee_payer),
        )
        .await?,
        None
    );
    assert_eq!(tx.fee_token, None);
    Ok(())
}

#[tokio::test]
async fn sponsor_fee_token_resolution_uses_sponsor_address() -> eyre::Result<()> {
    let asserter = Asserter::new();
    let provider =
        ProviderBuilder::new_with_network::<TempoNetwork>().connect_mocked_client(asserter.clone());
    let sponsor = TempoSponsor::new(Address::repeat_byte(0x22), None, None);
    let sender = Address::repeat_byte(0x11);
    let mut tx = TempoTransactionRequest {
        inner: TransactionRequest::default().with_from(sender),
        ..Default::default()
    };

    asserter.push_success(&BETA_USD_ADDRESS.abi_encode());

    assert_eq!(
        sponsor
            .resolve_and_set_fee_token::<TempoNetwork>(
                Some(&provider),
                Some(Chain::from_named(NamedChain::Tempo)),
                &mut tx,
            )
            .await?,
        Some(BETA_USD_ADDRESS)
    );
    assert_eq!(tx.fee_token, Some(BETA_USD_ADDRESS));
    Ok(())
}

#[tokio::test]
async fn sponsor_fee_token_resolution_preserves_explicit_token() -> eyre::Result<()> {
    let asserter = Asserter::new();
    let provider =
        ProviderBuilder::new_with_network::<TempoNetwork>().connect_mocked_client(asserter);
    let explicit = Address::repeat_byte(0x42);
    let sponsor = TempoSponsor::new(Address::repeat_byte(0x22), None, None);
    let mut tx = TempoTransactionRequest { fee_token: Some(explicit), ..Default::default() };

    assert_eq!(
        sponsor
            .resolve_and_set_fee_token::<TempoNetwork>(
                Some(&provider),
                Some(Chain::from_named(NamedChain::Tempo)),
                &mut tx,
            )
            .await?,
        Some(explicit)
    );
    assert_eq!(tx.fee_token, Some(explicit));
    Ok(())
}

#[tokio::test]
async fn distribute_reward_does_not_infer_fee_token() -> eyre::Result<()> {
    let mut tx = TempoTransactionRequest {
        inner: TransactionRequest::default()
            .with_to(ALPHA_USD_ADDRESS)
            .with_input(ITIP20::distributeRewardCall { amount: U256::from(1) }.abi_encode()),
        ..Default::default()
    };

    assert_eq!(
        resolve_and_set_fee_token::<TempoNetwork>(
            None,
            Some(Chain::from_named(NamedChain::Tempo)),
            &mut tx,
            None,
        )
        .await?,
        None
    );
    assert_eq!(tx.fee_token, None);
    Ok(())
}

#[tokio::test]
async fn tip20_transfer_calls_infer_called_token() -> eyre::Result<()> {
    for input in [
        ITIP20::transferCall { to: Address::repeat_byte(0x01), amount: U256::from(1) }.abi_encode(),
        ITIP20::transferWithMemoCall {
            to: Address::repeat_byte(0x01),
            amount: U256::from(1),
            memo: Default::default(),
        }
        .abi_encode(),
    ] {
        let mut tx = TempoTransactionRequest {
            inner: TransactionRequest::default().with_to(ALPHA_USD_ADDRESS).with_input(input),
            ..Default::default()
        };

        assert_eq!(
            resolve_and_set_fee_token::<TempoNetwork>(
                None,
                Some(Chain::from_named(NamedChain::Tempo)),
                &mut tx,
                None,
            )
            .await?,
            Some(ALPHA_USD_ADDRESS)
        );
        assert_eq!(tx.fee_token, Some(ALPHA_USD_ADDRESS));
    }
    Ok(())
}

#[tokio::test]
async fn sponsored_single_tip20_call_does_not_infer_called_token() -> eyre::Result<()> {
    let sender = Address::repeat_byte(0x11);
    let sponsor = Address::repeat_byte(0x22);
    let mut tx = TempoTransactionRequest {
        inner: TransactionRequest::default()
            .with_from(sender)
            .with_to(ALPHA_USD_ADDRESS)
            .with_input(
                ITIP20::transferCall { to: Address::repeat_byte(0x01), amount: U256::from(1) }
                    .abi_encode(),
            ),
        ..Default::default()
    };

    assert_eq!(
        resolve_and_set_fee_token::<TempoNetwork>(
            None,
            Some(Chain::from_named(NamedChain::Tempo)),
            &mut tx,
            Some(sponsor),
        )
        .await?,
        None
    );
    assert_eq!(tx.fee_token, None);
    Ok(())
}

#[tokio::test]
async fn non_matching_tip20_selectors_do_not_infer_fee_token() -> eyre::Result<()> {
    let mut tx = TempoTransactionRequest {
        inner: TransactionRequest::default().with_to(ALPHA_USD_ADDRESS).with_input(
            ITIP20::approveCall { spender: Address::repeat_byte(0x01), amount: U256::from(1) }
                .abi_encode(),
        ),
        ..Default::default()
    };

    assert_eq!(
        resolve_and_set_fee_token::<TempoNetwork>(
            None,
            Some(Chain::from_named(NamedChain::Tempo)),
            &mut tx,
            None,
        )
        .await?,
        None
    );
    assert_eq!(tx.fee_token, None);
    Ok(())
}

#[tokio::test]
async fn self_paid_set_user_token_overrides_stored_fee_token() -> eyre::Result<()> {
    let asserter = Asserter::new();
    let provider =
        ProviderBuilder::new_with_network::<TempoNetwork>().connect_mocked_client(asserter);
    let sender = Address::repeat_byte(0x11);
    let mut tx = TempoTransactionRequest {
        inner: TransactionRequest::default()
            .with_from(sender)
            .with_to(TIP_FEE_MANAGER_ADDRESS)
            .with_input(IFeeManager::setUserTokenCall { token: ALPHA_USD_ADDRESS }.abi_encode()),
        ..Default::default()
    };

    assert_eq!(
        resolve_and_set_fee_token::<TempoNetwork>(
            Some(&provider),
            Some(Chain::from_named(NamedChain::Tempo)),
            &mut tx,
            Some(sender),
        )
        .await?,
        Some(ALPHA_USD_ADDRESS)
    );
    assert_eq!(tx.fee_token, Some(ALPHA_USD_ADDRESS));
    Ok(())
}

#[tokio::test]
async fn aa_set_user_token_is_not_inferred() -> eyre::Result<()> {
    let sender = Address::repeat_byte(0x11);
    let mut tx = TempoTransactionRequest {
        inner: TransactionRequest::default()
            .with_from(sender)
            .with_to(TIP_FEE_MANAGER_ADDRESS)
            .with_input(IFeeManager::setUserTokenCall { token: ALPHA_USD_ADDRESS }.abi_encode()),
        nonce_key: Some(U256::from(7)),
        ..Default::default()
    };

    assert_eq!(
        resolve_and_set_fee_token::<TempoNetwork>(
            None,
            Some(Chain::from_named(NamedChain::Tempo)),
            &mut tx,
            Some(sender),
        )
        .await?,
        None
    );
    assert_eq!(tx.fee_token, None);
    Ok(())
}

#[tokio::test]
async fn tip20_batch_infers_only_when_calls_match_sender_and_token() -> eyre::Result<()> {
    let sender = Address::repeat_byte(0x11);
    let transfer =
        ITIP20::transferCall { to: Address::repeat_byte(0x01), amount: U256::from(1) }.abi_encode();
    let transfer_with_memo = ITIP20::transferWithMemoCall {
        to: Address::repeat_byte(0x01),
        amount: U256::from(1),
        memo: Default::default(),
    }
    .abi_encode();

    let mut tx = TempoTransactionRequest {
        inner: TransactionRequest::default().with_from(sender),
        calls: vec![
            tempo_call(ALPHA_USD_ADDRESS, transfer.clone()),
            tempo_call(ALPHA_USD_ADDRESS, transfer_with_memo.clone()),
        ],
        ..Default::default()
    };
    assert_eq!(
        resolve_and_set_fee_token::<TempoNetwork>(
            None,
            Some(Chain::from_named(NamedChain::Tempo)),
            &mut tx,
            Some(sender),
        )
        .await?,
        Some(ALPHA_USD_ADDRESS)
    );
    assert_eq!(tx.fee_token, Some(ALPHA_USD_ADDRESS));

    let mut tx = TempoTransactionRequest {
        inner: TransactionRequest::default().with_from(sender),
        calls: vec![
            tempo_call(ALPHA_USD_ADDRESS, transfer.clone()),
            tempo_call(BETA_USD_ADDRESS, transfer_with_memo.clone()),
        ],
        ..Default::default()
    };
    assert_eq!(
        resolve_and_set_fee_token::<TempoNetwork>(
            None,
            Some(Chain::from_named(NamedChain::Tempo)),
            &mut tx,
            Some(sender),
        )
        .await?,
        None
    );

    let mut tx = TempoTransactionRequest {
        inner: TransactionRequest::default().with_from(sender),
        calls: vec![
            tempo_call(ALPHA_USD_ADDRESS, transfer),
            tempo_call(ALPHA_USD_ADDRESS, vec![0xde, 0xad, 0xbe, 0xef]),
        ],
        ..Default::default()
    };
    assert_eq!(
        resolve_and_set_fee_token::<TempoNetwork>(
            None,
            Some(Chain::from_named(NamedChain::Tempo)),
            &mut tx,
            Some(sender),
        )
        .await?,
        None
    );

    let mut tx = TempoTransactionRequest {
        inner: TransactionRequest::default().with_from(sender),
        calls: vec![tempo_call(ALPHA_USD_ADDRESS, transfer_with_memo)],
        ..Default::default()
    };
    assert_eq!(
        resolve_and_set_fee_token::<TempoNetwork>(
            None,
            Some(Chain::from_named(NamedChain::Tempo)),
            &mut tx,
            Some(Address::repeat_byte(0x22)),
        )
        .await?,
        None
    );
    Ok(())
}

#[tokio::test]
async fn non_tip20_transfer_is_not_inferred() -> eyre::Result<()> {
    let sender = Address::repeat_byte(0x11);
    let erc20 = Address::repeat_byte(0xab);
    let mut tx = TempoTransactionRequest {
        inner: TransactionRequest::default().with_from(sender).with_to(erc20).with_input(
            ITIP20::transferCall { to: Address::repeat_byte(0x01), amount: U256::from(1) }
                .abi_encode(),
        ),
        ..Default::default()
    };

    assert_eq!(
        resolve_and_set_fee_token::<TempoNetwork>(
            None,
            Some(Chain::from_named(NamedChain::Tempo)),
            &mut tx,
            Some(sender),
        )
        .await?,
        None
    );
    assert_eq!(tx.fee_token, None);
    Ok(())
}

#[tokio::test]
async fn tempo_call_inspection_matches_built_aa_call_list() -> eyre::Result<()> {
    let transfer =
        ITIP20::transferCall { to: Address::repeat_byte(0x01), amount: U256::from(1) }.abi_encode();
    let transfer_with_memo = ITIP20::transferWithMemoCall {
        to: Address::repeat_byte(0x01),
        amount: U256::from(1),
        memo: Default::default(),
    }
    .abi_encode();
    let mut tx = TempoTransactionRequest {
        inner: TransactionRequest::default().with_to(ALPHA_USD_ADDRESS).with_input(transfer),
        calls: vec![tempo_call(ALPHA_USD_ADDRESS, transfer_with_memo)],
        ..Default::default()
    };

    let calls = tx.tempo_calls();
    assert_eq!(calls.len(), 2);
    assert!(calls.iter().all(|(to, _)| *to == TxKind::Call(ALPHA_USD_ADDRESS)));
    assert_eq!(
        resolve_and_set_fee_token::<TempoNetwork>(
            None,
            Some(Chain::from_named(NamedChain::Tempo)),
            &mut tx,
            None,
        )
        .await?,
        Some(ALPHA_USD_ADDRESS)
    );
    assert_eq!(tx.fee_token, Some(ALPHA_USD_ADDRESS));
    Ok(())
}

#[tokio::test]
async fn non_tip20_stablecoin_dex_token_in_is_not_inferred() -> eyre::Result<()> {
    let erc20 = Address::repeat_byte(0xab);
    let mut tx = TempoTransactionRequest {
        inner: TransactionRequest::default().with_to(STABLECOIN_DEX_ADDRESS).with_input(
            IStablecoinDEX::swapExactAmountInCall {
                tokenIn: erc20,
                tokenOut: BETA_USD_ADDRESS,
                amountIn: 1,
                minAmountOut: 1,
            }
            .abi_encode(),
        ),
        ..Default::default()
    };

    assert_eq!(
        resolve_and_set_fee_token::<TempoNetwork>(
            None,
            Some(Chain::from_named(NamedChain::Tempo)),
            &mut tx,
            None,
        )
        .await?,
        None
    );
    assert_eq!(tx.fee_token, None);
    Ok(())
}

#[tokio::test]
async fn stablecoin_dex_swaps_infer_token_in() -> eyre::Result<()> {
    for input in [
        IStablecoinDEX::swapExactAmountInCall {
            tokenIn: ALPHA_USD_ADDRESS,
            tokenOut: BETA_USD_ADDRESS,
            amountIn: 1,
            minAmountOut: 1,
        }
        .abi_encode(),
        IStablecoinDEX::swapExactAmountOutCall {
            tokenIn: ALPHA_USD_ADDRESS,
            tokenOut: BETA_USD_ADDRESS,
            amountOut: 1,
            maxAmountIn: 1,
        }
        .abi_encode(),
    ] {
        let mut tx = TempoTransactionRequest {
            inner: TransactionRequest::default().with_to(STABLECOIN_DEX_ADDRESS).with_input(input),
            ..Default::default()
        };

        assert_eq!(
            resolve_and_set_fee_token::<TempoNetwork>(
                None,
                Some(Chain::from_named(NamedChain::Tempo)),
                &mut tx,
                None,
            )
            .await?,
            Some(ALPHA_USD_ADDRESS)
        );
        assert_eq!(tx.fee_token, Some(ALPHA_USD_ADDRESS));
    }
    Ok(())
}

#[tokio::test]
async fn stablecoin_dex_batch_inference_requires_one_call() -> eyre::Result<()> {
    let input = IStablecoinDEX::swapExactAmountInCall {
        tokenIn: ALPHA_USD_ADDRESS,
        tokenOut: BETA_USD_ADDRESS,
        amountIn: 1,
        minAmountOut: 1,
    }
    .abi_encode();

    let mut tx = TempoTransactionRequest {
        calls: vec![tempo_call(STABLECOIN_DEX_ADDRESS, input.clone())],
        ..Default::default()
    };
    assert_eq!(
        resolve_and_set_fee_token::<TempoNetwork>(
            None,
            Some(Chain::from_named(NamedChain::Tempo)),
            &mut tx,
            None,
        )
        .await?,
        Some(ALPHA_USD_ADDRESS)
    );

    let mut tx = TempoTransactionRequest {
        calls: vec![
            tempo_call(STABLECOIN_DEX_ADDRESS, input.clone()),
            tempo_call(STABLECOIN_DEX_ADDRESS, input),
        ],
        ..Default::default()
    };
    assert_eq!(
        resolve_and_set_fee_token::<TempoNetwork>(
            None,
            Some(Chain::from_named(NamedChain::Tempo)),
            &mut tx,
            None,
        )
        .await?,
        None
    );
    assert_eq!(tx.fee_token, None);
    Ok(())
}

#[tokio::test]
async fn stored_fee_token_overrides_inferred_fee_token() -> eyre::Result<()> {
    let asserter = Asserter::new();
    let provider =
        ProviderBuilder::new_with_network::<TempoNetwork>().connect_mocked_client(asserter.clone());
    let fee_payer = Address::repeat_byte(0x11);
    let mut tx = TempoTransactionRequest {
        inner: TransactionRequest::default()
            .with_from(fee_payer)
            .with_to(ALPHA_USD_ADDRESS)
            .with_input(
                ITIP20::transferCall { to: Address::repeat_byte(0x01), amount: U256::from(1) }
                    .abi_encode(),
            ),
        ..Default::default()
    };

    asserter.push_success(&BETA_USD_ADDRESS.abi_encode());
    assert_eq!(
        resolve_and_set_fee_token::<TempoNetwork>(
            Some(&provider),
            Some(Chain::from_named(NamedChain::Tempo)),
            &mut tx,
            Some(fee_payer),
        )
        .await?,
        Some(BETA_USD_ADDRESS)
    );
    assert_eq!(tx.fee_token, Some(BETA_USD_ADDRESS));
    Ok(())
}

#[tokio::test]
async fn non_tempo_chains_do_not_infer_fee_token() -> eyre::Result<()> {
    let mut tx = TempoTransactionRequest {
        inner: TransactionRequest::default().with_to(ALPHA_USD_ADDRESS).with_input(
            ITIP20::transferCall { to: Address::repeat_byte(0x01), amount: U256::from(1) }
                .abi_encode(),
        ),
        ..Default::default()
    };

    assert_eq!(
        resolve_and_set_fee_token::<TempoNetwork>(
            None,
            Some(Chain::from_named(NamedChain::Mainnet)),
            &mut tx,
            None,
        )
        .await?,
        None
    );
    assert_eq!(tx.fee_token, None);
    Ok(())
}

fn tempo_call(to: Address, input: Vec<u8>) -> Call {
    Call { to: TxKind::Call(to), value: U256::ZERO, input: input.into() }
}

#[test]
fn resolves_known_fee_token_symbols() {
    for (fee_token, symbol) in [
        (PATH_USD_ADDRESS, "PathUSD"),
        (ALPHA_USD_ADDRESS, "AlphaUSD"),
        (BETA_USD_ADDRESS, "BetaUSD"),
        (THETA_USD_ADDRESS, "ThetaUSD"),
    ] {
        assert_eq!(known_fee_token_symbol(fee_token), Some(symbol));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn resolves_fee_token_symbol_from_tempo_mainnet() -> eyre::Result<()> {
    let provider = ProviderBuilder::new_with_network::<TempoNetwork>()
        .connect_http("https://rpc.tempo.xyz".parse()?);
    let valid_fee_token = address!("0x20C00000000000000000000014f22CA97301EB73");

    assert_eq!(
        resolve_fee_token_symbol(&provider, valid_fee_token).await.as_deref(),
        Some("USDT0")
    );

    // Non-existent fee token should not cause an error, but return None
    let invalid_fee_token = address!("0x20C0000000000000000000000000000000000004");
    assert_eq!(resolve_fee_token_symbol(&provider, invalid_fee_token).await.as_deref(), None);
    Ok(())
}
