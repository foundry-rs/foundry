//! Tempo network utilities.

pub mod auth;

use crate::FoundryTransactionBuilder;
use alloy_network::Network;
use alloy_primitives::{Address, B256, Signature};
use alloy_signer::Signer;
use eyre::{Context, Result};
use foundry_wallets::{RawWalletOpts, WalletOpts, WalletSigner};
use std::sync::Arc;

mod keystore;

pub(crate) use auth::is_known_tempo_endpoint;
pub use auth::{AccessKeyOutcome, EnsureAccessKeyConfig, ensure_access_key};
pub use keystore::*;

#[cfg(test)]
pub(crate) use keystore::test_env_mutex;

#[cfg(test)]
mod tests;

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
