use crate::{
    cmd::confirm_continue,
    tempo::tempo_provider,
    tx::{SendTxOpts, TxParams},
};
use alloy_ens::NameOrAddress;
use alloy_network::{Network, TransactionBuilder};
use alloy_primitives::{B256, Bytes};
use alloy_provider::Provider;
use alloy_rpc_types::TransactionInputKind;
use alloy_sol_types::{SolCall, SolError};
use eyre::Result;
use tempo_alloy::TempoNetwork;
use tempo_contracts::precompiles::{
    TIP20_FACTORY_ADDRESS, UnknownFunctionSelector, createTokenCall, createTokenWithLogoCall,
    is_iso4217_currency,
};

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
    logo_uri: Option<String>,
    force: bool,
    send_tx: SendTxOpts,
    tx_opts: TxParams,
) -> Result<()> {
    if let Some(logo_uri) = logo_uri.as_deref() {
        super::logo::validate_logo_uri(logo_uri)?;
    }

    if !is_iso4217_currency(&currency) && !force {
        sh_warn!("{}", iso4217_warning_message(&currency))?;
        if !confirm_continue()? {
            return Ok(());
        }
    }

    let (_, provider) = tempo_provider(&send_tx.eth.rpc)?;
    let quote_token = quote_token.resolve(&provider).await?;
    let admin = admin.resolve(&provider).await?;

    let data = match logo_uri {
        Some(logo_uri) => {
            let call = createTokenWithLogoCall {
                name,
                symbol,
                currency,
                quoteToken: quote_token,
                admin,
                salt,
                logoURI: logo_uri,
            };
            ensure_t5_create_logo_supported(&provider, &call).await?;
            call.abi_encode()
        }
        None => createTokenCall { name, symbol, currency, quoteToken: quote_token, admin, salt }
            .abi_encode(),
    };
    super::send_tip20_transaction(TIP20_FACTORY_ADDRESS, data, send_tx, tx_opts).await
}

/// Fails early when the factory rejects the 7-arg `createToken` selector, which only T5+
/// factories implement.
async fn ensure_t5_create_logo_supported<P: Provider<TempoNetwork>>(
    provider: &P,
    call: &createTokenWithLogoCall,
) -> Result<()> {
    let mut tx = <TempoNetwork as Network>::TransactionRequest::default();
    tx.set_kind(TIP20_FACTORY_ADDRESS.into());
    tx.set_input_kind(call.abi_encode(), TransactionInputKind::Both);

    let unknown_selector =
        UnknownFunctionSelector { selector: createTokenWithLogoCall::SELECTOR.into() }.abi_encode();
    if let Err(err) = provider.call(tx).await
        && let Some(data) = err.as_error_resp().and_then(|resp| resp.data.as_ref())
        && serde_json::from_str::<Bytes>(data.get()).is_ok_and(|data| data == unknown_selector)
    {
        eyre::bail!(
            "--logo-uri requires a T5-compatible TIP20Factory; the configured RPC rejected the 7-arg createToken selector 0x5323d222"
        );
    }
    Ok(())
}
