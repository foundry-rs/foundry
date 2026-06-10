#![allow(clippy::too_many_arguments)]

use crate::tx::{SendTxOpts, TxParams};
use alloy_ens::NameOrAddress;
use alloy_network::{Network, TransactionBuilder};
use alloy_primitives::B256;
use alloy_provider::Provider;
use alloy_rpc_types::TransactionInputKind;
use alloy_sol_types::{SolCall, SolError};
use alloy_transport::{RpcError, TransportErrorKind};
use foundry_cli::utils::LoadConfig;
use foundry_common::provider::ProviderBuilder;
use tempo_alloy::TempoNetwork;
use tempo_contracts::precompiles::{
    TIP20_FACTORY_ADDRESS, UnknownFunctionSelector, createTokenWithLogoCall, is_iso4217_currency,
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
) -> eyre::Result<()> {
    if let Some(logo_uri) = logo_uri.as_deref() {
        super::logo::validate_logo_uri(logo_uri)?;
    }

    let (signer, tempo_access_key) = super::resolve_tip20_signer(&send_tx, &tx_opts).await?;

    let config = send_tx.eth.rpc.load_config()?;

    if !is_iso4217_currency(&currency) && !force {
        sh_warn!("{}", super::iso4217_warning_message(&currency))?;
        let response: String = foundry_common::prompt!("\nContinue anyway? [y/N] ")?;
        if !matches!(response.trim(), "y" | "Y") {
            sh_status!("Aborted.")?;
            return Ok(());
        }
    }

    let provider = ProviderBuilder::<TempoNetwork>::from_config(&config)?.build()?;
    let quote_token_addr = quote_token.resolve(&provider).await?;
    let admin_addr = admin.resolve(&provider).await?;

    let (sig, mut args) = match logo_uri {
        Some(logo_uri) => {
            let tx = create_logo_call_request(createTokenWithLogoCall {
                name: name.clone(),
                symbol: symbol.clone(),
                currency: currency.clone(),
                quoteToken: quote_token_addr,
                admin: admin_addr,
                salt,
                logoURI: logo_uri.clone(),
            });
            ensure_t5_create_logo_supported(&provider, &tx).await?;
            (
                "createToken(string,string,string,address,address,bytes32,string)",
                vec![
                    name,
                    symbol,
                    currency,
                    quote_token_addr.to_string(),
                    admin_addr.to_string(),
                    salt.to_string(),
                    logo_uri,
                ],
            )
        }
        None => (
            "createToken(string,string,string,address,address,bytes32)",
            vec![
                name,
                symbol,
                currency,
                quote_token_addr.to_string(),
                admin_addr.to_string(),
                salt.to_string(),
            ],
        ),
    };
    super::send_tip20_transaction(
        NameOrAddress::Address(TIP20_FACTORY_ADDRESS),
        sig,
        std::mem::take(&mut args),
        send_tx,
        tx_opts,
        signer,
        tempo_access_key,
    )
    .await?;

    Ok(())
}

fn create_logo_call_request(
    call: createTokenWithLogoCall,
) -> <TempoNetwork as Network>::TransactionRequest {
    let mut tx = <TempoNetwork as Network>::TransactionRequest::default();
    tx.set_kind(TIP20_FACTORY_ADDRESS.into());
    tx.set_input_kind(call.abi_encode(), TransactionInputKind::Both);
    tx
}

async fn ensure_t5_create_logo_supported<P>(
    provider: &P,
    tx: &<TempoNetwork as Network>::TransactionRequest,
) -> eyre::Result<()>
where
    P: Provider<TempoNetwork>,
{
    match provider.call(tx.clone()).await {
        Ok(_) => Ok(()),
        Err(err) if is_t5_create_logo_unknown_selector(&err) => {
            eyre::bail!(
                "--logo-uri requires a T5-compatible TIP20Factory; the configured RPC rejected the 7-arg createToken selector 0x5323d222"
            )
        }
        Err(_) => Ok(()),
    }
}

fn is_t5_create_logo_unknown_selector(err: &RpcError<TransportErrorKind>) -> bool {
    let Some(data) = err
        .as_error_resp()
        .and_then(|error| error.data.as_ref())
        .and_then(|data| serde_json::from_str::<alloy_primitives::Bytes>(data.get()).ok())
    else {
        return false;
    };

    is_t5_create_logo_unknown_selector_data(data.as_ref())
}

fn is_t5_create_logo_unknown_selector_data(data: &[u8]) -> bool {
    data == UnknownFunctionSelector { selector: createTokenWithLogoCall::SELECTOR.into() }
        .abi_encode()
        .as_slice()
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{address, b256, bytes};
    use tempo_contracts::precompiles::createTokenCall;

    #[test]
    fn legacy_create_selector_is_preserved() {
        let calldata = createTokenCall {
            name: "US Dollar Coin".to_string(),
            symbol: "USDC".to_string(),
            currency: "USD".to_string(),
            quoteToken: address!("0000000000000000000000000000000000000001"),
            admin: address!("0000000000000000000000000000000000000002"),
            salt: b256!("0000000000000000000000000000000000000000000000000000000000000003"),
        }
        .abi_encode();

        assert_eq!(&calldata[..4], bytes!("68130445").as_ref());
    }

    #[test]
    fn t5_create_selector_includes_logo_uri_overload() {
        let calldata = createTokenWithLogoCall {
            name: "US Dollar Coin".to_string(),
            symbol: "USDC".to_string(),
            currency: "USD".to_string(),
            quoteToken: address!("0000000000000000000000000000000000000001"),
            admin: address!("0000000000000000000000000000000000000002"),
            salt: b256!("0000000000000000000000000000000000000000000000000000000000000003"),
            logoURI: "https://example.com/logo.png".to_string(),
        }
        .abi_encode();

        assert_ne!(&calldata[..4], bytes!("68130445").as_ref());
        assert_eq!(&calldata[..4], createTokenWithLogoCall::SELECTOR.as_ref());
    }

    #[test]
    fn detects_t5_create_logo_unknown_selector_revert_data() {
        let data = UnknownFunctionSelector { selector: createTokenWithLogoCall::SELECTOR.into() }
            .abi_encode();

        assert!(is_t5_create_logo_unknown_selector_data(&data));
    }
}
