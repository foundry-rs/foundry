use alloy_provider::{Provider, ProviderBuilder};
use eyre::WrapErr;
use foundry_evm_hardforks::TempoHardfork;
use serde::Deserialize;
use std::env;

use crate::FoundryTransactionBuilder;
use alloy_network::{Ethereum, TransactionBuilder};
use alloy_primitives::{Address, B256, Bytes, TxKind, U256, address};
use alloy_provider::mock::Asserter;
use alloy_rpc_types::TransactionRequest;
use alloy_signer::SignerSync;
use alloy_signer_local::PrivateKeySigner;
use alloy_sol_types::{SolCall, SolValue};
use foundry_wallets::WalletOpts;
use tempo_alloy::{
    TempoNetwork,
    accounts::TempoAccountsError,
    contracts::precompiles::{IFeeManager, IStablecoinDEX, ITIP20, STABLECOIN_DEX_ADDRESS},
    rpc::TempoTransactionRequest,
};
use tempo_primitives::transaction::{
    Call, KeyAuthorization, PrimitiveSignature, SignatureType, SignedKeyAuthorization,
    tt_signature::P256SignatureWithPreHash,
};

use super::{
    ALPHA_USD_ADDRESS, BETA_USD_ADDRESS, OUSD_ADDRESS, PATH_USD_ADDRESS, THETA_USD_ADDRESS,
    TIP_FEE_MANAGER_ADDRESS, TempoSponsor, known_fee_token_symbol, maybe_signer,
    resolve_and_set_fee_token, resolve_fee_token_symbol, verify_browser_key_authorization,
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
        resolve_and_set_fee_token::<TempoNetwork>(Some(&provider), &mut tx, Some(fee_payer),)
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

    let resolved = resolve_and_set_fee_token::<TempoNetwork>(None, &mut tx, None).await?;
    assert_eq!(resolved, Some(explicit));
    assert_eq!(tx.fee_token, Some(explicit));

    let mut tx = TempoTransactionRequest::default();
    let resolved = resolve_and_set_fee_token::<TempoNetwork>(None, &mut tx, None).await?;
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
        resolve_and_set_fee_token::<TempoNetwork>(Some(&provider), &mut tx, Some(fee_payer),)
            .await?,
        Some(BETA_USD_ADDRESS)
    );
    assert_eq!(tx.fee_token, Some(BETA_USD_ADDRESS));

    let mut tx = TempoTransactionRequest::default();
    assert_eq!(
        resolve_and_set_fee_token::<TempoNetwork>(None, &mut tx, Some(fee_payer),).await?,
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
        resolve_and_set_fee_token::<TempoNetwork>(Some(&provider), &mut tx, Some(fee_payer),)
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
        resolve_and_set_fee_token::<TempoNetwork>(Some(&provider), &mut tx, Some(fee_payer),)
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
        sponsor.resolve_and_set_fee_token::<TempoNetwork>(Some(&provider), &mut tx,).await?,
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
        sponsor.resolve_and_set_fee_token::<TempoNetwork>(Some(&provider), &mut tx,).await?,
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
            .with_input(ITIP20::distributeRewardCall { amount: U256::ONE }.abi_encode()),
        ..Default::default()
    };

    assert_eq!(resolve_and_set_fee_token::<TempoNetwork>(None, &mut tx, None,).await?, None);
    assert_eq!(tx.fee_token, None);
    Ok(())
}

#[tokio::test]
async fn tip20_transfer_calls_infer_called_token() -> eyre::Result<()> {
    for input in [
        ITIP20::transferCall { to: Address::repeat_byte(0x01), amount: U256::ONE }.abi_encode(),
        ITIP20::transferWithMemoCall {
            to: Address::repeat_byte(0x01),
            amount: U256::ONE,
            memo: Default::default(),
        }
        .abi_encode(),
    ] {
        let mut tx = TempoTransactionRequest {
            inner: TransactionRequest::default().with_to(ALPHA_USD_ADDRESS).with_input(input),
            ..Default::default()
        };

        assert_eq!(
            resolve_and_set_fee_token::<TempoNetwork>(None, &mut tx, None,).await?,
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
                ITIP20::transferCall { to: Address::repeat_byte(0x01), amount: U256::ONE }
                    .abi_encode(),
            ),
        ..Default::default()
    };

    assert_eq!(
        resolve_and_set_fee_token::<TempoNetwork>(None, &mut tx, Some(sponsor),).await?,
        None
    );
    assert_eq!(tx.fee_token, None);
    Ok(())
}

#[tokio::test]
async fn non_matching_tip20_selectors_do_not_infer_fee_token() -> eyre::Result<()> {
    let mut tx = TempoTransactionRequest {
        inner: TransactionRequest::default().with_to(ALPHA_USD_ADDRESS).with_input(
            ITIP20::approveCall { spender: Address::repeat_byte(0x01), amount: U256::ONE }
                .abi_encode(),
        ),
        ..Default::default()
    };

    assert_eq!(resolve_and_set_fee_token::<TempoNetwork>(None, &mut tx, None,).await?, None);
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
        resolve_and_set_fee_token::<TempoNetwork>(Some(&provider), &mut tx, Some(sender),).await?,
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
        resolve_and_set_fee_token::<TempoNetwork>(None, &mut tx, Some(sender),).await?,
        None
    );
    assert_eq!(tx.fee_token, None);
    Ok(())
}

#[tokio::test]
async fn tip20_batch_infers_only_when_calls_match_sender_and_token() -> eyre::Result<()> {
    let sender = Address::repeat_byte(0x11);
    let transfer =
        ITIP20::transferCall { to: Address::repeat_byte(0x01), amount: U256::ONE }.abi_encode();
    let transfer_with_memo = ITIP20::transferWithMemoCall {
        to: Address::repeat_byte(0x01),
        amount: U256::ONE,
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
        resolve_and_set_fee_token::<TempoNetwork>(None, &mut tx, Some(sender),).await?,
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
        resolve_and_set_fee_token::<TempoNetwork>(None, &mut tx, Some(sender),).await?,
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
        resolve_and_set_fee_token::<TempoNetwork>(None, &mut tx, Some(sender),).await?,
        None
    );

    let mut tx = TempoTransactionRequest {
        inner: TransactionRequest::default().with_from(sender),
        calls: vec![tempo_call(ALPHA_USD_ADDRESS, transfer_with_memo)],
        ..Default::default()
    };
    assert_eq!(
        resolve_and_set_fee_token::<TempoNetwork>(None, &mut tx, Some(Address::repeat_byte(0x22)),)
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
            ITIP20::transferCall { to: Address::repeat_byte(0x01), amount: U256::ONE }.abi_encode(),
        ),
        ..Default::default()
    };

    assert_eq!(
        resolve_and_set_fee_token::<TempoNetwork>(None, &mut tx, Some(sender),).await?,
        None
    );
    assert_eq!(tx.fee_token, None);
    Ok(())
}

#[tokio::test]
async fn tempo_call_inspection_matches_built_aa_call_list() -> eyre::Result<()> {
    let transfer =
        ITIP20::transferCall { to: Address::repeat_byte(0x01), amount: U256::ONE }.abi_encode();
    let transfer_with_memo = ITIP20::transferWithMemoCall {
        to: Address::repeat_byte(0x01),
        amount: U256::ONE,
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
        resolve_and_set_fee_token::<TempoNetwork>(None, &mut tx, None,).await?,
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

    assert_eq!(resolve_and_set_fee_token::<TempoNetwork>(None, &mut tx, None,).await?, None);
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
            resolve_and_set_fee_token::<TempoNetwork>(None, &mut tx, None,).await?,
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
        resolve_and_set_fee_token::<TempoNetwork>(None, &mut tx, None,).await?,
        Some(ALPHA_USD_ADDRESS)
    );

    let mut tx = TempoTransactionRequest {
        calls: vec![
            tempo_call(STABLECOIN_DEX_ADDRESS, input.clone()),
            tempo_call(STABLECOIN_DEX_ADDRESS, input),
        ],
        ..Default::default()
    };
    assert_eq!(resolve_and_set_fee_token::<TempoNetwork>(None, &mut tx, None,).await?, None);
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
                ITIP20::transferCall { to: Address::repeat_byte(0x01), amount: U256::ONE }
                    .abi_encode(),
            ),
        ..Default::default()
    };

    asserter.push_success(&BETA_USD_ADDRESS.abi_encode());
    assert_eq!(
        resolve_and_set_fee_token::<TempoNetwork>(Some(&provider), &mut tx, Some(fee_payer),)
            .await?,
        Some(BETA_USD_ADDRESS)
    );
    assert_eq!(tx.fee_token, Some(BETA_USD_ADDRESS));
    Ok(())
}

// No responses are queued, so a fee-token lookup would fail this test.
#[tokio::test]
async fn non_tempo_requests_do_not_resolve_fee_token() -> eyre::Result<()> {
    let provider = ProviderBuilder::new().connect_mocked_client(Asserter::new());
    let fee_payer = Address::repeat_byte(0x11);
    let mut tx =
        TransactionRequest::default().with_from(fee_payer).with_to(ALPHA_USD_ADDRESS).with_input(
            ITIP20::transferCall { to: Address::repeat_byte(0x01), amount: U256::ONE }.abi_encode(),
        );

    assert_eq!(
        resolve_and_set_fee_token::<Ethereum>(Some(&provider), &mut tx, Some(fee_payer)).await?,
        None
    );
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
        (OUSD_ADDRESS, "OUSD"),
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

/// Signs `authorization` with `signer` and RLP-encodes the result, as the browser wallet returns
/// it.
fn browser_signed(authorization: &KeyAuthorization, signer: &PrivateKeySigner) -> Vec<u8> {
    let signature = signer.sign_hash_sync(&authorization.signature_hash()).unwrap();
    alloy_rlp::encode(authorization.clone().into_signed(PrimitiveSignature::Secp256k1(signature)))
}

fn root_signer() -> PrivateKeySigner {
    "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80".parse().unwrap()
}

#[test]
fn browser_key_authorization_accepts_root_signature() {
    let root = root_signer();
    let requested =
        KeyAuthorization::unrestricted(1, SignatureType::Secp256k1, Address::repeat_byte(0xbb))
            .with_expiry(123);

    let signed = verify_browser_key_authorization(
        &browser_signed(&requested, &root),
        &requested,
        root.address(),
    )
    .unwrap();

    assert_eq!(signed.authorization, requested);
}

#[test]
fn browser_key_authorization_rejects_mutated_payload() {
    let root = root_signer();
    let key_id = Address::repeat_byte(0xbb);
    let requested =
        KeyAuthorization::unrestricted(1, SignatureType::Secp256k1, key_id).with_expiry(123);
    let mutated =
        KeyAuthorization::unrestricted(1, SignatureType::Secp256k1, key_id).with_expiry(456);

    let err = verify_browser_key_authorization(
        &browser_signed(&mutated, &root),
        &requested,
        root.address(),
    )
    .unwrap_err();

    assert_eq!(
        err.to_string(),
        format!(
            "wallet returned a mutated KeyAuthorization payload: signed digest {} but requested {}",
            mutated.signature_hash(),
            requested.signature_hash(),
        )
    );
}

#[test]
fn browser_key_authorization_rejects_other_signer() {
    let root = root_signer();
    let other: PrivateKeySigner =
        "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d".parse().unwrap();
    let requested =
        KeyAuthorization::unrestricted(1, SignatureType::Secp256k1, Address::repeat_byte(0xbb));

    let err = verify_browser_key_authorization(
        &browser_signed(&requested, &other),
        &requested,
        root.address(),
    )
    .unwrap_err();

    assert_eq!(
        err.to_string(),
        "wallet returned a SignedKeyAuthorization signed by \
         0x70997970C51812dc3A010C7d01b50e0d17dc79C8 but the connected root account is \
         0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
    );
}

#[test]
fn browser_key_authorization_rejects_unrecoverable_signature() {
    let requested =
        KeyAuthorization::unrestricted(1, SignatureType::P256, Address::repeat_byte(0xbb));
    let signed: SignedKeyAuthorization =
        requested.clone().into_signed(PrimitiveSignature::P256(P256SignatureWithPreHash {
            r: B256::ZERO,
            s: B256::ZERO,
            pub_key_x: B256::ZERO,
            pub_key_y: B256::ZERO,
            pre_hash: false,
        }));

    let err = verify_browser_key_authorization(
        &alloy_rlp::encode(signed),
        &requested,
        root_signer().address(),
    )
    .unwrap_err();

    assert_eq!(
        err.to_string(),
        "wallet returned an unrecoverable SignedKeyAuthorization signature"
    );
}

#[test]
fn browser_key_authorization_rejects_invalid_rlp() {
    let requested =
        KeyAuthorization::unrestricted(1, SignatureType::Secp256k1, Address::repeat_byte(0xbb));

    let err =
        verify_browser_key_authorization(&[0xf8, 0x01, 0x02], &requested, root_signer().address())
            .unwrap_err();

    assert_eq!(err.to_string(), "invalid SignedKeyAuthorization RLP from wallet");
}

#[tokio::test]
async fn maybe_signer_pins_access_key_wallet_to_chain() {
    let root = Address::repeat_byte(0x11);
    let wallet = WalletOpts {
        tempo_access_key: Some(
            "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d".to_owned(),
        ),
        tempo_root_account: Some(root),
        ..Default::default()
    };

    let (signer, pinned) = maybe_signer(&wallet, Some(4217)).await.unwrap();
    assert!(signer.is_none());
    let key = pinned.unwrap().active_access_key().unwrap();
    assert_eq!(key.account(), root);
    assert_eq!(key.chain_id(), 4217);
    assert_eq!(key.address(), address!("0x70997970C51812dc3A010C7d01b50e0d17dc79C8"));

    let (_, unpinned) = maybe_signer(&wallet, None).await.unwrap();
    assert!(matches!(
        unpinned.unwrap().active_access_key(),
        Err(TempoAccountsError::MissingChainId)
    ));
}
