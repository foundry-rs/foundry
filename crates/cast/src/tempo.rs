//! Tempo transaction helpers used by Cast-facing commands.

use crate::tx::fill_transaction_gas_fees;
use alloy_network::{Ethereum, Network, TransactionBuilder};
use alloy_provider::Provider;
use alloy_rpc_client::BuiltInConnectionString;
use alloy_transport::{BoxTransport, TransportConnect, TransportError};
use eyre::Result;
use foundry_cli::{
    json::print_json_success,
    opts::{EthereumOpts, TempoOpts},
    utils::{LoadConfig, get_chain},
};
use foundry_common::{provider::ProviderBuilder, shell};
use foundry_config::{Chain, Eip1559FeeEstimatePreset};
use foundry_wallets::{TempoAccountsWallet, WalletOpts, WalletSigner};
use serde_json::Value;
use std::str::FromStr;
use tempo_alloy::{
    TempoNetwork,
    transport::{RelayConnector, SponsorshipMode},
};

pub use foundry_common::tempo::{TempoSponsor, TempoSponsorPreview, resolve_tempo_sponsor_signer};

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
