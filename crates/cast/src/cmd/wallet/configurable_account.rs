//! Tempo configurable accounts (T14: TIP-1107, TIP-1113) in `cast wallet`, built on
//! [`foundry_common::tempo::configurable_account`].

use crate::{
    cmd::{
        auth::confirm_and_build,
        confirm_continue,
        keychain::{KeychainRootSigner, send_precompile_tx_with_root_signer},
        send::{SendOptions, cast_send_raw},
    },
    tempo::{apply_fee_payment, print_expires, tempo_provider},
    tx::{CastTxBuilder, SendTxOpts, apply_poll_interval},
};
use alloy_chains::Chain;
use alloy_eips::{Decodable2718, Encodable2718};
use alloy_ens::NameOrAddress;
use alloy_primitives::{Address, hex, keccak256};
use alloy_provider::Provider;
use alloy_signer::Signer;
use alloy_sol_types::SolCall;
use clap::Parser;
use eyre::{Context, Result};
use foundry_cli::{
    json::print_json_success,
    opts::{RpcOpts, TransactionOpts},
    utils::{LoadConfig, now, resolve_lane},
};
use foundry_common::{
    provider::ProviderBuilder,
    shell,
    tempo::{
        active_tempo_hardfork,
        configurable_account::{
            abi_owners, ca_error, combine_approvals, current_config, parse_owner_signature,
            simulation_spec,
        },
    },
};
use foundry_config::{Config, FoundryHardfork};
use serde_json::{Value, json};
use std::{str::FromStr, time::Duration};
use tempo_alloy::{
    TempoNetwork,
    chainspec::hardfork::TempoHardfork,
    rpc::{ConfigurableAccountSimulationSpec, PrimitiveSignatureType},
};
use tempo_contracts::precompiles::{CONFIGURABLE_ACCOUNT_ADDRESS, IConfigurableAccount};
use tempo_primitives::transaction::{
    CONFIGURABLE_ACCOUNT_UPGRADE_DOMAIN, ConfigurableAccountConfig, ConfigurableAccountOwner,
    PrimitiveSignature, TEMPO_TX_TYPE_ID, TempoSignature, TempoTransaction, TempoTxEnvelope,
    configurable_account_digest,
};

/// Decides whether `cast wallet new` creates a configurable account.
///
/// It does unless `eoa` is set, when a Tempo signal is present and T14 is active there: `--chain`,
/// `chain` in `foundry.toml`, a Tempo network selected in `foundry.toml`, or an RPC from
/// `--rpc-url`, `ETH_RPC_URL` or `foundry.toml` that serves a Tempo chain. An RPC is asked for its
/// active hardfork; otherwise the compiled Tempo hardfork schedule is read for the chain.
/// A non-Tempo `--chain` creates an EOA without reading the configuration. Without `requested`
/// (`--configurable-account`), a configuration or RPC URL that cannot be read also creates an EOA;
/// `requested` fails on it, and reports why it creates an EOA instead of an account.
pub(crate) async fn new_wallet_is_configurable(
    eoa: bool,
    requested: bool,
    chain: Option<Chain>,
    rpc: &RpcOpts,
) -> Result<bool> {
    if eoa || chain.is_some_and(|chain| !chain.is_tempo()) {
        return Ok(false);
    }
    let loaded = rpc.load_config().map_err(Into::into).and_then(|config| {
        let url = rpc.url(Some(&config))?.is_some();
        eyre::Ok((config, url))
    });
    let (config, url) = match loaded {
        Ok(loaded) => loaded,
        Err(_) if !requested => return Ok(false),
        Err(err) => return Err(err),
    };
    let chain = chain.or(config.chain);

    let mut signal = None;
    if url {
        match rpc_tempo_hardfork(&config).await {
            Ok(rpc_signal) => signal = rpc_signal,
            Err(err) => {
                sh_warn!("Could not ask the RPC for its chain ({err}); using the configured chain")?
            }
        }
    }
    let signal = match signal {
        Some(signal) => Some(signal),
        None => match chain {
            Some(chain) => chain.is_tempo().then(|| scheduled_hardfork(chain)),
            None if config.networks.is_tempo() => Some(match config.hardfork {
                Some(FoundryHardfork::Tempo(hardfork)) => {
                    (format!("the configured hardfork {hardfork}"), Some(hardfork))
                }
                _ => scheduled_hardfork(Chain::from_id(TEMPO_MAINNET)),
            }),
            None => None,
        },
    };

    let Some((source, hardfork)) = signal else {
        if requested {
            sh_status!(
                "No Tempo chain was given (--chain, an RPC or foundry.toml); creating an EOA."
            )?;
        }
        return Ok(false);
    };
    if hardfork.is_some_and(|hardfork| hardfork >= TempoHardfork::T14) {
        return Ok(true);
    }
    if requested {
        sh_status!("Tempo T14 is not active on {source}; creating an EOA.")?;
    }
    Ok(false)
}

/// Tempo mainnet's chain ID.
const TEMPO_MAINNET: u64 = 4217;

/// Asks the configured RPC for its Tempo hardfork. Returns `None` for a non-Tempo chain.
async fn rpc_tempo_hardfork(config: &Config) -> Result<Option<(String, Option<TempoHardfork>)>> {
    // Key generation stays fast with an unreachable or non-Tempo RPC: one short attempt each.
    let provider = ProviderBuilder::<TempoNetwork>::from_config(config)?
        .max_retry(0)
        .timeout(Duration::from_secs(5))
        .build()?;
    if let Ok(hardfork) = active_tempo_hardfork(&provider).await {
        return Ok(Some(("the RPC chain".to_string(), Some(hardfork))));
    }
    let chain = Chain::from_id(provider.get_chain_id().await?);
    Ok(chain.is_tempo().then(|| scheduled_hardfork(chain)))
}

/// Returns the hardfork that the compiled Tempo schedule activates on `chain` now, if known.
fn scheduled_hardfork(chain: Chain) -> (String, Option<TempoHardfork>) {
    let timestamp = now().as_secs();
    (format!("chain {chain}"), TempoHardfork::from_chain_and_timestamp(chain.id(), timestamp))
}

/// Decodes the unsigned Tempo transaction that `cast mktx --raw-unsigned` prints.
fn decode_unsigned_tx(raw: &str) -> Result<TempoTransaction> {
    let bytes = hex::decode(raw.trim()).wrap_err("invalid hex transaction")?;
    let Some((&TEMPO_TX_TYPE_ID, mut fields)) = bytes.split_first() else {
        eyre::bail!(
            "expected an unsigned Tempo (0x76) transaction from `cast mktx --raw-unsigned`"
        );
    };
    let tx = <TempoTransaction as alloy_rlp::Decodable>::decode(&mut fields).map_err(|_| {
        eyre::eyre!("invalid unsigned Tempo transaction; sponsored transactions are not supported")
    })?;
    eyre::ensure!(fields.is_empty(), "trailing bytes after the unsigned Tempo transaction");
    Ok(tx)
}

/// Parses an owner as `ADDRESS` or `ADDRESS:WEIGHT`.
fn parse_owner(value: &str) -> Result<ConfigurableAccountOwner> {
    let (owner, weight) = match value.split_once(':') {
        Some((owner, weight)) => (owner, weight.parse().wrap_err("invalid owner weight")?),
        None => (value, 1),
    };
    Ok(ConfigurableAccountOwner { owner: Address::from_str(owner)?, weight })
}

/// Prints a configuration as a table, or as JSON in `--json` mode.
fn print_config(account: Address, config: &ConfigurableAccountConfig) -> Result<()> {
    if shell::is_json() {
        return print_json_success(config_json(account, config)?);
    }
    sh_println!("Account:    {account}")?;
    sh_println!("Version:    {}", config.version)?;
    sh_println!("Threshold:  {}", config.threshold)?;
    sh_println!("Salt:       {}", config.salt)?;
    sh_println!("Commitment: {}", config.commitment().map_err(ca_error)?)?;
    sh_println!("Owners:")?;
    for owner in &config.owners {
        sh_println!("  {} (weight {})", owner.owner, owner.weight)?;
    }
    Ok(())
}

fn config_json(account: Address, config: &ConfigurableAccountConfig) -> Result<Value> {
    Ok(json!({
        "account": account,
        "version": config.version,
        "threshold": config.threshold,
        "salt": config.salt,
        "commitment": config.commitment().map_err(ca_error)?,
        "owners": config.owners.iter().map(|owner| json!({
            "owner": owner.owner,
            "weight": owner.weight,
        })).collect::<Vec<_>>(),
    }))
}

/// `cast wallet owners`: shows or changes the owners of a configurable account.
#[derive(Debug, Parser)]
#[command(args_conflicts_with_subcommands = true, subcommand_negates_reqs = true)]
pub struct OwnersArgs {
    #[command(subcommand)]
    command: Option<OwnersSubcommand>,

    /// The configurable account.
    #[arg(required = true)]
    account: Option<Address>,

    #[command(flatten)]
    rpc: RpcOpts,
}

/// Owner changes, each sent as an `updateConfig` transaction from the account itself.
///
/// The configured wallet signs as an owner and must reach the threshold on its own. With more
/// owners, the command prints the transaction to sign with the multi-party flow:
/// `cast mktx --raw-unsigned --tempo.digest`, `cast wallet sign --no-hash` on each owner's
/// machine, `cast wallet assemble`, then `cast publish`.
#[derive(Clone, Debug, Parser)]
pub enum OwnersSubcommand {
    /// Add an owner.
    Add {
        /// The configurable account.
        account: Address,

        /// The new owner's address.
        owner: Address,

        /// The new owner's weight.
        #[arg(long, default_value_t = 1)]
        weight: u8,

        /// The new threshold. Defaults to the current threshold.
        #[arg(long)]
        threshold: Option<u8>,

        #[command(flatten)]
        send: OwnerSendArgs,
    },

    /// Remove an owner.
    #[command(visible_alias = "rm")]
    Remove {
        /// The configurable account.
        account: Address,

        /// The owner to remove.
        owner: Address,

        /// The new threshold. Defaults to the current threshold.
        #[arg(long)]
        threshold: Option<u8>,

        #[command(flatten)]
        send: OwnerSendArgs,
    },

    /// Change the threshold.
    Threshold {
        /// The configurable account.
        account: Address,

        /// The new threshold: the total owner weight that must approve a transaction.
        threshold: u8,

        #[command(flatten)]
        send: OwnerSendArgs,
    },
}

/// Transaction options for an owner change.
#[derive(Clone, Debug, Parser)]
pub struct OwnerSendArgs {
    /// Skip the EIP-7702 authorization disclosure confirmation.
    #[arg(long)]
    force: bool,

    #[command(flatten)]
    tx: TransactionOpts,

    #[command(flatten)]
    send_tx: SendTxOpts,
}

impl OwnersArgs {
    pub async fn run(self) -> Result<()> {
        let Some(command) = self.command else {
            let account = self.account.expect("required by clap");
            let (_, provider) = tempo_provider(&self.rpc)?;
            let Some(config) = current_config(&provider, account, &[]).await? else {
                eyre::bail!(
                    "{account} is not a registered configurable account. An account registers on \
                     its first transaction; until then its configuration is only known to its \
                     owners."
                );
            };
            return print_config(account, &config);
        };

        update_config(command).await
    }
}

/// Sends the `updateConfig` transaction that `command` describes.
async fn update_config(command: OwnersSubcommand) -> Result<()> {
    let (account, OwnerSendArgs { force, mut tx, send_tx }) = match &command {
        OwnersSubcommand::Add { account, send, .. }
        | OwnersSubcommand::Remove { account, send, .. }
        | OwnersSubcommand::Threshold { account, send, .. } => (*account, send.clone()),
    };
    eyre::ensure!(
        tx.tempo.sponsor_url.is_none() && !tx.tempo.print_sponsor_hash,
        "--sponsor-url and --tempo.print-sponsor-hash are not supported for configurable accounts"
    );
    eyre::ensure!(
        tx.tempo.session_id()?.is_none(),
        "configuration updates need an owner key; Tempo Accounts sessions cannot sign them"
    );
    let signer = send_tx.eth.wallet.signer().await?;
    let (config, provider) = tempo_provider(&send_tx.eth)?;
    apply_poll_interval(&provider, send_tx.poll_interval);

    let current =
        current_config(&provider, account, &[signer.address()]).await?.ok_or_else(|| {
            eyre::eyre!(
                "{account} is not a registered configurable account, and the wallet {} does not \
                 derive it",
                signer.address()
            )
        })?;
    let mut next = current.clone();
    match command {
        OwnersSubcommand::Add { owner, weight, threshold, .. } => {
            eyre::ensure!(
                next.owner_weight(owner).is_none(),
                "{owner} is already an owner of {account}"
            );
            next.owners.push(ConfigurableAccountOwner { owner, weight });
            next.owners.sort_by_key(|owner| owner.owner);
            next.threshold = threshold.unwrap_or(next.threshold);
        }
        OwnersSubcommand::Remove { owner, threshold, .. } => {
            eyre::ensure!(
                next.owner_weight(owner).is_some(),
                "{owner} is not an owner of {account}"
            );
            next.owners.retain(|entry| entry.owner != owner);
            next.threshold = threshold.unwrap_or(next.threshold);
        }
        OwnersSubcommand::Threshold { threshold, .. } => next.threshold = threshold,
    }
    next.version = current.version.checked_add(1).ok_or_else(|| eyre::eyre!("version overflow"))?;
    next.validate().map_err(|err| eyre::eyre!("invalid new configuration: {err}"))?;
    let calldata = IConfigurableAccount::updateConfigCall {
        current: IConfigurableAccount::ConfigurableAccountConfig {
            salt: current.salt,
            version: current.version,
            threshold: current.threshold,
            owners: abi_owners(&current),
        },
        threshold: next.threshold,
        owners: abi_owners(&next),
    }
    .abi_encode();

    let signer_weight = current.owner_weight(signer.address()).unwrap_or_default();
    if signer_weight < current.threshold {
        eyre::bail!(
            "the wallet {} has weight {signer_weight} in {account}, which requires {}. Collect \
             the owners' approvals instead:\n  cast mktx {CONFIGURABLE_ACCOUNT_ADDRESS} {} --from \
             {account} --raw-unsigned --tempo.digest\nthen `cast wallet sign --no-hash <DIGEST>` \
             on each owner's machine, `cast wallet assemble`, and `cast publish`",
            signer.address(),
            current.threshold,
            hex::encode_prefixed(&calldata),
        );
    }

    let print_expires_at = tx.tempo.resolve_expires();
    let tempo_sponsor = tx.tempo.sponsor_config().await?;
    let resolved_lane = resolve_lane(&mut tx.tempo, &config.root)?;
    let mut builder = CastTxBuilder::new(&provider, tx, &config)
        .await?
        .with_to(Some(NameOrAddress::Address(CONFIGURABLE_ACCOUNT_ADDRESS)))
        .await?
        .with_code_sig_and_args(None, Some(hex::encode_prefixed(&calldata)), vec![])
        .await?;
    // The node models the owner quorum and, on the first transaction, the registration.
    builder.tx_mut().configurable_account_simulation = Some(simulation_spec(
        account,
        &current,
        &[(signer.address(), PrimitiveSignatureType::Secp256k1)],
    ));
    let Some(mut request) =
        confirm_and_build(builder, account, force, resolved_lane.as_ref(), false).await?
    else {
        return Ok(());
    };
    print_expires(print_expires_at)?;
    apply_fee_payment::<TempoNetwork, _>(
        tempo_sponsor.as_ref(),
        Some(&provider),
        &mut request,
        account,
    )
    .await?;

    let unsigned = request.build_aa().map_err(|err| eyre::eyre!("{err}"))?;
    let inner = unsigned.signature_hash();
    let approval =
        signer.sign_hash(&configurable_account_digest(inner, account, current.version)).await?;
    let signature =
        combine_approvals(account, &current, inner, vec![PrimitiveSignature::Secp256k1(approval)])?;
    let raw =
        TempoTxEnvelope::AA(unsigned.into_signed(TempoSignature::ConfigurableAccount(signature)))
            .encoded_2718();

    let send_opts = SendOptions::new(&send_tx, &config);
    let (tx_hash, receipt) = cast_send_raw(&provider, &raw, send_tx.sync).await?;
    send_opts.print_send_result(&provider, tx_hash, receipt).await?;
    Ok(())
}

/// `cast wallet upgrade`: converts an EOA into a configurable account in place (TIP-1113).
#[derive(Debug, Parser)]
pub struct UpgradeArgs {
    /// An owner of the upgraded account, as `ADDRESS` or `ADDRESS:WEIGHT` (weight 1 by default).
    /// Repeat for several owners. Defaults to the account's own key.
    #[arg(long = "owner", value_name = "OWNER", value_parser = parse_owner)]
    owners: Vec<ConfigurableAccountOwner>,

    /// The total owner weight that must approve a transaction.
    #[arg(long, default_value_t = 1)]
    threshold: u8,

    /// Upgrade without asking for confirmation.
    #[arg(long)]
    force: bool,

    #[command(flatten)]
    tx: TransactionOpts,

    #[command(flatten)]
    send_tx: SendTxOpts,
}

impl UpgradeArgs {
    pub async fn run(self) -> Result<()> {
        let Self { mut owners, threshold, force, tx, send_tx } = self;
        eyre::ensure!(
            tx.tempo.session_id()?.is_none(),
            "the upgrade needs the account's own key; Tempo Accounts sessions cannot sign it"
        );
        let signer = send_tx.eth.wallet.signer().await?;
        let account = signer.address();
        if let Some(from) = send_tx.eth.wallet.from {
            eyre::ensure!(
                from == account,
                "--from {from} does not match the signer {account}; the upgrade converts the \
                 signer's account"
            );
        }
        if owners.is_empty() {
            owners.push(ConfigurableAccountOwner { owner: account, weight: 1 });
        }
        owners.sort_by_key(|owner| owner.owner);
        let config = ConfigurableAccountConfig {
            salt: keccak256([CONFIGURABLE_ACCOUNT_UPGRADE_DOMAIN, account.as_slice()].concat()),
            version: 1,
            threshold,
            owners,
        };
        config.validate().map_err(|err| eyre::eyre!("invalid configuration: {err}"))?;

        let (_, provider) = tempo_provider(&send_tx.eth)?;
        if current_config(&provider, account, &[]).await?.is_some() {
            eyre::bail!("{account} is already a configurable account");
        }

        sh_status!("Upgrading {account} to a configurable account:")?;
        sh_status!("  Threshold: {}", config.threshold)?;
        for owner in &config.owners {
            sh_status!("  Owner:     {} (weight {})", owner.owner, owner.weight)?;
        }
        sh_warn!(
            "The upgrade is irreversible: the key of {account} can no longer sign for it, only \
             its owners can. The account can also no longer act as an MPP voucher signer: \
             opening or topping up payment channels reverts with RootKeyRetired. Keep an EOA for \
             MPP."
        )?;
        if !force {
            if shell::is_quiet() {
                eyre::bail!("the upgrade requires confirmation; pass `--force` with `--quiet`");
            }
            if !confirm_continue()? {
                return Ok(());
            }
        }

        let calldata = IConfigurableAccount::upgradeAccountCall {
            threshold: config.threshold,
            owners: abi_owners(&config),
        }
        .abi_encode();
        send_precompile_tx_with_root_signer(
            CONFIGURABLE_ACCOUNT_ADDRESS,
            calldata,
            tx,
            &send_tx,
            KeychainRootSigner::Wallet(Box::new(signer)),
            force,
            || Ok(()),
        )
        .await?;
        Ok(())
    }
}

/// `cast wallet assemble`: combines owner approvals with an unsigned transaction.
#[derive(Debug, Parser)]
pub struct AssembleArgs {
    /// The unsigned transaction, as printed by `cast mktx --raw-unsigned --tempo.digest`.
    unsigned_tx: String,

    /// Owner approvals of the digest that `cast mktx --tempo.digest` prints.
    ///
    /// Each one is a hex signature from `cast wallet sign --no-hash` (or a hex Tempo P-256 or
    /// WebAuthn signature), a JSON signature, or a WebAuthn assertion made with a passkey in a
    /// browser: a JSON object with `authenticatorData`, `clientDataJSON` and `signature`
    /// (top-level or under `response`) and the credential's `publicKey`, as base64url or hex.
    /// The assertion's challenge is the digest. A value naming a file is read from it.
    #[arg(required = true, value_name = "SIGNATURE")]
    signatures: Vec<String>,

    /// The configurable account that sends the transaction.
    #[arg(long, value_name = "ACCOUNT")]
    from: Address,

    #[command(flatten)]
    rpc: RpcOpts,
}

impl AssembleArgs {
    pub async fn run(self) -> Result<()> {
        let Self { unsigned_tx, signatures, from: account, rpc } = self;
        let tx = decode_unsigned_tx(&unsigned_tx)?;
        let approvals = signatures
            .iter()
            .enumerate()
            .map(|(index, value)| {
                parse_owner_signature(value)
                    .wrap_err_with(|| format!("invalid signature {}", index + 1))
            })
            .collect::<Result<Vec<_>>>()?;

        // A 1-of-1, salt-0 account is recomputed from its approval of the version-zero digest.
        let inner = tx.signature_hash();
        let initial_digest = configurable_account_digest(inner, account, 0);
        let keys = approvals
            .iter()
            .filter_map(|approval| approval.recover_signer(&initial_digest).ok())
            .collect::<Vec<_>>();
        let (_, provider) = tempo_provider(&rpc)?;
        let config = current_config(&provider, account, &keys).await?.ok_or_else(|| {
            eyre::eyre!(
                "{account} is not a registered configurable account, and no approval derives it"
            )
        })?;

        let signature = combine_approvals(account, &config, inner, approvals)?;
        let raw =
            TempoTxEnvelope::AA(tx.into_signed(TempoSignature::ConfigurableAccount(signature)))
                .encoded_2718();
        // The envelope must decode as the node will.
        TempoTxEnvelope::decode_2718_exact(&raw)?;
        crate::cmd::print_result_line(hex::encode_prefixed(raw))
    }
}

/// Returns the owner digest and the configuration-change quorum for `account`, used by
/// `cast mktx --tempo.digest`.
///
/// An unregistered account's configuration is only known to its owners, so its quorum cannot be
/// simulated: it needs an explicit `gas_limit`.
pub(crate) async fn digest_context(
    eth: &impl LoadConfig,
    account: Address,
    gas_limit: bool,
) -> Result<(u64, Option<ConfigurableAccountSimulationSpec>)> {
    let (_, provider) = tempo_provider(eth)?;
    match current_config(&provider, account, &[]).await? {
        Some(config) => Ok((config.version, Some(simulation_spec(account, &config, &[])))),
        None => {
            eyre::ensure!(
                provider.get_transaction_count(account).await? == 0,
                "{account} is not a configurable account"
            );
            eyre::ensure!(
                gas_limit,
                "{account} is not registered yet, so its owners and the gas of its registration \
                 are unknown. Send its first transaction with `cast wallet owners`, or pass \
                 --gas-limit"
            );
            sh_warn!("{account} is not registered yet: the digest is for configuration version 0")?;
            Ok((0, None))
        }
    }
}
