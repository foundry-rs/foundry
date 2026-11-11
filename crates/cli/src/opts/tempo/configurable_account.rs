use super::TempoOpts;
use alloy_primitives::Address;
use alloy_provider::Provider;
use alloy_signer::Signer;
use eyre::{Result, WrapErr};
use foundry_common::tempo::{
    configurable_account::{
        ConfigurableAccountOwnerKey, ConfigurableAccountSigner, detect_configurable_account,
        linked_account, parse_config, stored_owner_keys,
    },
    resolve_tempo_sponsor_signer,
};
use foundry_wallets::{WalletOpts, WalletSigner, utils::maybe_get_keystore_path};
use std::sync::Arc;
use tempo_alloy::TempoNetwork;
use tempo_primitives::transaction::ConfigurableAccountConfig;

impl TempoOpts {
    /// Returns whether the configurable account options select the sender.
    pub const fn has_configurable_account_options(&self) -> bool {
        self.configurable_account.is_some()
            || self.configurable_account_config.is_some()
            || !self.owners.is_empty()
    }

    /// Returns the configurable account linked to the keystore of `wallet` by `cast wallet new`,
    /// and its owner key, unless `--from` or `--eoa` selects the key.
    pub fn linked_account(&self, wallet: &WalletOpts) -> Result<Option<(Address, Address)>> {
        if self.eoa || wallet.from.is_some() {
            return Ok(None);
        }
        match maybe_get_keystore_path(
            wallet.keystore_path.as_deref(),
            wallet.keystore_account_name.as_deref(),
        )? {
            Some(path) => linked_account(&path),
            None => Ok(None),
        }
    }

    /// Detects a configurable account sender and selects the local owners that sign for it.
    ///
    /// The sender is `--tempo.configurable-account`, else `from`, else the account `linked` to
    /// the signer's keystore (see [`Self::linked_account`]) when T14 is active, else the address
    /// of the only signer in `signers`. It is a configurable account when the chain says so (see
    /// [`detect_configurable_account`]), or `--tempo.config` gives its configuration. Its owners
    /// are taken from `signers`, `--tempo.owner` and the Tempo Accounts store. Fails with `hint`
    /// when the local owners do not reach the threshold.
    ///
    /// Returns `None` for a sender that is not a configurable account, and before T14.
    pub async fn configurable_account_signer<P: Provider<TempoNetwork>>(
        &self,
        provider: &P,
        from: Option<Address>,
        linked: Option<(Address, Address)>,
        signers: &[Arc<WalletSigner>],
        hint: &str,
    ) -> Result<Option<ConfigurableAccountSigner>> {
        let Some((account, config, keys)) =
            self.configurable_account_owners(provider, from, linked, signers).await?
        else {
            return Ok(None);
        };
        ConfigurableAccountSigner::new(account, config, keys, hint).map(Some)
    }

    /// Detects a configurable account sender as [`Self::configurable_account_signer`] does, and
    /// returns the account, its configuration and all its local owner keys, which may not reach
    /// the threshold.
    pub async fn configurable_account_owners<P: Provider<TempoNetwork>>(
        &self,
        provider: &P,
        from: Option<Address>,
        linked: Option<(Address, Address)>,
        signers: &[Arc<WalletSigner>],
    ) -> Result<Option<(Address, ConfigurableAccountConfig, Vec<ConfigurableAccountOwnerKey>)>>
    {
        if let (Some(account), Some(from)) = (self.configurable_account, from) {
            eyre::ensure!(
                account == from,
                "--tempo.configurable-account {account} does not match the sender {from}"
            );
        }
        let only_signer = match signers {
            [signer] => Some(signer.address()),
            _ => None,
        };
        let explicit = self.configurable_account.or(from);
        // A linked account only replaces its own owner key as the implicit sender.
        let linked = linked
            .filter(|(_, owner)| explicit.is_none() && only_signer == Some(*owner))
            .map(|(account, _)| account);
        let Some(mut account) = explicit.or(only_signer) else {
            eyre::ensure!(
                !self.has_configurable_account_options(),
                "configurable account options need a sender: pass --from or \
                 --tempo.configurable-account"
            );
            return Ok(None);
        };
        let config = self.configurable_account_config.as_deref().map(parse_config).transpose()?;

        let mut owners = Vec::with_capacity(self.owners.len());
        for spec in &self.owners {
            owners.push(Box::pin(resolve_tempo_sponsor_signer(spec)).await?);
        }
        let owners = owners.into_iter().map(Arc::new).collect::<Vec<_>>();
        let keys = signers.iter().chain(&owners).map(|signer| signer.address()).collect::<Vec<_>>();
        let required = self.has_configurable_account_options();
        let mut detected = None;
        if let Some(linked) = linked {
            // Before T14 the linked account does not exist yet, and the key sends as before.
            detected = detect_configurable_account(provider, linked, &keys, config.clone(), false)
                .await
                .wrap_err_with(|| {
                    format!(
                        "could not check the configurable account {linked} linked to the \
                         keystore; pass --from {account} or --eoa to send from the key"
                    )
                })?;
            if detected.is_some() {
                account = linked;
            }
        }
        let config = match detected {
            Some(config) => config,
            None => {
                let Some(config) =
                    detect_configurable_account(provider, account, &keys, config, required).await?
                else {
                    return Ok(None);
                };
                config
            }
        };

        let chain_id = provider.get_chain_id().await?;
        let keys = signers
            .iter()
            .chain(&owners)
            .filter(|signer| config.owner_weight(signer.address()).is_some())
            .map(|signer| ConfigurableAccountOwnerKey::Wallet(signer.clone()))
            .chain(stored_owner_keys(chain_id, &config))
            .collect();
        Ok(Some((account, config, keys)))
    }
}
