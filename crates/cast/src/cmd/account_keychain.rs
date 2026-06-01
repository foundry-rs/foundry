//! Shared helpers for AccountKeychain precompile transactions.

use std::time::Duration;

use alloy_ens::NameOrAddress;
use alloy_network::{EthereumWallet, TransactionBuilder};
use alloy_primitives::{Address, hex};
use alloy_provider::{Provider, ProviderBuilder as AlloyProviderBuilder};
use alloy_signer::Signer;
use foundry_cli::{
    opts::TransactionOpts,
    utils::{LoadConfig, maybe_print_resolved_lane, resolve_lane},
};
use foundry_common::{
    FoundryTransactionBuilder, provider::ProviderBuilder, shell, tempo::TEMPO_BROWSER_GAS_BUFFER,
};
use tempo_alloy::TempoNetwork;
use tempo_contracts::precompiles::ACCOUNT_KEYCHAIN_ADDRESS;

use crate::{
    cmd::send::cast_send,
    tx::{CastTxBuilder, CastTxSender, SendTxOpts},
};

/// Send calldata to the Tempo AccountKeychain precompile as a root-authorized transaction.
pub(crate) async fn send_account_keychain_tx(
    calldata: Vec<u8>,
    tx_opts: TransactionOpts,
    send_tx: &SendTxOpts,
) -> eyre::Result<()> {
    send_account_keychain_tx_inner(calldata, tx_opts, send_tx, None).await
}

/// Send calldata to AccountKeychain, requiring the resolved root signer to match `expected`.
pub(crate) async fn send_account_keychain_tx_from(
    calldata: Vec<u8>,
    tx_opts: TransactionOpts,
    send_tx: &SendTxOpts,
    expected: Address,
) -> eyre::Result<()> {
    send_account_keychain_tx_inner(calldata, tx_opts, send_tx, Some(expected)).await
}

async fn send_account_keychain_tx_inner(
    calldata: Vec<u8>,
    mut tx_opts: TransactionOpts,
    send_tx: &SendTxOpts,
    expected_from: Option<Address>,
) -> eyre::Result<()> {
    let (signer, tempo_access_key) = send_tx.eth.wallet.maybe_signer().await?;
    let print_sponsor_hash = tx_opts.tempo.print_sponsor_hash;
    let expires_at = tx_opts.tempo.resolve_expires();
    let tempo_sponsor =
        if print_sponsor_hash { None } else { tx_opts.tempo.sponsor_config().await? };

    let config = send_tx.eth.load_config()?;
    let timeout = send_tx.timeout.unwrap_or(config.transaction_timeout);
    let provider = ProviderBuilder::<TempoNetwork>::from_config(&config)?.build()?;

    if let Some(interval) = send_tx.poll_interval {
        provider.client().set_poll_interval(Duration::from_secs(interval));
    }

    // Resolve `--tempo.lane <name>` against the lanes file (default
    // `<root>/tempo.lanes.toml`) and populate `tx_opts.tempo.nonce_key` from the lane.
    let resolved_lane = resolve_lane(&mut tx_opts.tempo, &config.root)?;

    let builder = CastTxBuilder::new(&provider, tx_opts, &config)
        .await?
        .with_to(Some(NameOrAddress::Address(ACCOUNT_KEYCHAIN_ADDRESS)))
        .await?
        .with_code_sig_and_args(None, Some(hex::encode_prefixed(&calldata)), vec![])
        .await?;

    // AccountKeychain management calls are authorized by the root account. Access keys can use
    // their permissions, but cannot mutate their own key policy.
    let browser = send_tx.browser.run::<TempoNetwork>().await?;

    if print_sponsor_hash {
        let from = if let Some(ref browser) = browser {
            browser.address()
        } else {
            signer
                .as_ref()
                .ok_or_else(|| {
                    eyre::eyre!(
                        "--tempo.print-sponsor-hash requires a root account signer, such as \
                         --browser, --private-key, or --keystore"
                    )
                })?
                .address()
        };
        ensure_root_sender(from, expected_from)?;

        let (tx, _) = builder.build(from).await?;
        let hash = tx
            .compute_sponsor_hash(from)
            .ok_or_else(|| eyre::eyre!("This network does not support sponsored transactions"))?;
        if shell::is_json() {
            sh_println!("{}", serde_json::json!({ "sponsor_hash": format!("{hash:?}") }))?;
        } else {
            sh_println!("{hash:?}")?;
        }
        return Ok(());
    }

    crate::tempo::print_expires(expires_at)?;

    if let Some(browser) = browser {
        ensure_root_sender(browser.address(), expected_from)?;
        let chain = builder.chain();
        let (mut tx, _) = builder.build(browser.address()).await?;
        if chain.is_tempo()
            && let Some(gas) = tx.gas_limit()
        {
            tx.set_gas_limit(gas + TEMPO_BROWSER_GAS_BUFFER);
        }
        if let Some(sponsor) = &tempo_sponsor {
            sponsor.attach_and_print::<TempoNetwork>(&mut tx, browser.address()).await?;
        }

        let tx_hash = browser.send_transaction_via_browser(tx).await?;
        CastTxSender::new(&provider)
            .print_tx_result(tx_hash, send_tx.cast_async, send_tx.confirmations, timeout)
            .await?;
    } else if tempo_access_key.is_some() {
        eyre::bail!(
            "keychain policy changes must be signed by the root account; the selected `--from` \
             resolved to a Tempo access key. Use `--browser` for passkey roots, or pass a root \
             account signer with `--private-key`, `--keystore`, Ledger, Trezor, AWS, GCP, or Turnkey."
        );
    } else {
        let signer = match signer {
            Some(s) => s,
            None => send_tx.eth.wallet.signer().await?,
        };
        let from = signer.address();
        ensure_root_sender(from, expected_from)?;
        let (mut tx, _) = builder.build(from).await?;
        maybe_print_resolved_lane(resolved_lane.as_ref(), tx.nonce().unwrap_or_default())?;
        if let Some(sponsor) = &tempo_sponsor {
            sponsor.attach_and_print::<TempoNetwork>(&mut tx, from).await?;
        }

        let wallet = EthereumWallet::from(signer);
        let provider = AlloyProviderBuilder::<_, _, TempoNetwork>::default()
            .wallet(wallet)
            .connect_provider(&provider);

        cast_send(provider, tx, send_tx.cast_async, send_tx.sync, send_tx.confirmations, timeout)
            .await?;
    }

    Ok(())
}

fn ensure_root_sender(actual: Address, expected: Option<Address>) -> eyre::Result<()> {
    if let Some(expected) = expected
        && actual != expected
    {
        eyre::bail!(
            "AccountKeychain transaction must be signed by root account {expected}; resolved signer is {actual}"
        );
    }
    Ok(())
}
