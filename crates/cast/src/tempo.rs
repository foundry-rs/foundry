//! Tempo transaction helpers used by Cast-facing commands.

use crate::tx::fill_transaction_gas_fees;
use alloy_network::{Network, TransactionBuilder};
use alloy_provider::Provider;
use eyre::Result;
use foundry_cli::{json::print_json_success, opts::TempoOpts};
use foundry_common::{FoundryTransactionBuilder, shell};
use foundry_config::{Chain, Eip1559FeeEstimatePreset};
use foundry_wallets::{TempoAccessKeyConfig, WalletOpts, WalletSigner};
use serde_json::Value;
use tempo_alloy::TempoNetwork;

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
) -> Result<(Option<WalletSigner>, Option<TempoAccessKeyConfig>)> {
    match tempo.session_signer_for_wallet(wallet, chain_id)? {
        Some(session) => Ok((Some(session.signer), Some(session.access_key))),
        None => wallet.maybe_signer().await,
    }
}

pub(crate) fn ensure_session_not_browser(tempo: &TempoOpts, browser: bool) -> Result<()> {
    if browser && tempo.session_id()?.is_some() {
        eyre::bail!("--tempo.session/TEMPO_SESSION_ID cannot be combined with --browser");
    }
    Ok(())
}

/// Fills a Tempo transaction request that was built outside [`crate::tx::CastTxBuilder`] before
/// access-key signing.
pub(crate) async fn fill_access_key_transaction<P>(
    provider: &P,
    tx: &mut <TempoNetwork as Network>::TransactionRequest,
    access_key: &TempoAccessKeyConfig,
    chain: Chain,
    eip1559_fee_estimate: Eip1559FeeEstimatePreset,
) -> Result<()>
where
    P: Provider<TempoNetwork>,
{
    tx.set_from(access_key.wallet_address);
    tx.set_chain_id(chain.id());
    tx.set_key_id(access_key.key_address);
    tx.prepare_access_key_authorization(
        provider,
        access_key.wallet_address,
        access_key.key_address,
        access_key.key_authorization.as_ref(),
    )
    .await?;

    if tx.nonce().is_none() {
        tx.set_nonce(provider.get_transaction_count(access_key.wallet_address).await?);
    }
    fill_transaction_gas_fees(provider, tx, chain.is_legacy(), false, eip1559_fee_estimate).await?;
    if tx.gas_limit().is_none() {
        tx.set_gas_limit(provider.estimate_gas(tx.clone()).await?);
    }

    Ok(())
}
