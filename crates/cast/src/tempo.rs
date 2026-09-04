//! Tempo transaction helpers used by Cast-facing commands.

use crate::tx::fill_transaction_gas_fees;
use alloy_network::{Ethereum, Network, TransactionBuilder};
use alloy_primitives::{Address, B256};
use alloy_provider::Provider;
use alloy_rpc_client::BuiltInConnectionString;
use alloy_transport::{BoxTransport, TransportConnect, TransportError};
use eyre::Result;
use foundry_cli::{
    json::print_json_success,
    opts::{EthereumOpts, TempoOpts},
    utils::{LoadConfig, get_chain},
};
use foundry_common::{
    FoundryTransactionBuilder,
    provider::{ProviderBuilder, RetryProvider, is_rpc_method_not_found},
    shell,
    tempo::{maybe_print_fee_token, resolve_and_set_fee_token},
};
use foundry_config::{Chain, Config, Eip1559FeeEstimatePreset};
use foundry_evm::hardfork::TempoHardfork;
use foundry_wallets::{TempoAccountsWallet, WalletOpts, WalletSigner};
use serde::Deserialize;
use serde_json::Value;
use std::str::FromStr;
use tempo_alloy::{
    TempoNetwork,
    provider::TempoProviderExt,
    transport::{RelayConnector, SponsorshipMode},
};

pub use foundry_common::tempo::TempoSponsor;

/// Loads the config for `opts` and builds a Tempo provider from it.
pub(crate) fn tempo_provider(
    opts: &impl LoadConfig,
) -> Result<(Config, RetryProvider<TempoNetwork>)> {
    let config = opts.load_config()?;
    let provider = ProviderBuilder::<TempoNetwork>::from_config(&config)?.build()?;
    Ok((config, provider))
}

/// Attaches the fee payment to a built transaction: the sponsor signature when a sponsor is
/// configured, otherwise the resolved fee token for `payer` (printing it when it was resolved).
pub(crate) async fn apply_fee_payment<N, P>(
    sponsor: Option<&TempoSponsor>,
    provider: Option<&P>,
    chain: Chain,
    tx: &mut N::TransactionRequest,
    payer: Address,
) -> Result<()>
where
    N: Network,
    N::TransactionRequest: Default + FoundryTransactionBuilder<N>,
    P: Provider<N>,
{
    if sponsor.is_some() {
        maybe_attach_sponsor(sponsor, provider, chain, tx, payer).await
    } else {
        resolve_and_print_fee_token(provider, Some(chain), tx, Some(payer)).await
    }
}

/// Resolves the sponsored fee token and attaches the sponsor signature preview for `payer` when a
/// sponsor is configured.
pub(crate) async fn maybe_attach_sponsor<N, P>(
    sponsor: Option<&TempoSponsor>,
    provider: Option<&P>,
    chain: Chain,
    tx: &mut N::TransactionRequest,
    payer: Address,
) -> Result<()>
where
    N: Network,
    N::TransactionRequest: Default + FoundryTransactionBuilder<N>,
    P: Provider<N>,
{
    if let Some(sponsor) = sponsor {
        let provider = provider.map(|p| p as &dyn Provider<N>);
        sponsor.resolve_and_set_fee_token(provider, Some(chain), tx).await?;
        sponsor.attach_and_print::<N>(tx, payer).await?;
    }
    Ok(())
}

/// Resolves and sets the fee token paid by `fee_payer`, printing it when it was resolved.
pub(crate) async fn resolve_and_print_fee_token<N, P>(
    provider: Option<&P>,
    chain: Option<Chain>,
    tx: &mut N::TransactionRequest,
    fee_payer: Option<Address>,
) -> Result<()>
where
    N: Network,
    N::TransactionRequest: Default + FoundryTransactionBuilder<N>,
    P: Provider<N>,
{
    let dyn_provider = provider.map(|p| p as &dyn Provider<N>);
    let fee_token = resolve_and_set_fee_token(dyn_provider, chain, tx, fee_payer).await?;
    maybe_print_fee_token(provider, fee_token).await
}

/// Computes the sponsor hash of a built transaction, resolving the fee token for `fee_payer` first
/// when one is configured. Used by `--tempo.print-sponsor-hash`.
pub(crate) async fn sponsor_hash<N, P>(
    provider: Option<&P>,
    chain: Chain,
    tx: &mut N::TransactionRequest,
    from: Address,
    fee_payer: Option<Address>,
) -> Result<B256>
where
    N: Network,
    N::TransactionRequest: Default + FoundryTransactionBuilder<N>,
    P: Provider<N>,
{
    if fee_payer.is_some() {
        let provider = provider.map(|p| p as &dyn Provider<N>);
        resolve_and_set_fee_token(provider, Some(chain), tx, fee_payer).await?;
    }
    tx.compute_sponsor_hash(from)
        .ok_or_else(|| eyre::eyre!("This network does not support sponsored transactions"))
}

/// Prints a command result: the raw payload in JSON mode, the human rendering otherwise.
pub(crate) fn print_payload<F>(payload: Value, human: F) -> Result<()>
where
    F: FnOnce(&Value) -> Result<()>,
{
    if shell::is_json() {
        print_json_success(payload)?;
    } else {
        human(&payload)?;
    }
    Ok(())
}

pub(crate) fn print_expires(expires_at: Option<u64>) -> Result<()> {
    if let Some(ts) = expires_at {
        sh_status!("Transaction expires at unix timestamp {ts}")?;
    }
    Ok(())
}

/// Resolves a command signer, preferring an explicitly selected Tempo session.
///
/// Session resolution is fail-closed: when `--tempo.session` or `TEMPO_SESSION_ID` is set, wallet
/// signer options are rejected by [`TempoOpts::session_signer_for_wallet`] instead of falling back
/// to a long-lived signer.
pub(crate) async fn resolve_session_or_wallet_signer(
    tempo: &TempoOpts,
    wallet: &WalletOpts,
    chain_id: u64,
) -> Result<(Option<WalletSigner>, Option<TempoAccountsWallet>)> {
    match tempo.session_signer_for_wallet(wallet, chain_id)? {
        Some(session) => Ok((None, Some(session.access_key))),
        None => wallet.maybe_signer_for_chain(chain_id).await,
    }
}

pub(crate) fn ensure_session_not_browser(tempo: &TempoOpts, browser: bool) -> Result<()> {
    if browser && tempo.session_id()?.is_some() {
        eyre::bail!("--tempo.session/TEMPO_SESSION_ID cannot be combined with --browser");
    }
    Ok(())
}

/// Fails with `message` unless `hardfork` is active on the RPC.
pub(crate) async fn require_hardfork<P: Provider<TempoNetwork>>(
    provider: &P,
    hardfork: TempoHardfork,
    message: &str,
) -> Result<()> {
    if !is_tempo_hardfork_active(provider, hardfork).await? {
        eyre::bail!("{message}");
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AnvilNodeInfo {
    hard_fork: Option<String>,
    network: Option<String>,
}

pub(crate) async fn is_tempo_hardfork_active<P: Provider<TempoNetwork>>(
    provider: &P,
    hardfork: TempoHardfork,
) -> Result<bool> {
    match provider.is_hardfork_active(hardfork).await {
        Ok(active) => Ok(active),
        Err(err) if is_rpc_method_not_found(&err) => {
            match anvil_tempo_hardfork_active(provider, hardfork).await {
                Ok(Some(active)) => Ok(active),
                _ => Err(err.into()),
            }
        }
        Err(err) => Err(err.into()),
    }
}

/// Fails early with `requirement` when a Tempo precompile is not active yet: a pre-fork call
/// would succeed as a silent no-op instead of reverting. Prefers the hardfork query and falls
/// back to checking the precompile's code when the RPC lacks the method.
pub(crate) async fn ensure_tempo_precompile_active<P: Provider<TempoNetwork>>(
    provider: &P,
    hardfork: TempoHardfork,
    precompile: Address,
    requirement: &str,
) -> Result<()> {
    let active = match is_tempo_hardfork_active(provider, hardfork).await {
        Ok(active) => active,
        Err(_) => !provider.get_code_at(precompile).await?.is_empty(),
    };
    eyre::ensure!(active, "{requirement}");
    Ok(())
}

async fn anvil_tempo_hardfork_active<P: Provider<TempoNetwork>>(
    provider: &P,
    hardfork: TempoHardfork,
) -> Result<Option<bool>, TransportError> {
    let info = provider.raw_request::<_, AnvilNodeInfo>("anvil_nodeInfo".into(), ()).await?;
    Ok(active_from_anvil_node_info(&info, hardfork))
}

fn active_from_anvil_node_info(info: &AnvilNodeInfo, hardfork: TempoHardfork) -> Option<bool> {
    (info.network.as_deref() == Some("tempo")).then(|| {
        info.hard_fork
            .as_deref()
            .and_then(|active_hardfork| active_hardfork.parse::<TempoHardfork>().ok())
            .is_some_and(|active_hardfork| active_hardfork >= hardfork)
    })
}

/// Connector for reusing an already-configured RPC transport.
///
/// This preserves Foundry transport behavior such as MPP payment handling when a sponsor relay is
/// layered over the default RPC.
#[derive(Clone, Debug)]
pub(crate) struct ExistingTransportConnector {
    transport: BoxTransport,
    is_local: bool,
}

impl TransportConnect for ExistingTransportConnector {
    fn is_local(&self) -> bool {
        self.is_local
    }

    async fn get_transport(&self) -> Result<BoxTransport, TransportError> {
        Ok(self.transport.clone())
    }
}

pub(crate) fn sponsor_relay_connector<N: Network>(
    provider: &impl Provider<N>,
    sponsor_url: &str,
) -> Result<RelayConnector<ExistingTransportConnector, BuiltInConnectionString>> {
    let default = ExistingTransportConnector {
        transport: provider.client().transport().clone(),
        is_local: provider.client().is_local(),
    };
    let relay = BuiltInConnectionString::from_str(sponsor_url)?;
    Ok(RelayConnector::with_config(default, relay, SponsorshipMode::SignOnly, false))
}

/// Resolves the transaction network and any configured signer without letting an unrelated Tempo
/// Accounts store change ordinary Ethereum commands.
///
/// Explicit signer options are resolved with `from` cleared so the store fallback is not consulted.
/// The fallback is only enabled when a Tempo transaction option is present or the RPC chain is a
/// known Tempo chain.
pub(crate) async fn resolve_transaction_network_and_signer(
    tempo: &TempoOpts,
    eth: &EthereumOpts,
) -> Result<(bool, Option<WalletSigner>, Option<TempoAccountsWallet>)> {
    let mut explicit_wallet = eth.wallet.clone();
    explicit_wallet.from = None;
    let (signer, access_key) = explicit_wallet.maybe_signer().await?;

    if access_key.is_some() {
        return Ok((true, signer, access_key));
    }

    if tempo.is_tempo() {
        if signer.is_some() || eth.wallet.from.is_none() {
            return Ok((true, signer, None));
        }
        let (signer, access_key) = eth.wallet.maybe_signer().await?;
        return Ok((true, signer, access_key));
    }

    if signer.is_some() || eth.wallet.from.is_none() {
        return Ok((false, signer, None));
    }

    let config = eth.load_config()?;
    let provider = ProviderBuilder::<Ethereum>::from_config(&config)?.build()?;
    let chain = get_chain(config.chain, &provider).await?;
    if !chain.is_tempo() {
        return Ok((false, None, None));
    }

    let (signer, access_key) = eth.wallet.maybe_signer_for_chain(chain.id()).await?;
    Ok((true, signer, access_key))
}

/// Fills a Tempo transaction request that was built outside [`crate::tx::CastTxBuilder`] before
/// access-key signing.
pub(crate) async fn fill_access_key_transaction<P>(
    provider: &P,
    tx: &mut <TempoNetwork as Network>::TransactionRequest,
    access_key: &TempoAccountsWallet,
    chain: Chain,
    eip1559_fee_estimate: Eip1559FeeEstimatePreset,
) -> Result<TempoAccountsWallet>
where
    P: Provider<TempoNetwork>,
{
    tx.set_chain_id(chain.id());
    let prepared = access_key.prepare_request(provider, tx).await?;

    if tx.nonce().is_none() {
        tx.set_nonce(provider.get_transaction_count(prepared.account()).await?);
    }
    fill_transaction_gas_fees(provider, tx, chain.is_legacy(), false, eip1559_fee_estimate).await?;
    if tx.gas_limit().is_none() {
        tx.set_gas_limit(provider.estimate_gas(tx.clone()).await?);
    }

    Ok(prepared)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_provider::{ProviderBuilder as AlloyProviderBuilder, mock::Asserter};
    use alloy_rpc_client::RpcClient;

    #[test]
    fn active_from_anvil_node_info_requires_tempo_network() {
        let info = |network: &str, hard_fork: &str| AnvilNodeInfo {
            network: Some(network.to_string()),
            hard_fork: Some(hard_fork.to_string()),
        };
        let tempo_t3 = info("tempo", "T3");
        assert_eq!(active_from_anvil_node_info(&tempo_t3, TempoHardfork::T2), Some(true));
        assert_eq!(active_from_anvil_node_info(&tempo_t3, TempoHardfork::T3), Some(true));
        assert_eq!(active_from_anvil_node_info(&tempo_t3, TempoHardfork::T4), Some(false));
        assert_eq!(
            active_from_anvil_node_info(&info("tempo", "T11"), TempoHardfork::T11),
            Some(true)
        );
        assert_eq!(active_from_anvil_node_info(&info("ethereum", "T3"), TempoHardfork::T3), None);
    }

    #[tokio::test]
    async fn sponsor_relay_reuses_existing_default_transport() {
        let asserter = Asserter::new();
        asserter.push_success(&alloy_primitives::U64::from(42));
        let provider = AlloyProviderBuilder::new().connect_mocked_client(asserter);
        let connector =
            sponsor_relay_connector(&provider, "http://127.0.0.1:1").expect("valid relay");
        let transport = connector.get_transport().await.expect("relay transport");
        let client = RpcClient::builder().transport(transport, true);
        let relayed = AlloyProviderBuilder::new().connect_client(client);

        assert_eq!(relayed.get_block_number().await.unwrap(), 42);
    }
}
