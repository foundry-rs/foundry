//! Tempo network utilities.

pub mod auth;

use crate::FoundryTransactionBuilder;
use alloy_chains::Chain;
use alloy_network::{Network, NetworkTransactionBuilder, TransactionBuilder};
use alloy_primitives::{Address, B256, Signature, TxKind, address};
use alloy_provider::Provider;
use alloy_signer::Signer;
use alloy_sol_types::SolCall;
use eyre::{Context, Result};
use foundry_evm_hardforks::{TempoHardfork, latest_active_tempo_hardfork};
use foundry_wallets::{RawWalletOpts, WalletOpts, WalletSigner};
use serde::Deserialize;
use std::sync::Arc;
pub use tempo_alloy::contracts::precompiles::PATH_USD_ADDRESS;
use tempo_alloy::contracts::precompiles::{
    IFeeManager, IStablecoinDEX, ITIP20, STABLECOIN_DEX_ADDRESS, TIP_FEE_MANAGER_ADDRESS,
};
use tempo_primitives::TempoAddressExt;

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

/// Resolves and applies the Tempo fee token selected by the network.
///
/// This must happen before computing a sponsor digest, because Tempo sponsor signatures commit to
/// the fee token.
pub async fn resolve_and_set_fee_token<N>(
    provider: Option<&dyn Provider<N>>,
    chain: Option<Chain>,
    tx: &mut N::TransactionRequest,
    fee_payer: Option<Address>,
) -> Result<Option<Address>>
where
    N: Network,
    N::TransactionRequest: Default + FoundryTransactionBuilder<N>,
{
    if let Some(fee_token) = tx.fee_token() {
        return Ok(Some(fee_token));
    }
    if !chain.is_some_and(Chain::is_tempo) {
        return Ok(None);
    }
    let fee_payer = fee_payer.or_else(|| tx.from());
    let calls = tx.tempo_calls();
    let has_call_list = tx.has_tempo_call_list();
    let tx_from = tx.from();

    let immediate_user_token =
        infer_fee_token_from_set_user_token_call(&calls, has_call_list, tx_from, fee_payer);
    let stored_fee_token = if immediate_user_token.is_none()
        && let (Some(provider), Some(fee_payer)) = (provider, fee_payer)
    {
        stored_user_fee_token(provider, fee_payer).await?
    } else {
        None
    };
    let inferred_fee_token =
        if immediate_user_token.is_none() && stored_fee_token.is_none() && !calls.is_empty() {
            let hardfork =
                active_tempo_hardfork(provider).await.unwrap_or_else(latest_active_tempo_hardfork);
            infer_fee_token_from_tip20_calls(&calls, tx_from, fee_payer, hardfork)
                .or_else(|| infer_fee_token_from_stablecoin_dex_calls(&calls, has_call_list))
        } else {
            None
        };
    let inferred_fee_token = match inferred_fee_token {
        Some(fee_token) if is_usd_tip20_fee_token(provider, fee_token).await => Some(fee_token),
        _ => None,
    };
    let fee_token = immediate_user_token.or(stored_fee_token).or(inferred_fee_token);

    let Some(fee_token) = fee_token else { return Ok(None) };
    tx.set_fee_token(fee_token);
    Ok(Some(fee_token))
}

async fn stored_user_fee_token<N>(
    provider: &dyn Provider<N>,
    fee_payer: Address,
) -> Result<Option<Address>>
where
    N: Network,
    N::TransactionRequest: Default + NetworkTransactionBuilder<N>,
{
    let call = IFeeManager::userTokensCall { user: fee_payer };
    let tx = N::TransactionRequest::default()
        .with_to(TIP_FEE_MANAGER_ADDRESS)
        .with_input(call.abi_encode());
    let output = provider
        .call(tx)
        .await
        .wrap_err_with(|| format!("failed to resolve Tempo fee token for {fee_payer}"))?;
    let fee_token = IFeeManager::userTokensCall::abi_decode_returns(&output)
        .wrap_err("failed to decode Tempo fee token lookup")?;
    Ok((!fee_token.is_zero()).then_some(fee_token))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TempoForkSchedule {
    active: String,
}

async fn active_tempo_hardfork<N>(provider: Option<&dyn Provider<N>>) -> Option<TempoHardfork>
where
    N: Network,
{
    let provider = provider?;
    let params = serde_json::value::to_raw_value(&()).ok()?;
    let response = provider.raw_request_dyn("tempo_forkSchedule".into(), &params).await.ok()?;
    serde_json::from_str::<TempoForkSchedule>(response.get()).ok()?.active.parse().ok()
}

async fn is_usd_tip20_fee_token<N>(provider: Option<&dyn Provider<N>>, fee_token: Address) -> bool
where
    N: Network,
    N::TransactionRequest: Default + NetworkTransactionBuilder<N>,
{
    if !fee_token.is_tip20() {
        return false;
    }
    if known_fee_token_symbol(fee_token).is_some() {
        return true;
    }

    let Some(provider) = provider else {
        return false;
    };

    let tx = N::TransactionRequest::default()
        .with_to(fee_token)
        .with_input(ITIP20::currencyCall.abi_encode());
    let Ok(output) = provider.call(tx).await else {
        return false;
    };
    ITIP20::currencyCall::abi_decode_returns(&output).is_ok_and(|currency| currency == "USD")
}

fn infer_fee_token_from_tip20_calls(
    calls: &[(TxKind, &[u8])],
    tx_from: Option<Address>,
    fee_payer: Option<Address>,
    hardfork: TempoHardfork,
) -> Option<Address> {
    if calls.is_empty() || !calls.iter().all(|(_, input)| is_tip20_fee_token_call(input, hardfork))
    {
        return None;
    }

    let target = common_call_target(calls)?;
    if fee_payer != tx_from {
        return None;
    }
    Some(target)
}

fn infer_fee_token_from_set_user_token_call(
    calls: &[(TxKind, &[u8])],
    has_call_list: bool,
    tx_from: Option<Address>,
    fee_payer: Option<Address>,
) -> Option<Address> {
    if has_call_list || fee_payer != tx_from {
        return None;
    }

    let (to, input) = calls.first()?;
    if *to != TxKind::Call(TIP_FEE_MANAGER_ADDRESS) {
        return None;
    }

    let call = IFeeManager::setUserTokenCall::abi_decode(input).ok()?;
    call.token.is_tip20().then_some(call.token)
}

fn infer_fee_token_from_stablecoin_dex_calls(
    calls: &[(TxKind, &[u8])],
    has_call_list: bool,
) -> Option<Address> {
    if has_call_list && calls.len() != 1 {
        return None;
    }
    let (to, input) = calls.first()?;
    if *to != TxKind::Call(STABLECOIN_DEX_ADDRESS) {
        return None;
    }
    decode_stablecoin_dex_fee_token(input)
}

fn common_call_target(calls: &[(TxKind, &[u8])]) -> Option<Address> {
    let mut targets = calls.iter().map(|(to, _)| match to {
        TxKind::Call(target) => Some(*target),
        TxKind::Create => None,
    });
    let target = targets.next()??;
    targets.all(|next| next == Some(target)).then_some(target)
}

fn is_tip20_fee_token_call(input: &[u8], hardfork: TempoHardfork) -> bool {
    input_selector(input).is_some_and(|selector| {
        selector == ITIP20::transferCall::SELECTOR
            || selector == ITIP20::transferWithMemoCall::SELECTOR
            || (!hardfork.is_t7() && selector == ITIP20::distributeRewardCall::SELECTOR)
    })
}

fn decode_stablecoin_dex_fee_token(input: &[u8]) -> Option<Address> {
    let selector = input_selector(input)?;
    if selector == IStablecoinDEX::swapExactAmountInCall::SELECTOR {
        IStablecoinDEX::swapExactAmountInCall::abi_decode(input).ok().map(|call| call.tokenIn)
    } else if selector == IStablecoinDEX::swapExactAmountOutCall::SELECTOR {
        IStablecoinDEX::swapExactAmountOutCall::abi_decode(input).ok().map(|call| call.tokenIn)
    } else {
        None
    }
}

fn input_selector(input: &[u8]) -> Option<&[u8]> {
    input.get(..4)
}

/// Returns the known symbol for a Tempo fee token without making an RPC call.
const fn known_fee_token_symbol(fee_token: Address) -> Option<&'static str> {
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

/// Prints the fee token selected for display.
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

    /// Resolves the fee token paid by this sponsor and applies it to the transaction request.
    ///
    /// This must happen before computing a sponsor digest, because Tempo sponsor signatures commit
    /// to the fee token.
    pub async fn resolve_and_set_fee_token<N>(
        &self,
        provider: Option<&dyn Provider<N>>,
        chain: Option<Chain>,
        tx: &mut N::TransactionRequest,
    ) -> Result<Option<Address>>
    where
        N: Network,
        N::TransactionRequest: Default + FoundryTransactionBuilder<N>,
    {
        resolve_and_set_fee_token(provider, chain, tx, Some(self.sponsor)).await
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
