use super::TempoOpts;
use alloy_eips::BlockId;
use alloy_primitives::Address;
use alloy_provider::Provider;
use alloy_signer::Signer;
use eyre::{Result, WrapErr};
use foundry_common::{
    provider::ProviderBuilder,
    tempo::{
        configurable_account::{
            ConfigurableAccountOwnerKey, ConfigurableAccountSigner, detect_configurable_account,
            linked_account, parse_config, stored_owner_keys,
        },
        resolve_tempo_sponsor_signer,
    },
};
use foundry_config::Config;
use foundry_wallets::{WalletOpts, WalletSigner, utils::maybe_get_keystore_path};
use std::sync::Arc;
use tempo_alloy::TempoNetwork;
use tempo_primitives::transaction::ConfigurableAccountConfig;

impl TempoOpts {
    /// Returns whether the configurable account options select the sender.
    pub const fn has_configurable_account_options(&self) -> bool {
        self.configurable_account.is_some()
            || self.configurable_access_key.is_some()
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

    /// Resolves the `--tempo.owner` signers.
    pub async fn owner_signers(&self) -> Result<Vec<WalletSigner>> {
        let mut owners = Vec::with_capacity(self.owners.len());
        for spec in &self.owners {
            owners.push(Box::pin(resolve_tempo_sponsor_signer(spec)).await?);
        }
        Ok(owners)
    }

    /// Detects a configurable account sender and selects the local owners that sign for it,
    /// taking `signer` when it is one of them.
    ///
    /// The sender is `--tempo.configurable-account`, else `--from`, else the account linked to
    /// the signer's keystore (see [`Self::linked_account`]) when T14 is active, else the signer's
    /// address. It is a configurable account when the chain says so (see
    /// [`detect_configurable_account`]), or `--tempo.config` gives its configuration. Its owners
    /// are taken from `signer`, `--tempo.owner` and the Tempo Accounts store. Fails with `hint`
    /// when the local owners do not reach the threshold.
    ///
    /// With `--tempo.configurable-access-key`, that account signs as an access key of `--from`
    /// instead (see [`ConfigurableAccountSigner::with_parent`]).
    ///
    /// Returns `None`, and leaves `signer` in place, for a sender that is not a configurable
    /// account, and before T14.
    pub async fn configurable_account_signer(
        &self,
        config: &Config,
        wallet: &WalletOpts,
        signer: &mut Option<WalletSigner>,
        hint: &str,
    ) -> Result<Option<ConfigurableAccountSigner>> {
        let provider = ProviderBuilder::<TempoNetwork>::from_config(config)?.build()?;
        let signers = signer.take().map(Arc::new).into_iter().collect::<Vec<_>>();
        let linked = self.linked_account(wallet)?;
        let owners = self
            .configurable_account_owners(
                &provider,
                wallet.from,
                linked,
                &signers,
                BlockId::latest(),
            )
            .await?;
        let Some((account, config, keys)) = owners else {
            // Only a configurable account keeps the signer.
            *signer = signers.into_iter().next().and_then(Arc::into_inner);
            return Ok(None);
        };
        let account = ConfigurableAccountSigner::new(account, config, keys, hint)?;
        Ok(Some(match self.configurable_access_key.and(wallet.from) {
            Some(parent) => account.with_parent(parent),
            None => account,
        }))
    }

    /// Detects a configurable account sender as [`Self::configurable_account_signer`] does, at
    /// `block`, and returns the account, its configuration there and all its local owner keys,
    /// which may not reach the threshold.
    ///
    /// With `--tempo.configurable-access-key`, the returned account is that access key, and
    /// `from` is the account it signs for, which must be given.
    pub async fn configurable_account_owners<P: Provider<TempoNetwork>>(
        &self,
        provider: &P,
        from: Option<Address>,
        linked: Option<(Address, Address)>,
        signers: &[Arc<WalletSigner>],
        block: BlockId,
    ) -> Result<Option<(Address, ConfigurableAccountConfig, Vec<ConfigurableAccountOwnerKey>)>>
    {
        let from = match self.configurable_access_key {
            Some(key) => {
                let parent = from.ok_or_else(|| {
                    eyre::eyre!(
                        "--tempo.configurable-access-key needs the account it signs for: pass \
                         --from"
                    )
                })?;
                eyre::ensure!(parent != key, "a configurable account cannot be its own access key");
                Some(key)
            }
            None => from,
        };
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

        let owners = self.owner_signers().await?.into_iter().map(Arc::new).collect::<Vec<_>>();
        let keys = signers.iter().chain(&owners).map(|signer| signer.address()).collect::<Vec<_>>();
        let required = self.has_configurable_account_options();
        let mut detected = None;
        if let Some(linked) = linked {
            // Before T14 the linked account does not exist yet, and the key sends as before.
            detected =
                detect_configurable_account(provider, linked, &keys, config.clone(), false, block)
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
        if detected.is_none() {
            detected =
                detect_configurable_account(provider, account, &keys, config, required, block)
                    .await?;
        }
        let Some(config) = detected else { return Ok(None) };

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
