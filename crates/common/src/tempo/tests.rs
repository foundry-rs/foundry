use alloy_provider::{Provider, ProviderBuilder};
use eyre::WrapErr;
use foundry_evm_hardforks::TempoHardfork;
use serde::Deserialize;
use std::env;
use tempo_alloy::contracts::precompiles::DEFAULT_FEE_TOKEN;

use alloy_chains::{Chain, NamedChain};
use alloy_network::TransactionBuilder;
use alloy_primitives::{Address, address};
use alloy_provider::mock::Asserter;
use alloy_rpc_types::TransactionRequest;
use alloy_sol_types::SolValue;
use tempo_alloy::{TempoNetwork, rpc::TempoTransactionRequest};

use super::{
    ALPHA_USD_ADDRESS, BETA_USD_ADDRESS, PATH_USD_ADDRESS, THETA_USD_ADDRESS, TempoSponsor,
    known_fee_token_symbol, resolve_and_set_fee_token, resolve_fee_token, resolve_fee_token_symbol,
};

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
async fn resolves_canonical_fee_token_for_tempo_chains() -> eyre::Result<()> {
    let asserter = Asserter::new();
    let provider =
        ProviderBuilder::new_with_network::<TempoNetwork>().connect_mocked_client(asserter.clone());
    let tx = TempoTransactionRequest {
        inner: TransactionRequest::default().with_from(Address::repeat_byte(0x11)),
        ..Default::default()
    };

    for chain in [
        NamedChain::Tempo,
        NamedChain::TempoModerato,
        NamedChain::TempoTestnet,
        NamedChain::TempoDevnet,
    ] {
        asserter.push_success(&Address::ZERO.abi_encode());
        assert_eq!(
            resolve_fee_token::<TempoNetwork>(&provider, Some(chain.into()), Some(&tx), None)
                .await?,
            Some(DEFAULT_FEE_TOKEN)
        );
    }
    Ok(())
}

#[tokio::test]
async fn leaves_non_tempo_chains_without_a_default() -> eyre::Result<()> {
    let asserter = Asserter::new();
    let provider =
        ProviderBuilder::new_with_network::<TempoNetwork>().connect_mocked_client(asserter);
    let tx = TempoTransactionRequest::default();

    assert_eq!(
        resolve_fee_token::<TempoNetwork>(
            &provider,
            Some(NamedChain::Mainnet.into()),
            Some(&tx),
            None
        )
        .await?,
        None
    );
    Ok(())
}

#[tokio::test]
async fn leaves_unknown_chain_without_a_default() -> eyre::Result<()> {
    let asserter = Asserter::new();
    let provider =
        ProviderBuilder::new_with_network::<TempoNetwork>().connect_mocked_client(asserter);

    assert_eq!(resolve_fee_token::<TempoNetwork>(&provider, None, None, None).await?, None);
    Ok(())
}

#[tokio::test]
async fn explicit_fee_token_overrides_chain_default() -> eyre::Result<()> {
    let asserter = Asserter::new();
    let provider =
        ProviderBuilder::new_with_network::<TempoNetwork>().connect_mocked_client(asserter);
    let explicit = Address::repeat_byte(0x42);
    let tx = TempoTransactionRequest { fee_token: Some(explicit), ..Default::default() };

    assert_eq!(
        resolve_fee_token::<TempoNetwork>(
            &provider,
            Some(Chain::from_named(NamedChain::Tempo)),
            Some(&tx),
            None
        )
        .await?,
        Some(explicit)
    );
    assert_eq!(
        resolve_fee_token::<TempoNetwork>(&provider, None, Some(&tx), None).await?,
        Some(explicit)
    );
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
async fn stored_user_token_takes_priority_before_default() -> eyre::Result<()> {
    let asserter = Asserter::new();
    let provider =
        ProviderBuilder::new_with_network::<TempoNetwork>().connect_mocked_client(asserter.clone());
    let fee_payer = Address::repeat_byte(0x11);
    let tx = TempoTransactionRequest {
        inner: TransactionRequest::default().with_from(fee_payer),
        ..Default::default()
    };

    asserter.push_success(&BETA_USD_ADDRESS.abi_encode());

    assert_eq!(
        resolve_fee_token::<TempoNetwork>(
            &provider,
            Some(Chain::from_named(NamedChain::Tempo)),
            Some(&tx),
            None
        )
        .await?,
        Some(BETA_USD_ADDRESS)
    );
    Ok(())
}

#[tokio::test]
async fn unset_user_token_falls_back_to_default() -> eyre::Result<()> {
    let asserter = Asserter::new();
    let provider =
        ProviderBuilder::new_with_network::<TempoNetwork>().connect_mocked_client(asserter.clone());
    let tx = TempoTransactionRequest {
        inner: TransactionRequest::default().with_from(Address::repeat_byte(0x11)),
        ..Default::default()
    };

    asserter.push_success(&Address::ZERO.abi_encode());

    assert_eq!(
        resolve_fee_token::<TempoNetwork>(
            &provider,
            Some(Chain::from_named(NamedChain::Tempo)),
            Some(&tx),
            None
        )
        .await?,
        Some(DEFAULT_FEE_TOKEN)
    );
    Ok(())
}

#[tokio::test]
async fn fee_token_lookup_rpc_error_is_propagated() {
    let asserter = Asserter::new();
    let provider =
        ProviderBuilder::new_with_network::<TempoNetwork>().connect_mocked_client(asserter.clone());
    let tx = TempoTransactionRequest {
        inner: TransactionRequest::default().with_from(Address::repeat_byte(0x11)),
        ..Default::default()
    };

    asserter.push_failure_msg("user token lookup failed");

    assert!(
        resolve_fee_token::<TempoNetwork>(
            &provider,
            Some(Chain::from_named(NamedChain::Tempo)),
            Some(&tx),
            None
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn fee_token_lookup_decode_error_is_propagated() {
    let asserter = Asserter::new();
    let provider =
        ProviderBuilder::new_with_network::<TempoNetwork>().connect_mocked_client(asserter.clone());
    let tx = TempoTransactionRequest {
        inner: TransactionRequest::default().with_from(Address::repeat_byte(0x11)),
        ..Default::default()
    };

    let malformed = vec![0u8; 1];
    asserter.push_success(&malformed);

    assert!(
        resolve_fee_token::<TempoNetwork>(
            &provider,
            Some(Chain::from_named(NamedChain::Tempo)),
            Some(&tx),
            None
        )
        .await
        .is_err()
    );
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
