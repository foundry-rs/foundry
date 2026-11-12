//! Tempo access keys authorized by a configurable account's owners (T14, TIP-1111).
//!
//! `cast keychain authorize` sends `authorizeKey` from the account in a `0x05` transaction.
//! `cast key-authorization sign` signs an inline key authorization: the owners approve
//! `configurable_account_digest(authorization.signature_hash(), account, version)`, and their
//! approvals form its `0x05` signature. The account's configuration is read from the chain, as
//! `cast wallet` does.

use crate::{
    cmd::{
        auth::confirm_and_build,
        send::{SendOptions, cast_send_with_configurable_account},
    },
    tempo::{apply_fee_payment, print_expires, resolve_configurable_account, tempo_provider},
    tx::{CastTxBuilder, SendTxOpts, apply_poll_interval},
};
use alloy_ens::NameOrAddress;
use alloy_primitives::{Address, U64, hex};
use alloy_provider::Provider;
use alloy_signer::Signer;
use clap::Args;
use eyre::{Result, WrapErr};
use foundry_cli::{
    opts::TransactionOpts,
    utils::{LoadConfig, resolve_lane},
};
use foundry_common::tempo::configurable_account::{
    combine_approvals, current_config, parse_owner_signature,
};
use foundry_wallets::WalletSigner;
use tempo_alloy::{TempoNetwork, provider::TempoProviderExt};
use tempo_contracts::precompiles::ACCOUNT_KEYCHAIN_ADDRESS;
use tempo_primitives::transaction::{
    KeyAuthorization, PrimitiveSignature, SignedKeyAuthorization, TEMPO_EXPIRING_NONCE_KEY,
    configurable_account_digest,
};

/// Sign a key authorization as a Tempo configurable account's owners (T14, TIP-1111).
#[derive(Clone, Debug, Args)]
#[command(next_help_heading = "Configurable account (T14)")]
pub struct ConfigurableAccountArgs {
    /// Sign as this configurable account, whose current configuration is read from the RPC.
    ///
    /// The authorization is bound to the account and stays valid only while the configuration
    /// does not change. The wallet signer approves as one owner; `--tempo.owner-signature` adds
    /// approvals made elsewhere. The account's own `0x05` transaction or the authorized key's
    /// first transaction can carry it.
    #[arg(long = "tempo.configurable-account", value_name = "ADDRESS")]
    configurable_account: Option<Address>,

    /// An owner approval made elsewhere: a hex Tempo primitive signature, a JSON signature or a
    /// WebAuthn assertion, or a file holding one. Owners sign the owner digest with
    /// `cast wallet sign --no-hash <DIGEST>`. Repeat for several owners.
    #[arg(
        long = "tempo.owner-signature",
        value_name = "SIGNATURE",
        value_parser = parse_owner_signature,
        requires = "configurable_account"
    )]
    owner_signatures: Vec<PrimitiveSignature>,
}

impl ConfigurableAccountArgs {
    /// The configurable account whose owners sign, if any.
    pub const fn account(&self) -> Option<Address> {
        self.configurable_account
    }

    /// Signs `authorization`, bound to `account`, with the approvals of `owner` and
    /// `--tempo.owner-signature`.
    pub async fn sign(
        &self,
        rpc: &impl LoadConfig,
        account: Address,
        owner: Option<&WalletSigner>,
        authorization: KeyAuthorization,
    ) -> Result<SignedKeyAuthorization> {
        let (_, provider) = tempo_provider(rpc)?;
        let chain_id = provider.get_chain_id().await?;
        eyre::ensure!(
            chain_id == authorization.chain_id,
            "--chain-id {} does not match the RPC's chain {chain_id}, where the configuration \
             of {account} is read",
            authorization.chain_id
        );
        let inner = authorization.signature_hash();

        // A 1-of-1, salt-0 account is recomputed from an owner's key before it registers.
        let initial = configurable_account_digest(inner, account, 0);
        let mut keys = self
            .owner_signatures
            .iter()
            .filter_map(|approval| approval.recover_signer(&initial).ok())
            .collect::<Vec<_>>();
        keys.extend(owner.map(Signer::address));
        let config = current_config(&provider, account, &keys).await?.ok_or_else(|| {
            eyre::eyre!(
                "{account} is not a registered configurable account, and no owner key derives it"
            )
        })?;

        let digest = configurable_account_digest(inner, account, config.version);
        let mut approvals = self.owner_signatures.clone();
        if let Some(owner) = owner {
            approvals.push(PrimitiveSignature::Secp256k1(owner.sign_hash(&digest).await?));
        }
        let signature =
            combine_approvals(account, &config, inner, approvals).wrap_err_with(|| {
                format!(
                    "other owners sign the owner digest with `cast wallet sign --no-hash \
                     {digest}`; pass their signatures with --tempo.owner-signature"
                )
            })?;
        Ok(authorization.into_signed(signature))
    }
}

/// Sends `calldata` to the AccountKeychain precompile from the configurable account that
/// `--tempo.configurable-account` names, in a `0x05` transaction that its local owners approve
/// (T14): the wallet signer, `--tempo.owner` and the Tempo Accounts store, as `cast send` does.
pub(crate) async fn send_keychain_tx_as_configurable_account(
    calldata: Vec<u8>,
    mut tx: TransactionOpts,
    send_tx: &SendTxOpts,
    force: bool,
) -> Result<()> {
    eyre::ensure!(
        tx.tempo.sponsor_url.is_none() && !tx.tempo.print_sponsor_hash,
        "--sponsor-url and --tempo.print-sponsor-hash are not supported for configurable accounts"
    );
    eyre::ensure!(
        !send_tx.browser.browser,
        "browser wallets cannot approve as a configurable account owner"
    );
    eyre::ensure!(
        tx.tempo.session_id()?.is_none(),
        "configurable account owners approve with their own keys; Tempo Accounts sessions cannot \
         sign for them"
    );
    let (config, provider) = tempo_provider(&send_tx.eth)?;
    apply_poll_interval(&provider, send_tx.poll_interval);

    // `--from` may name the account, so the wallet's own key is resolved without it.
    let mut key_wallet = send_tx.eth.wallet.clone();
    key_wallet.from = None;
    let (mut signer, access_key) = foundry_common::tempo::maybe_signer(&key_wallet, None).await?;
    eyre::ensure!(
        access_key.is_none(),
        "configurable account owners approve with their own keys, not a Tempo access key"
    );
    let account = resolve_configurable_account::<TempoNetwork>(
        &tx.tempo,
        &config,
        &send_tx.eth.wallet,
        &mut signer,
    )
    .await?
    .ok_or_else(|| eyre::eyre!("the sender is not a Tempo configurable account"))?;

    let print_expires_at = tx.tempo.resolve_expires();
    let tempo_sponsor = tx.tempo.sponsor_config().await?;
    let resolved_lane = resolve_lane(&mut tx.tempo, &config.root)?;
    // The builder fills the protocol nonce; a lane or other nonce key has its own counter.
    if tx.nonce.is_none()
        && let Some(nonce_key) = tx.tempo.nonce_key
        && !nonce_key.is_zero()
        && nonce_key != TEMPO_EXPIRING_NONCE_KEY
    {
        let nonce =
            provider.get_transaction_count_with_nonce_key(account.account(), nonce_key).await?;
        tx.nonce = Some(U64::from(nonce));
    }
    let mut builder = CastTxBuilder::new(&provider, tx, &config)
        .await?
        .with_to(Some(NameOrAddress::Address(ACCOUNT_KEYCHAIN_ADDRESS)))
        .await?
        .with_code_sig_and_args(None, Some(hex::encode_prefixed(&calldata)), vec![])
        .await?;
    account.prepare_request::<TempoNetwork>(builder.tx_mut());
    let Some(mut request) =
        confirm_and_build(builder, account.account(), force, resolved_lane.as_ref(), false).await?
    else {
        return Ok(());
    };
    print_expires(print_expires_at)?;
    apply_fee_payment::<TempoNetwork, _>(
        tempo_sponsor.as_ref(),
        Some(&provider),
        &mut request,
        account.account(),
    )
    .await?;
    cast_send_with_configurable_account(
        &provider,
        request,
        &account,
        &SendOptions::new(send_tx, &config),
    )
    .await?;
    Ok(())
}
