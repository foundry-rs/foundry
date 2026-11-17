//! The sender of zone deposits and withdrawals, which also signs zone RPC authorization tokens.

use crate::tempo::CAST_LOCAL_OWNERS_HINT;
use alloy_consensus::SignableTransaction;
use alloy_network::NetworkWallet;
use alloy_primitives::{Address, B256, Bytes};
use alloy_signer::Signer;
use clap::Parser;
use eyre::{Result, ensure};
use foundry_cli::opts::TempoOpts;
use foundry_common::{sh_status, tempo::configurable_account::ConfigurableAccountSigner};
use foundry_config::Config;
use foundry_wallets::{WalletOpts, WalletSigner};
use tempo_alloy::{TempoNetwork, rpc::TempoTransactionRequest};
use tempo_primitives::{TempoTxEnvelope, transaction::TempoTypedTransaction};

/// Tempo configurable account options of the zone commands, as `cast send` takes them.
#[derive(Clone, Debug, Default, Parser)]
#[command(next_help_heading = "Configurable account (T14)")]
pub(super) struct ConfigurableAccountOpts {
    /// Send as this Tempo configurable account (T14), failing unless it is one.
    ///
    /// Not needed for a sender named with `--from` other than the wallet's key: a configurable
    /// account sender is detected from Tempo L1, and the local owner keys (the wallet options,
    /// `--tempo.owner`, and owner keys in the Tempo Accounts store) sign its `0x05` transactions
    /// and zone RPC authorization tokens. An EOA upgraded to a configurable account names itself
    /// here to withdraw.
    #[arg(long = "tempo.configurable-account", value_name = "ADDRESS")]
    configurable_account: Option<Address>,

    /// The configuration of the configurable account sender, instead of rebuilding it from
    /// Tempo L1.
    ///
    /// JSON, inline or in a file, as `cast wallet owners --json` prints it.
    #[arg(long = "tempo.config", value_name = "CONFIG")]
    config: Option<String>,

    /// An owner key of the configurable account sender, in addition to the wallet options.
    ///
    /// Repeat for several owners. Takes the forms of `cast send --tempo.owner`.
    #[arg(long = "tempo.owner", value_name = "SIGNER")]
    owners: Vec<String>,

    /// Send from the key itself instead of the Tempo configurable account linked to its
    /// keystore.
    #[arg(long, conflicts_with = "configurable_account")]
    eoa: bool,
}

impl ConfigurableAccountOpts {
    fn tempo(&self) -> TempoOpts {
        TempoOpts {
            configurable_account: self.configurable_account,
            configurable_account_config: self.config.clone(),
            owners: self.owners.clone(),
            eoa: self.eoa,
            ..Default::default()
        }
    }

    /// Returns whether the sender may be a configurable account: the options select one,
    /// `--from` names a sender other than the wallet's key `local`, or the wallet's keystore is
    /// linked to one.
    pub(super) fn may_select(
        &self,
        wallet: &WalletOpts,
        local: Option<&WalletSigner>,
    ) -> Result<bool> {
        let tempo = self.tempo();
        Ok(tempo.has_configurable_account_options()
            || wallet.from.is_some_and(|from| local.is_none_or(|key| key.address() != from))
            || tempo.linked_account(wallet)?.is_some())
    }
}

/// Signs zone transactions and RPC authorization tokens as the wallet's key or, as `cast send`
/// does, as the Tempo configurable account the sender is.
#[derive(Debug)]
pub(super) enum ZoneSigner {
    Wallet(WalletSigner),
    ConfigurableAccount(ConfigurableAccountSigner),
}

impl ZoneSigner {
    /// Resolves the sender as `cast send` does from the wallet's key `signer`, detecting a
    /// configurable account on the Tempo chain that `l1` connects to. Zones follow the account's
    /// Tempo configuration (TIP-1154). Without `l1`, the wallet's key sends.
    pub(super) async fn resolve(
        wallet: &WalletOpts,
        opts: &ConfigurableAccountOpts,
        mut signer: Option<WalletSigner>,
        l1: Option<&Config>,
    ) -> Result<Self> {
        if let Some(config) = l1 {
            let account = opts
                .tempo()
                .configurable_account_signer(config, wallet, &mut signer, CAST_LOCAL_OWNERS_HINT)
                .await?;
            if let Some(account) = account {
                sh_status!("Sending as Tempo configurable account {}", account.account())?;
                return Ok(Self::ConfigurableAccount(account));
            }
        }
        let signer = match signer {
            Some(signer) => signer,
            None => wallet.signer().await?,
        };
        if let Some(from) = wallet.from {
            ensure!(from == signer.address(), "--from does not match the signing wallet");
        }
        Ok(Self::Wallet(signer))
    }

    /// The sender of the zone transactions and the caller the zone RPC authenticates.
    pub(super) fn address(&self) -> Address {
        match self {
            Self::Wallet(signer) => signer.address(),
            Self::ConfigurableAccount(account) => account.sender(),
        }
    }

    /// Makes `tx` a Tempo transaction whose gas estimate models the owners' approvals, when the
    /// sender is a configurable account.
    pub(super) fn prepare(&self, mut tx: TempoTransactionRequest) -> TempoTransactionRequest {
        if let Self::ConfigurableAccount(account) = self {
            account.prepare_request::<TempoNetwork>(&mut tx);
        }
        tx
    }

    /// Signs a zone RPC token `digest`: a raw 65-byte secp256k1 signature for the wallet's key,
    /// as `cast wallet sign --no-hash` makes, or the `0x05` owner quorum of a configurable
    /// account, which approves `configurable_account_digest(digest, account, version)`.
    pub(super) async fn sign_token_digest(&self, digest: B256) -> Result<Bytes> {
        Ok(match self {
            Self::Wallet(signer) => signer.sign_hash(&digest).await?.as_bytes().into(),
            Self::ConfigurableAccount(account) => account.sign_hash(digest).await?.to_bytes(),
        })
    }
}

impl NetworkWallet<TempoNetwork> for ZoneSigner {
    fn default_signer_address(&self) -> Address {
        self.address()
    }

    fn has_signer_for(&self, address: &Address) -> bool {
        *address == self.address()
    }

    fn signer_addresses(&self) -> impl Iterator<Item = Address> {
        std::iter::once(self.address())
    }

    async fn sign_transaction_from(
        &self,
        sender: Address,
        mut tx: TempoTypedTransaction,
    ) -> alloy_signer::Result<TempoTxEnvelope> {
        if sender != self.address() {
            return Err(alloy_signer::Error::other(format!("no signer for {sender}")));
        }
        match self {
            Self::Wallet(signer) => {
                let signature = alloy_network::TxSigner::sign_transaction(signer, &mut tx).await?;
                Ok(tx.into_signed(signature).into())
            }
            Self::ConfigurableAccount(account) => {
                let TempoTypedTransaction::AA(tx) = tx else {
                    return Err(alloy_signer::Error::other(
                        "a configurable account only signs Tempo transactions",
                    ));
                };
                let signature = account
                    .sign_hash(tx.signature_hash())
                    .await
                    .map_err(|err| alloy_signer::Error::other(format!("{err:#}")))?;
                Ok(TempoTxEnvelope::AA(tx.into_signed(signature)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Parser)]
    struct Args {
        #[command(flatten)]
        wallet: WalletOpts,
        #[command(flatten)]
        configurable_account: ConfigurableAccountOpts,
    }

    /// Withdrawals query Tempo L1 only for a sender that may be a configurable account, so that a
    /// plain key, even named with `--from` or `ETH_FROM`, stays off a public L1 RPC.
    #[tokio::test]
    async fn only_possible_configurable_accounts_query_l1() {
        let key = "0x6161616161616161616161616161616161616161616161616161616161616161";
        let address = WalletSigner::from_private_key(&key.parse().unwrap()).unwrap().address();
        let (address, other) = (address.to_string(), Address::repeat_byte(1).to_string());
        let config = r#"{"salt":"0x00","version":0,"threshold":1,"owners":[]}"#;
        for (args, expected) in [
            (&[][..], false),
            (&["--private-key", key], false),
            (&["--private-key", key, "--from", &address], false),
            (&["--private-key", key, "--eoa"], false),
            (&["--private-key", key, "--from", &other], true),
            (&["--from", &other], true),
            (&["--tempo.configurable-account", &other], true),
            (&["--tempo.config", config], true),
            (&["--tempo.owner", "env://OWNER"], true),
        ] {
            let args =
                Args::try_parse_from(std::iter::once("zone").chain(args.iter().copied())).unwrap();
            let local = args.wallet.maybe_signer().await.unwrap().0;
            let selected = args.configurable_account.may_select(&args.wallet, local.as_ref());
            assert_eq!(selected.unwrap(), expected, "{args:?}", args = args.wallet);
        }
    }
}
