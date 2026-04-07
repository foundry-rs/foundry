use alloy_primitives::Address;
use alloy_provider::Provider;
use tempo_alloy::{TempoNetwork, provider::TempoProviderExt};

pub use tempo_contracts::precompiles::is_iso4217_currency;

/// Checks whether an access key is already provisioned on-chain.
///
/// Queries the AccountKeychain precompile's `getKey` function. A key is considered
/// provisioned if the returned `keyId` is non-zero (i.e. the key exists and has not
/// been revoked).
pub async fn is_key_provisioned<P: Provider<TempoNetwork>>(
    provider: &P,
    wallet_address: Address,
    key_address: Address,
) -> bool {
    match provider.get_keychain_key(wallet_address, key_address).await {
        Ok(info) => info.keyId != Address::ZERO,
        Err(_) => false,
    }
}

/// Returns a warning message for non-ISO 4217 currency codes used in TIP-20 token creation.
pub fn iso4217_warning_message(currency: &str) -> String {
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
