//! Signing and verifying wallet messages with a Tempo configurable account.

use crate::tempo::{CAST_LOCAL_OWNERS_HINT, tempo_provider};
use alloy_primitives::{Address, B256, Bytes};
use clap::Args;
use eyre::{OptionExt, Result, WrapErr};
use foundry_cli::{
    opts::{RpcOpts, TempoOpts},
    utils::LoadConfig,
};
use foundry_wallets::WalletOpts;
use tempo_contracts::precompiles::{ISignatureVerifier, SIGNATURE_VERIFIER_ADDRESS};
use tempo_primitives::transaction::{PrimitiveSignature, TempoSignature};

#[derive(Debug, Args)]
#[command(next_help_heading = "Tempo configurable account")]
pub struct ConfigurableSignOpts {
    /// Sign as this configurable account with its current owner quorum (Tempo T14).
    #[arg(
        long = "tempo.configurable-account",
        id = "configurable_account",
        value_name = "ADDRESS",
        conflicts_with_all = ["browser", "p256", "tempo_access_key"]
    )]
    pub(super) account: Option<Address>,

    /// Current configuration as JSON or a file. Supply the initial config for a fresh multisig.
    #[arg(long = "tempo.config", requires = "configurable_account", value_name = "CONFIG")]
    config: Option<String>,

    /// Additional owner signer, in the same format as `cast send --tempo.owner`. Can be repeated.
    #[arg(long = "tempo.owner", requires = "configurable_account", value_name = "SIGNER")]
    owners: Vec<String>,
}

impl ConfigurableSignOpts {
    pub(super) async fn sign(
        self,
        account: Address,
        hash: B256,
        mut wallet: WalletOpts,
        rpc: &RpcOpts,
    ) -> Result<Bytes> {
        if let Some(from) = wallet.from.take() {
            eyre::ensure!(
                from == account,
                "--from {from} does not match the configurable account {account}"
            );
        }
        let (mut signer, _) = wallet.maybe_signer().await?;
        let tempo = TempoOpts {
            configurable_account: Some(account),
            configurable_account_config: self.config,
            owners: self.owners,
            ..Default::default()
        };
        let signer = tempo
            .configurable_account_signer(
                &rpc.load_config()?,
                &wallet,
                &mut signer,
                CAST_LOCAL_OWNERS_HINT,
            )
            .await?
            .ok_or_eyre("expected a Tempo configurable account")?;
        Ok(signer.sign_hash(hash).await?.to_bytes())
    }
}

/// Verifies ECDSA locally and configurable account approvals against the current commitment.
pub(super) async fn verify(
    address: Address,
    hash: B256,
    signature: Bytes,
    rpc: &RpcOpts,
) -> Result<bool> {
    match TempoSignature::from_bytes(&signature).map_err(eyre::Report::msg)? {
        TempoSignature::Primitive(PrimitiveSignature::Secp256k1(sig)) => {
            Ok(sig.recover_address_from_prehash(&hash)? == address)
        }
        TempoSignature::ConfigurableAccount(_) => {
            let (_, provider) = tempo_provider(rpc)?;
            ISignatureVerifier::new(SIGNATURE_VERIFIER_ADDRESS, provider)
                .verifyConfigurableAccount(address, hash, signature)
                .call()
                .await
                .wrap_err("could not verify the configurable account signature on the Tempo RPC")
        }
        _ => eyre::bail!("expected an ECDSA or Tempo configurable account signature"),
    }
}
