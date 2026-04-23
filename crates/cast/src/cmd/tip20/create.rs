use crate::{
    cmd::{
        erc20::build_provider_with_signer,
        send::{cast_send, cast_send_with_access_key},
    },
    tx::{SendTxOpts, TxParams},
};
use alloy_ens::NameOrAddress;
use alloy_primitives::B256;
use alloy_sol_types::sol;
use foundry_cli::utils::{LoadConfig, get_chain};
use foundry_common::provider::ProviderBuilder;
use tempo_alloy::TempoNetwork;
use tempo_contracts::precompiles::{TIP20_FACTORY_ADDRESS, is_iso4217_currency};

sol! {
    #[sol(rpc)]
    interface ITIP20Factory {
        function createToken(
            string memory name,
            string memory symbol,
            string memory currency,
            address quoteToken,
            address admin,
            bytes32 salt
        ) external returns (address token);
    }
}

/// Returns a warning message for non-ISO 4217 currency codes used in TIP-20 token creation.
pub(crate) fn iso4217_warning_message(currency: &str) -> String {
    let hyperlink = |url: &str| format!("\x1b]8;;{url}\x1b\\{url}\x1b]8;;\x1b\\");
    let tip20_docs = hyperlink("https://docs.tempo.xyz/protocol/tip20/overview");
    let iso_docs = hyperlink("https://www.iso.org/iso-4217-currency-codes.html");

    format!(
        "\"{currency}\" is not a recognized ISO 4217 currency code.\n\
         \n\
         If the token you are trying to deploy is a fiat-backed stablecoin, Tempo strongly\n\
         recommends that the currency code field be the ISO-4217 currency code of the fiat\n\
         currency your token tracks (e.g. \"USD\", \"EUR\", \"GBP\").\n\
         \n\
         The currency field is IMMUTABLE after token creation and affects fee payment\n\
         eligibility, DEX routing, and quote token pairing. Only \"USD\"-denominated tokens\n\
         can be used to pay transaction fees on Tempo.\n\
         \n\
         Learn more:\n  \
         - Tempo TIP-20 docs: {tip20_docs}\n  \
         - ISO 4217 standard: {iso_docs}"
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn run(
    name: String,
    symbol: String,
    currency: String,
    quote_token: NameOrAddress,
    admin: NameOrAddress,
    salt: B256,
    force: bool,
    send_tx: SendTxOpts,
    tx_opts: TxParams,
) -> eyre::Result<()> {
    let (signer, tempo_access_key) = if send_tx.eth.wallet.from.is_some() {
        send_tx.eth.wallet.maybe_signer().await?
    } else {
        (None, None)
    };

    let config = send_tx.eth.rpc.load_config()?;

    if !is_iso4217_currency(&currency) && !force {
        sh_warn!("{}", super::iso4217_warning_message(&currency))?;
        let response: String = foundry_common::prompt!("\nContinue anyway? [y/N] ")?;
        if !matches!(response.trim(), "y" | "Y") {
            sh_println!("Aborted.")?;
            return Ok(());
        }
    }

    let timeout = send_tx.timeout.unwrap_or(config.transaction_timeout);
    let provider = ProviderBuilder::<TempoNetwork>::from_config(&config)?.build()?;
    let quote_token_addr = quote_token.resolve(&provider).await?;
    let admin_addr = admin.resolve(&provider).await?;

    let mut tx = ITIP20Factory::new(TIP20_FACTORY_ADDRESS, &provider)
        .createToken(name, symbol, currency, quote_token_addr, admin_addr, salt)
        .into_transaction_request();

    tx_opts.apply::<TempoNetwork>(&mut tx, get_chain(config.chain, &provider).await?.is_legacy());

    if let Some(ref access_key) = tempo_access_key {
        let signer = signer.as_ref().ok_or_else(|| eyre::eyre!("access key requires a signer"))?;
        cast_send_with_access_key(
            &provider,
            tx,
            signer,
            access_key,
            send_tx.cast_async,
            send_tx.confirmations,
            timeout,
        )
        .await?;
    } else {
        let signer = signer.unwrap_or(send_tx.eth.wallet.signer().await?);
        let provider = build_provider_with_signer::<TempoNetwork>(&send_tx, signer)?;
        cast_send(provider, tx, send_tx.cast_async, send_tx.sync, send_tx.confirmations, timeout)
            .await?;
    }

    Ok(())
}
