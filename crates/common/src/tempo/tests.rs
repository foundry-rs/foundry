use alloy_provider::{Provider, ProviderBuilder};
use eyre::WrapErr;
use foundry_evm_hardforks::TempoHardfork;
use serde::Deserialize;
use std::env;
use tempo_alloy::contracts::precompiles::DEFAULT_FEE_TOKEN;

use alloy_chains::{Chain, NamedChain};
use alloy_primitives::Address;

use super::{
    ALPHA_USD_ADDRESS, BETA_USD_ADDRESS, PATH_USD_ADDRESS, THETA_USD_ADDRESS,
    format_fee_token_selection, known_fee_token_symbol, resolve_fee_token,
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

#[test]
fn resolves_canonical_fee_token_for_tempo_chains() {
    for chain in [
        NamedChain::Tempo,
        NamedChain::TempoModerato,
        NamedChain::TempoTestnet,
        NamedChain::TempoDevnet,
    ] {
        assert_eq!(resolve_fee_token(Some(chain.into()), None), Some(DEFAULT_FEE_TOKEN));
    }
}

#[test]
fn leaves_non_tempo_chains_without_a_default() {
    assert_eq!(resolve_fee_token(Some(NamedChain::Mainnet.into()), None), None);
}

#[test]
fn leaves_unknown_chain_without_a_default() {
    assert_eq!(resolve_fee_token(None, None), None);
}

#[test]
fn explicit_fee_token_overrides_chain_default() {
    let explicit = Address::repeat_byte(0x42);
    assert_eq!(
        resolve_fee_token(Some(Chain::from_named(NamedChain::Tempo)), Some(explicit)),
        Some(explicit)
    );
    assert_eq!(resolve_fee_token(None, Some(explicit)), Some(explicit));
}

#[test]
fn formats_known_fee_token_selection_with_label() {
    for (fee_token, symbol) in [
        (PATH_USD_ADDRESS, "PathUSD"),
        (ALPHA_USD_ADDRESS, "AlphaUSD"),
        (BETA_USD_ADDRESS, "BetaUSD"),
        (THETA_USD_ADDRESS, "ThetaUSD"),
    ] {
        assert_eq!(known_fee_token_symbol(fee_token), Some(symbol));
        assert_eq!(
            format_fee_token_selection(fee_token),
            format!("Paying gas in {symbol} ({fee_token})")
        );
    }
}

#[test]
fn formats_unknown_fee_token_selection_as_address() {
    let fee_token = Address::repeat_byte(0x42);
    assert_eq!(known_fee_token_symbol(fee_token), None);
    assert_eq!(format_fee_token_selection(fee_token), format!("Paying gas in {fee_token}"));
}
