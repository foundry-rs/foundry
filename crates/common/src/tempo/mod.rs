//! Tempo network utilities.

pub mod auth;

use crate::FoundryTransactionBuilder;
use alloy_chains::Chain;
use alloy_network::{Network, NetworkTransactionBuilder, TransactionBuilder};
use alloy_primitives::{Address, B256, Signature, address};
use alloy_provider::Provider;
use alloy_signer::Signer;
use alloy_sol_types::SolCall;
use eyre::{Context, Result};
use foundry_wallets::{RawWalletOpts, WalletOpts, WalletSigner};
use std::sync::Arc;
pub use tempo_alloy::contracts::precompiles::PATH_USD_ADDRESS;
use tempo_alloy::contracts::precompiles::{DEFAULT_FEE_TOKEN, ITIP20};

mod keystore;
mod registry;
mod session;
mod session_policy;
#[cfg(test)]
mod test_utils;
mod tip20;

pub(crate) use auth::is_known_tempo_endpoint;
pub use auth::{AccessKeyOutcome, EnsureAccessKeyConfig, ensure_access_key};
pub use keystore::*;
pub use session::*;
pub use session_policy::{
    GeneratedSessionKey, PreparedSessionAuthorization, SessionAuthorizationRequest,
    SessionSpendLimit,
};
pub use tip20::{
    TIP20_ALLOWED_LOGO_URI_SCHEMES, TIP20_MAX_LOGO_URI_BYTES, Tip20LogoUriValidationError,
    validate_tip20_logo_uri,
};

#[cfg(test)]
pub(crate) use test_utils::{test_env_mutex, with_tempo_home};

#[cfg(test)]
mod tests;

/// Placeholder rendered by `Debug` impls in place of secret key material.
fn redacted_debug(value: &str) -> &'static str {
    if value.trim().is_empty() { "<empty>" } else { "<redacted>" }
}

/// Conservative gas buffer for browser wallet transactions on Tempo chains.
///
/// Browser wallets may sign with P256 or WebAuthn instead of secp256k1, which costs more gas
/// for signature verification. Since we can't determine the signature type before signing,
/// we add the worst-case (WebAuthn) overhead:
///   - P256: +5,000 gas (P256 precompile cost minus ecrecover savings)
///   - WebAuthn: ~6,500 gas (P256 cost + calldata for webauthn_data)
///
/// See <https://github.com/tempoxyz/tempo/blob/6ebf1a8/crates/revm/src/handler.rs#L108-L124>
pub const TEMPO_BROWSER_GAS_BUFFER: u64 = 7_000;

/// Reserved Tempo TIP20 fee-token addresses created during Foundry genesis.
///
/// Unlike [`PATH_USD_ADDRESS`], these tokens are not defined by the canonical
/// `tempo-contracts` crate; they only exist in Foundry's local genesis setup, so
/// they are defined here as the single source of truth and re-exported elsewhere.
pub const ALPHA_USD_ADDRESS: Address = address!("0x20C0000000000000000000000000000000000001");
pub const BETA_USD_ADDRESS: Address = address!("0x20C0000000000000000000000000000000000002");
pub const THETA_USD_ADDRESS: Address = address!("0x20C0000000000000000000000000000000000003");

/// Resolves an explicit Tempo fee token or the canonical default for a known Tempo network.
///
/// TODO: fee token resolution is incomplete, must use FeeTokenManager.
pub fn resolve_fee_token(
    chain: Option<Chain>,
    explicit_fee_token: Option<Address>,
) -> Option<Address> {
    explicit_fee_token.or_else(|| chain.is_some_and(Chain::is_tempo).then_some(DEFAULT_FEE_TOKEN))
}

/// Returns the known symbol for a Tempo fee token without making an RPC call.
pub const fn known_fee_token_symbol(fee_token: Address) -> Option<&'static str> {
    match fee_token {
        PATH_USD_ADDRESS => Some("PathUSD"),
        ALPHA_USD_ADDRESS => Some("AlphaUSD"),
        BETA_USD_ADDRESS => Some("BetaUSD"),
        THETA_USD_ADDRESS => Some("ThetaUSD"),
        _ => None,
    }
}

async fn resolve_fee_token_symbol<N, P>(provider: &P, fee_token: Address) -> Option<String>
where
    N: Network,
    N::TransactionRequest: Default + NetworkTransactionBuilder<N>,
    P: Provider<N>,
{
    if let Some(symbol) = known_fee_token_symbol(fee_token) {
        return Some(symbol.to_string());
    }

    let tx = N::TransactionRequest::default()
        .with_to(fee_token)
        .with_input(ITIP20::symbolCall.abi_encode());
    let output = provider.call(tx).await.ok()?;
    let symbol = ITIP20::symbolCall::abi_decode_returns(&output).ok()?;
    (!symbol.is_empty()).then_some(symbol)
}

/// Prints the selected Tempo fee token when one is set.
///
/// Unknown symbols are resolved on-chain only when a provider is supplied, because some provider
/// modes such as `--curl` must preserve the first RPC request for the user's intended action.
pub async fn maybe_print_fee_token<N, P>(
    provider: Option<&P>,
    fee_token: Option<Address>,
) -> Result<()>
where
    N: Network,
    N::TransactionRequest: Default + NetworkTransactionBuilder<N>,
    P: Provider<N>,
{
    if let Some(fee_token) = fee_token {
        let symbol = if let Some(symbol) = known_fee_token_symbol(fee_token) {
            Some(symbol.to_string())
        } else if let Some(provider) = provider {
            resolve_fee_token_symbol(provider, fee_token).await
        } else {
            None
        };
        match symbol {
            Some(symbol) => sh_status!("Paying gas in {} ({})", symbol, fee_token)?,
            None => sh_status!("Paying gas in {}", fee_token)?,
        }
    }
    Ok(())
}

/// Prints the fee token selected for display, resolving the chain default and unknown symbols
/// without mutating a transaction request.
pub async fn maybe_print_resolved_fee_token<N, P>(
    provider: Option<&P>,
    chain: Option<Chain>,
    fee_token: Option<Address>,
) -> Result<()>
where
    N: Network,
    N::TransactionRequest: Default + NetworkTransactionBuilder<N>,
    P: Provider<N>,
{
    maybe_print_fee_token(provider, resolve_fee_token(chain, fee_token)).await
}

/// Gas sponsor configuration for Tempo fee-payer signatures.
#[derive(Clone, Debug)]
pub struct TempoSponsor {
    sponsor: Address,
    signer: Option<Arc<WalletSigner>>,
    signature: Option<Signature>,
}

impl TempoSponsor {
    pub const fn new(
        sponsor: Address,
        signer: Option<Arc<WalletSigner>>,
        signature: Option<Signature>,
    ) -> Self {
        Self { sponsor, signer, signature }
    }

    pub const fn sponsor(&self) -> Address {
        self.sponsor
    }

    pub async fn attach_and_print<N: Network>(
        &self,
        tx: &mut N::TransactionRequest,
        sender: Address,
    ) -> Result<TempoSponsorPreview>
    where
        N::TransactionRequest: FoundryTransactionBuilder<N>,
    {
        if self.sponsor == sender {
            eyre::bail!(
                "invalid Tempo sponsorship: sponsor {} must not equal transaction sender",
                self.sponsor
            );
        }

        let digest = tx.compute_sponsor_hash(sender).ok_or_else(|| {
            eyre::eyre!(
                "failed to compute Tempo sponsor digest; make sure this is a complete Tempo AA transaction"
            )
        })?;

        let preview = TempoSponsorPreview {
            sponsor: self.sponsor,
            fee_token: tx.fee_token(),
            valid_before: tx.valid_before().map(|v| v.get()),
            valid_after: tx.valid_after().map(|v| v.get()),
            digest,
        };
        preview.print()?;

        let signature = if let Some(signature) = self.signature {
            signature
        } else if let Some(signer) = &self.signer {
            signer.sign_hash(&digest).await.context("failed to sign Tempo sponsor digest")?
        } else {
            eyre::bail!("missing Tempo sponsor signature or signer")
        };

        let recovered = signature
            .recover_address_from_prehash(&digest)
            .context("failed to recover Tempo sponsor signature")?;
        if recovered != self.sponsor {
            eyre::bail!("Tempo sponsor signature recovered {recovered}, expected {}", self.sponsor);
        }
        if recovered == sender {
            eyre::bail!(
                "invalid Tempo sponsorship: recovered fee payer {recovered} must not equal transaction sender"
            );
        }

        tx.set_fee_payer_signature(signature);
        Ok(preview)
    }
}

/// User-visible sponsor digest metadata for a single outgoing Tempo transaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TempoSponsorPreview {
    pub sponsor: Address,
    pub fee_token: Option<Address>,
    pub valid_before: Option<u64>,
    pub valid_after: Option<u64>,
    pub digest: B256,
}

impl TempoSponsorPreview {
    pub fn print(&self) -> Result<()> {
        crate::sh_eprintln!("Tempo sponsor: {}", self.sponsor)?;
        crate::sh_eprintln!(
            "Tempo fee token: {}",
            self.fee_token.map_or_else(|| "network default".to_string(), |addr| addr.to_string())
        )?;
        crate::sh_eprintln!(
            "Tempo validity: after {}, before {}",
            self.valid_after.map_or_else(|| "none".to_string(), |v| v.to_string()),
            self.valid_before.map_or_else(|| "none".to_string(), |v| v.to_string())
        )?;
        crate::sh_eprintln!("Tempo sponsor digest: {:?}", self.digest)?;
        Ok(())
    }
}

/// Resolves a `--tempo.sponsor-signer` URI into a Foundry wallet signer.
pub async fn resolve_tempo_sponsor_signer(spec: &str) -> Result<WalletSigner> {
    let spec = spec.trim();
    let (scheme, value) = spec
        .split_once("://")
        .map(|(scheme, value)| (scheme.to_ascii_lowercase(), value))
        .unwrap_or_else(|| (spec.to_ascii_lowercase(), ""));

    match scheme.as_str() {
        "env" => {
            if value.is_empty() {
                eyre::bail!("env:// sponsor signer requires an environment variable name");
            }
            let private_key = std::env::var(value)
                .wrap_err_with(|| format!("{value} environment variable is required"))?;
            foundry_wallets::utils::create_private_key_signer(&private_key)
        }
        "private-key" => {
            if value.is_empty() {
                eyre::bail!("private-key:// sponsor signer requires a private key");
            }
            foundry_wallets::utils::create_private_key_signer(value)
        }
        "keystore" => {
            if value.is_empty() {
                eyre::bail!("keystore:// sponsor signer requires a keystore path");
            }
            WalletOpts { keystore_path: Some(value.to_string()), ..Default::default() }
                .signer()
                .await
        }
        "account" => {
            if value.is_empty() {
                eyre::bail!("account:// sponsor signer requires an account name");
            }
            WalletOpts { keystore_account_name: Some(value.to_string()), ..Default::default() }
                .signer()
                .await
        }
        "ledger" => {
            let raw = RawWalletOpts {
                hd_path: (!value.is_empty()).then(|| value.to_string()),
                ..Default::default()
            };
            WalletOpts { ledger: true, raw, ..Default::default() }.signer().await
        }
        "trezor" => {
            let raw = RawWalletOpts {
                hd_path: (!value.is_empty()).then(|| value.to_string()),
                ..Default::default()
            };
            WalletOpts { trezor: true, raw, ..Default::default() }.signer().await
        }
        "aws" => WalletOpts { aws: true, ..Default::default() }.signer().await,
        "gcp" => WalletOpts { gcp: true, ..Default::default() }.signer().await,
        "turnkey" => WalletOpts { turnkey: true, ..Default::default() }.signer().await,
        "browser" => {
            eyre::bail!(
                "browser:// sponsor signing is not supported by the current browser wallet API; use --tempo.sponsor-sig or another sponsor signer"
            )
        }
        _ => eyre::bail!(
            "unsupported Tempo sponsor signer `{spec}`; expected env://VAR, keystore://PATH, account://NAME, ledger://, trezor://, aws://, gcp://, turnkey://, or private-key://KEY"
        ),
    }
}
