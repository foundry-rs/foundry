use alloy_primitives::{Address, B256, U256};
use alloy_provider::Provider;
use alloy_signer::Signer;
use alloy_sol_types::SolCall;
use clap::{Args, Parser};
use eyre::{Context, Result};
use foundry_cli::{
    opts::{TEMPO_SESSION_ID_ENV, TransactionOpts},
    utils::{LoadConfig, parse_fee_token_address},
};
use foundry_common::{
    provider::ProviderBuilder,
    sh_println, shell,
    tempo::{
        GeneratedSessionKey, SessionAuthorizationRequest, SessionEntry, SessionSpendLimit,
        SessionStatus, read_session_entry, update_session_status, update_session_status_if,
        upsert_session_entry,
    },
};
use foundry_wallets::{WalletOpts, WalletSigner};
use serde_json::json;
use std::{
    num::NonZeroU64,
    process::{Command, ExitStatus},
    time::{SystemTime, UNIX_EPOCH},
};
use tempo_alloy::{TempoNetwork, provider::TempoProviderExt};
use tempo_contracts::precompiles::IAccountKeychain;
use tempo_primitives::transaction::{CallScope, PrimitiveSignature, SelectorRule};
use tokio::signal;

use crate::{
    cmd::{
        keychain::{
            KeychainTxOutcome, resolve_keychain_root_signer, send_keychain_tx_with_root_signer,
        },
        tempo_policy_args::{
            parse_period, parse_scope as parse_policy_scope, parse_selector_bytes,
        },
    },
    tx::SendTxOpts,
};

use super::process_tree::ManagedChild;

const PRINT_SPONSOR_HASH_REVOKE_ERROR: &str = "--tempo.print-sponsor-hash only prints a sponsor hash and does not revoke the session on-chain";
const SESSION_CHILD_SIGNER_ENV: &[&str] = &[
    "ETH_KEYSTORE",
    "ETH_KEYSTORE_ACCOUNT",
    "ETH_PASSWORD",
    "TEMPO_ACCESS_KEY",
    "TEMPO_ROOT_ACCOUNT",
];

/// Arguments for `cast wallet session`.
///
/// Without a subcommand, this runs an issue-style temporary session around `--for <COMMAND>`.
/// The existing `create` and `revoke` subcommands remain explicit lifecycle operations.
#[derive(Debug, Args)]
#[command(args_conflicts_with_subcommands = true)]
pub struct SessionArgs {
    #[command(subcommand)]
    pub command: Option<SessionSubcommands>,

    /// Root account that will authorize the temporary session.
    #[arg(long = "root", value_name = "ADDRESS")]
    pub root_account: Option<Address>,

    /// Session lifetime, expressed as a duration like `10m`, `2h`, or `7d`.
    #[arg(long = "expires", id = "session_expires", value_name = "DURATION", value_parser = parse_period)]
    pub expires: Option<u64>,

    /// Allowed call scope, in `TARGET[:SELECTORS[@RECIPIENTS]]` format.
    #[arg(long = "scope", value_parser = parse_scope)]
    pub scope: Vec<CallScope>,

    /// Allowed call target for issue-style `--target ... --selector ...` input.
    #[arg(long = "target", value_name = "ADDRESS")]
    pub target: Option<Address>,

    /// Function selector allowed for `--target`, such as `register(address)`.
    #[arg(long = "selector", value_name = "SELECTOR")]
    pub selectors: Vec<String>,

    /// Token spend limit, in `TOKEN:AMOUNT` or `TOKEN=AMOUNT` format.
    #[arg(long = "spend-limit", value_parser = parse_spend_limit)]
    pub spend_limits: Vec<SessionSpendLimit>,

    /// Command to run with the temporary Tempo session.
    #[arg(long = "for", value_name = "COMMAND")]
    pub for_command: Option<String>,

    #[command(flatten)]
    pub tx: Box<TransactionOpts>,

    #[command(flatten)]
    pub send_tx: Box<SendTxOpts>,
}

impl SessionArgs {
    pub async fn run(self) -> Result<()> {
        let Self {
            command,
            root_account,
            expires,
            scope,
            target,
            selectors,
            spend_limits,
            for_command,
            tx,
            send_tx,
        } = self;

        if let Some(command) = command {
            return command.run().await;
        }

        let root_account =
            root_account.ok_or_else(|| eyre::eyre!("cast wallet session requires --root"))?;
        let expires =
            expires.ok_or_else(|| eyre::eyre!("cast wallet session requires --expires"))?;
        let command =
            for_command.ok_or_else(|| eyre::eyre!("cast wallet session requires --for"))?;
        let command = InnerCommand::parse(command)?;
        let scope = session_scope(scope, target, selectors)?;
        let send_tx = *send_tx;
        let chain_id = resolve_session_chain_id(&send_tx).await?;

        let tx = *tx;
        if tx.tempo.print_sponsor_hash {
            eyre::bail!(PRINT_SPONSOR_HASH_REVOKE_ERROR);
        }

        let entry = build_session_entry(
            root_account,
            chain_id,
            expires,
            scope,
            spend_limits,
            send_tx.eth.wallet.clone(),
        )
        .await?;

        run_for_command(entry, command, tx, send_tx).await
    }
}

/// Tempo wallet session lifecycle commands.
#[derive(Debug, Parser)]
pub enum SessionSubcommands {
    /// Create a temporary Tempo session and persist it locally.
    Create {
        /// Root account that will authorize the session.
        #[arg(long = "root", value_name = "ADDRESS")]
        root_account: Address,

        /// Chain ID the session is valid on.
        #[arg(long = "chain-id", value_name = "CHAIN_ID")]
        chain_id: u64,

        /// Session lifetime, expressed as a duration like `10m`, `2h`, or `7d`.
        #[arg(long = "expires", value_name = "DURATION", value_parser = parse_period)]
        expires: u64,

        /// Allowed call scope, in `TARGET[:SELECTORS[@RECIPIENTS]]` format.
        #[arg(long = "scope", value_parser = parse_scope, required = true)]
        scope: Vec<CallScope>,

        /// Token spend limit, in `TOKEN:AMOUNT` or `TOKEN=AMOUNT` format.
        #[arg(long = "spend-limit", value_parser = parse_spend_limit)]
        spend_limits: Vec<SessionSpendLimit>,

        #[command(flatten)]
        wallet: Box<WalletOpts>,
    },

    /// Revoke a Tempo session key on-chain when provisioned, then clear local key material.
    Revoke {
        /// Session identifier to revoke.
        #[arg(value_name = "SESSION_ID")]
        session_id: B256,

        /// Only clear local session key material; do not query or submit an on-chain revoke.
        #[arg(long)]
        local: bool,

        #[command(flatten)]
        tx: Box<TransactionOpts>,

        #[command(flatten)]
        send_tx: Box<SendTxOpts>,
    },
}

impl SessionSubcommands {
    pub async fn run(self) -> Result<()> {
        match self {
            Self::Create { root_account, chain_id, expires, scope, spend_limits, wallet } => {
                run_create(root_account, chain_id, expires, scope, spend_limits, *wallet).await
            }
            Self::Revoke { session_id, local, tx, send_tx } => {
                run_revoke(session_id, local, *tx, *send_tx).await
            }
        }
    }
}

async fn run_for_command(
    entry: SessionEntry,
    command: InnerCommand,
    tx: TransactionOpts,
    send_tx: SendTxOpts,
) -> Result<()> {
    let session_id = entry.session_id;
    upsert_session_entry(entry)?;

    let child_result = command.run(session_id).await;
    let cleanup_result = cleanup_session_run(session_id, child_result.is_ok(), tx, send_tx).await;

    finish_session_run(session_id, child_result, cleanup_result)
}

async fn cleanup_session_run(
    session_id: B256,
    child_succeeded: bool,
    tx: TransactionOpts,
    send_tx: SendTxOpts,
) -> Result<()> {
    let retire_result = if child_succeeded {
        mark_session_run_revoking(session_id)
    } else {
        retire_session_run_locally(session_id)
    };
    let revoke_result =
        run_revoke_with_policy(session_id, false, tx, send_tx, UnprovisionedKeyPolicy::Fail).await;

    match (retire_result, revoke_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(retire_err), Ok(())) => Err(retire_err),
        (Ok(()), Err(revoke_err)) => Err(revoke_err),
        (Err(retire_err), Err(revoke_err)) => {
            Err(revoke_err
                .wrap_err(format!("also failed to retire local Tempo session: {retire_err}")))
        }
    }
}

fn mark_session_run_revoking(session_id: B256) -> Result<()> {
    update_session_run_status(session_id, SessionStatus::Revoking)
        .wrap_err_with(|| format!("failed to mark Tempo session {session_id:?} as revoking"))
}

fn retire_session_run_locally(session_id: B256) -> Result<()> {
    update_session_run_status(session_id, SessionStatus::Failed)
        .wrap_err_with(|| format!("failed to retire local Tempo session {session_id:?}"))
}

fn update_session_run_status(session_id: B256, status: SessionStatus) -> Result<()> {
    let Some(entry) = read_session_entry(session_id)? else {
        return Ok(());
    };
    let status = if entry.status.is_terminal() { entry.status } else { status };
    update_session_status(session_id, status).map(|_| ())
}

fn finish_session_run(
    session_id: B256,
    child_result: Result<()>,
    revoke_result: Result<()>,
) -> Result<()> {
    match (child_result, revoke_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(child_err), Ok(())) => Err(child_err),
        (Ok(()), Err(revoke_err)) => {
            Err(revoke_err.wrap_err("failed to clean up Tempo session after inner command"))
        }
        (Err(child_err), Err(revoke_err)) => Err(child_err.wrap_err(format!(
            "also failed to clean up Tempo session {session_id:?}: {revoke_err}"
        ))),
    }
}

#[derive(Debug)]
struct InnerCommand {
    raw: String,
    program: String,
    args: Vec<String>,
}

impl InnerCommand {
    fn parse(raw: String) -> Result<Self> {
        let mut argv = split_for_command(&raw)?.into_iter();
        let program = argv.next().ok_or_else(|| eyre::eyre!("--for command cannot be empty"))?;
        let args = argv.collect();
        Ok(Self { raw, program, args })
    }

    async fn run(&self, session_id: B256) -> Result<()> {
        let mut interrupt = SessionInterrupt::new()?;
        self.run_with_interrupt(session_id, interrupt.recv()).await
    }

    async fn run_with_interrupt<I>(&self, session_id: B256, interrupt: I) -> Result<()>
    where
        I: std::future::Future<Output = Result<&'static str>>,
    {
        let mut child = ManagedChild::spawn(self.command(session_id))
            .wrap_err_with(|| format!("failed to run inner command `{}`", self.raw))?;

        let status = tokio::select! {
            status = child.wait() => status.wrap_err_with(|| {
                format!("failed to wait for inner command `{}`", self.raw)
            })?,
            interrupt = interrupt => {
                let _ = child.terminate_tree().await;

                return match interrupt {
                    Ok(interrupt) => Err(self.interrupted_error(interrupt)),
                    Err(err) => Err(err),
                };
            }
        };

        let _ = child.terminate_tree().await;

        if status.success() { Ok(()) } else { Err(self.status_error(status)) }
    }

    fn command(&self, session_id: B256) -> Command {
        let mut command = Command::new(&self.program);
        command.args(&self.args);
        for key in SESSION_CHILD_SIGNER_ENV {
            command.env_remove(key);
        }
        command.env(TEMPO_SESSION_ID_ENV, format!("{session_id:?}"));
        command
    }

    fn status_error(&self, status: ExitStatus) -> eyre::Report {
        match status.code() {
            Some(code) => eyre::eyre!("inner command `{}` exited with code {code}", self.raw),
            None => eyre::eyre!("inner command `{}` terminated by a signal", self.raw),
        }
    }

    fn interrupted_error(&self, interrupt: &'static str) -> eyre::Report {
        eyre::eyre!("inner command `{}` interrupted by {interrupt}", self.raw)
    }
}

#[cfg(unix)]
struct SessionInterrupt {
    sigint: signal::unix::Signal,
    sigterm: signal::unix::Signal,
}

#[cfg(unix)]
impl SessionInterrupt {
    fn new() -> Result<Self> {
        Ok(Self {
            sigint: signal::unix::signal(signal::unix::SignalKind::interrupt())
                .wrap_err("failed to listen for SIGINT")?,
            sigterm: signal::unix::signal(signal::unix::SignalKind::terminate())
                .wrap_err("failed to listen for SIGTERM")?,
        })
    }

    async fn recv(&mut self) -> Result<&'static str> {
        tokio::select! {
            _ = self.sigint.recv() => Ok("SIGINT"),
            _ = self.sigterm.recv() => Ok("SIGTERM"),
        }
    }
}

#[cfg(not(unix))]
struct SessionInterrupt;

#[cfg(not(unix))]
impl SessionInterrupt {
    fn new() -> Result<Self> {
        Ok(Self)
    }

    async fn recv(&mut self) -> Result<&'static str> {
        signal::ctrl_c().await.wrap_err("failed to listen for Ctrl-C")?;
        Ok("Ctrl-C")
    }
}

async fn resolve_session_chain_id(send_tx: &SendTxOpts) -> Result<u64> {
    let config = send_tx.eth.load_config()?;
    if let Some(chain) = config.chain {
        return Ok(chain.id());
    }

    let provider = ProviderBuilder::<TempoNetwork>::from_config(&config)?.build()?;
    provider.get_chain_id().await.wrap_err(
        "failed to resolve session chain id from RPC; pass --chain/--chain-id or --rpc-url",
    )
}

fn session_scope(
    mut scope: Vec<CallScope>,
    target: Option<Address>,
    selectors: Vec<String>,
) -> Result<Vec<CallScope>> {
    if !selectors.is_empty() && target.is_none() {
        eyre::bail!("--selector requires --target");
    }
    if target.is_some() && selectors.is_empty() {
        eyre::bail!(
            "--target requires at least one --selector; use --scope TARGET for target-wide access"
        );
    }

    if let Some(target) = target {
        let selector_rules = selectors
            .into_iter()
            .map(|selector| {
                parse_selector_bytes(&selector)
                    .map(|selector| SelectorRule { selector, recipients: vec![] })
                    .map_err(|err| eyre::eyre!("{err}"))
            })
            .collect::<Result<Vec<_>>>()?;
        scope.push(CallScope { target, selector_rules });
    }

    if scope.is_empty() {
        eyre::bail!("cast wallet session requires --scope or --target");
    }

    Ok(scope)
}

fn split_for_command(command: &str) -> Result<Vec<String>> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;
    let mut in_token = false;

    for ch in command.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            in_token = true;
            continue;
        }

        match quote {
            Some('\'') => {
                if ch == '\'' {
                    quote = None;
                } else {
                    current.push(ch);
                }
            }
            Some('"') => {
                if ch == '"' {
                    quote = None;
                } else if ch == '\\' {
                    escaped = true;
                } else {
                    current.push(ch);
                }
            }
            Some(_) => unreachable!(),
            None if ch.is_whitespace() => {
                if in_token {
                    args.push(std::mem::take(&mut current));
                    in_token = false;
                }
            }
            None if ch == '\'' || ch == '"' => {
                quote = Some(ch);
                in_token = true;
            }
            None if ch == '\\' => {
                escaped = true;
                in_token = true;
            }
            None => {
                current.push(ch);
                in_token = true;
            }
        }
    }

    if escaped {
        eyre::bail!("unterminated escape in --for command");
    }
    if let Some(quote) = quote {
        eyre::bail!("unterminated {quote} quote in --for command");
    }
    if in_token {
        args.push(current);
    }
    Ok(args)
}

/// Creates a signed session entry and stores it in the local registry.
async fn run_create(
    root_account: Address,
    chain_id: u64,
    expires: u64,
    scope: Vec<CallScope>,
    spend_limits: Vec<SessionSpendLimit>,
    wallet: WalletOpts,
) -> Result<()> {
    let entry =
        build_session_entry(root_account, chain_id, expires, scope, spend_limits, wallet).await?;
    let session_id = entry.session_id;
    let root_account = entry.root_account;
    let chain_id = entry.chain_id;
    let key_address = entry.key_address;
    let expiry = entry.expiry;
    let scope_count = entry.scope.as_ref().map_or(0, |scopes| scopes.len());
    let spend_limit_count = entry.limits.as_ref().map_or(0, |limits| limits.len());
    upsert_session_entry(entry)?;

    if shell::is_json() {
        sh_println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "session_id": session_id.to_string(),
                "root_account": root_account.to_string(),
                "chain_id": chain_id,
                "key_address": key_address.to_string(),
                "expiry": expiry,
                "status": "active",
                "scope_count": scope_count,
                "spend_limit_count": spend_limit_count,
            }))?
        )?;
    } else {
        sh_println!("Created Tempo session {}", session_id)?;
        sh_println!("Root:  {}", root_account)?;
        sh_println!("Chain: {}", chain_id)?;
        sh_println!("Key:   {}", key_address)?;
        sh_println!("Expiry: {}", expiry)?;
    }

    Ok(())
}

/// Revokes a session entry locally and on-chain when the key has been provisioned.
async fn run_revoke(
    session_id: B256,
    local: bool,
    tx: TransactionOpts,
    send_tx: SendTxOpts,
) -> Result<()> {
    run_revoke_with_policy(session_id, local, tx, send_tx, UnprovisionedKeyPolicy::RevokeLocally)
        .await
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UnprovisionedKeyPolicy {
    RevokeLocally,
    Fail,
}

async fn run_revoke_with_policy(
    session_id: B256,
    local: bool,
    tx: TransactionOpts,
    send_tx: SendTxOpts,
    unprovisioned_policy: UnprovisionedKeyPolicy,
) -> Result<()> {
    let Some(entry) = read_session_entry(session_id)? else {
        print_revoke_status(session_id, None, SessionRevokeStatus::NotFound)?;
        return Ok(());
    };

    if local {
        update_session_status(session_id, SessionStatus::Revoked)?;
        print_revoke_status(session_id, Some(&entry), SessionRevokeStatus::Local)?;
        return Ok(());
    }

    if tx.tempo.print_sponsor_hash {
        eyre::bail!(PRINT_SPONSOR_HASH_REVOKE_ERROR);
    }

    let config = send_tx.eth.load_config()?;
    let provider = ProviderBuilder::<TempoNetwork>::from_config(&config)?.build()?;
    let rpc_chain_id = provider.get_chain_id().await?;
    if rpc_chain_id != entry.chain_id {
        eyre::bail!(
            "session {} was created for chain {}, but the RPC is connected to chain {}",
            entry.session_id,
            entry.chain_id,
            rpc_chain_id
        );
    }

    let info = provider.get_keychain_key(entry.root_account, entry.key_address).await?;
    if info.isRevoked {
        update_session_status(session_id, SessionStatus::Revoked)?;
        print_revoke_status(session_id, Some(&entry), SessionRevokeStatus::AlreadyRevoked)?;
        return Ok(());
    }
    if info.keyId == Address::ZERO {
        return handle_unprovisioned_revoke(session_id, &entry, unprovisioned_policy);
    }

    let root_signer =
        resolve_keychain_root_signer(&send_tx, Some(entry.root_account), false).await?;
    let revoke_result = async {
        let calldata = IAccountKeychain::revokeKeyCall { keyId: entry.key_address }.abi_encode();
        let before_submit = || {
            if entry.status != SessionStatus::Revoked {
                update_session_status_if(session_id, entry.status, SessionStatus::Revoking)?;
            }
            Ok(())
        };
        match send_keychain_tx_with_root_signer(calldata, tx, &send_tx, root_signer, before_submit)
            .await?
        {
            KeychainTxOutcome::Submitted => {}
            KeychainTxOutcome::PrintedSponsorHash => {
                eyre::bail!(PRINT_SPONSOR_HASH_REVOKE_ERROR);
            }
        }
        Ok(())
    }
    .await;
    if let Err(err) = revoke_result {
        handle_revoke_error(&provider, session_id, &entry).await;
        return Err(err.wrap_err("failed to revoke Tempo session key on-chain"));
    }

    update_session_status(session_id, SessionStatus::Revoked)?;

    Ok(())
}

fn handle_unprovisioned_revoke(
    session_id: B256,
    entry: &SessionEntry,
    policy: UnprovisionedKeyPolicy,
) -> Result<()> {
    match policy {
        UnprovisionedKeyPolicy::RevokeLocally => {
            update_session_status(session_id, SessionStatus::Revoked)?;
            print_revoke_status(session_id, Some(entry), SessionRevokeStatus::NotProvisioned)?;
            Ok(())
        }
        UnprovisionedKeyPolicy::Fail => {
            eyre::bail!(
                "session key is not provisioned on-chain yet; pending transactions from the \
                 wrapped command may still provision it. Wait for pending transactions to settle, \
                 then run `cast wallet session revoke {session_id}`."
            );
        }
    }
}

async fn handle_revoke_error(
    provider: &impl Provider<TempoNetwork>,
    session_id: B256,
    entry: &SessionEntry,
) {
    if provider
        .get_keychain_key(entry.root_account, entry.key_address)
        .await
        .map(|info| info.isRevoked)
        .unwrap_or(false)
    {
        let _ = update_session_status(session_id, SessionStatus::Revoked);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SessionRevokeStatus {
    NotFound,
    Local,
    NotProvisioned,
    AlreadyRevoked,
}

fn print_revoke_status(
    session_id: B256,
    entry: Option<&SessionEntry>,
    status: SessionRevokeStatus,
) -> Result<()> {
    if shell::is_json() {
        sh_println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "session_id": session_id.to_string(),
                "status": if status == SessionRevokeStatus::NotFound { "not_found" } else { "revoked" },
                "reason": match status {
                    SessionRevokeStatus::NotFound => "not_found",
                    SessionRevokeStatus::Local => "local",
                    SessionRevokeStatus::NotProvisioned => "not_provisioned",
                    SessionRevokeStatus::AlreadyRevoked => "already_revoked",
                },
                "root_account": entry.map(|entry| entry.root_account.to_string()),
                "chain_id": entry.map(|entry| entry.chain_id),
                "key_address": entry.map(|entry| entry.key_address.to_string()),
            }))?
        )?;
        return Ok(());
    }

    match status {
        SessionRevokeStatus::NotFound => {
            sh_status!("Tempo session {} was not found.", session_id)?;
        }
        SessionRevokeStatus::Local => {
            sh_status!("Revoked local Tempo session {}", session_id)?;
        }
        SessionRevokeStatus::NotProvisioned => {
            sh_status!(
                "Revoked Tempo session {} locally; key was not provisioned on-chain",
                session_id
            )?;
        }
        SessionRevokeStatus::AlreadyRevoked => {
            sh_status!(
                "Revoked Tempo session {} locally; key was already revoked on-chain",
                session_id
            )?;
        }
    }

    Ok(())
}

/// Builds an active session entry from CLI policy inputs and a root signature.
async fn build_session_entry(
    root_account: Address,
    chain_id: u64,
    expires: u64,
    scope: Vec<CallScope>,
    spend_limits: Vec<SessionSpendLimit>,
    wallet: WalletOpts,
) -> Result<foundry_common::tempo::SessionEntry> {
    if expires == 0 {
        eyre::bail!("--expires must be greater than 0");
    }
    if chain_id == 0 {
        eyre::bail!("--chain-id must be greater than 0");
    }
    if wallet.from.is_some_and(|from| from != root_account) {
        eyre::bail!("--from must match --root for cast wallet session create");
    }

    let signer = resolve_root_signer(wallet, root_account).await?;
    let session_key = GeneratedSessionKey::random();
    let session_id = B256::random();
    let now = now_unix_timestamp()?;
    let expiry = now
        .checked_add(expires)
        .ok_or_else(|| eyre::eyre!("session expiry overflows the unix timestamp range"))?;
    let expiry =
        NonZeroU64::new(expiry).ok_or_else(|| eyre::eyre!("session expiry cannot be zero"))?;

    let request = SessionAuthorizationRequest {
        session_id,
        root_account,
        chain_id,
        key_address: session_key.address(),
        expiry,
        scope,
        spend_limits,
    };
    let prepared = request.prepare(now)?;
    let signature = signer.sign_hash(&prepared.authorization.signature_hash()).await?;
    let signed_authorization =
        prepared.authorization.clone().into_signed(PrimitiveSignature::Secp256k1(signature));
    prepared.into_active_entry(session_key, &signed_authorization)
}

async fn resolve_root_signer(wallet: WalletOpts, root_account: Address) -> Result<WalletSigner> {
    let (signer, tempo_access_key) = wallet.maybe_signer().await?;
    if tempo_access_key.is_some() {
        eyre::bail!(
            "Tempo access keys cannot authorize Tempo sessions; use a persistent root signer"
        );
    }

    let signer = signer.ok_or_else(|| eyre::eyre!("a root wallet signer is required"))?;
    let signer_address = signer.address();
    if signer_address != root_account {
        eyre::bail!("resolved signer {} does not match --root {}", signer_address, root_account);
    }

    Ok(signer)
}

/// Adapts shared keychain scope parsing into the session authorization type.
fn parse_scope(s: &str) -> Result<CallScope, String> {
    parse_policy_scope(s).map(|scope| CallScope {
        target: scope.target,
        selector_rules: scope
            .selectorRules
            .into_iter()
            .map(|rule| SelectorRule {
                selector: rule.selector.into(),
                recipients: rule.recipients,
            })
            .collect(),
    })
}

/// Parses a session spend limit into the session policy model.
fn parse_spend_limit(s: &str) -> Result<SessionSpendLimit, String> {
    let Some((token_str, amount_str)) = s.split_once(':').or_else(|| s.split_once('=')) else {
        return Err(format!("invalid limit format: {s} (expected TOKEN:AMOUNT or TOKEN=AMOUNT)"));
    };

    let token = parse_fee_token_address(token_str.trim()).map_err(|e| e.to_string())?;
    let amount: U256 =
        amount_str.trim().parse().map_err(|e| format!("invalid amount '{amount_str}': {e}"))?;
    Ok(SessionSpendLimit { token, amount })
}

fn now_unix_timestamp() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system time is before UNIX_EPOCH")?
        .as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::address;
    use foundry_cli::opts::EthereumOpts;
    use std::{ffi::OsStr, sync::Mutex};
    use tempo_contracts::precompiles::PATH_USD_ADDRESS;

    const ROOT_PRIVATE_KEY: &str =
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    fn with_tempo_home(test: impl FnOnce()) {
        let _guard = ENV_MUTEX.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        // SAFETY: tests serialize all Tempo environment mutation through the mutex.
        unsafe { std::env::set_var("TEMPO_HOME", tmp.path()) };
        test();
        // SAFETY: restore the process environment after the critical section.
        unsafe { std::env::remove_var("TEMPO_HOME") };
    }

    #[test]
    fn session_revoke_is_idempotent_when_missing() {
        with_tempo_home(|| {
            let session_id = B256::from([0x42; 32]);
            assert!(!update_session_status(session_id, SessionStatus::Revoked).unwrap());
        });
    }

    #[test]
    fn parse_spend_limit_accepts_fee_token_symbol() {
        let limit = parse_spend_limit("PathUSD=0").unwrap();
        assert_eq!(limit.token, PATH_USD_ADDRESS);
        assert_eq!(limit.amount, U256::ZERO);
    }

    #[test]
    fn inner_command_parse_preserves_literal_argv() {
        let raw =
            r#"forge script "Deploy Script" --sig 'run(uint256)' value\ with\ spaces #literal"#;
        let command = InnerCommand::parse(raw.to_string()).unwrap();

        assert_eq!(command.raw, raw);
        assert_eq!(command.program, "forge");
        assert_eq!(
            command.args,
            ["script", "Deploy Script", "--sig", "run(uint256)", "value with spaces", "#literal",]
        );
    }

    #[test]
    fn inner_command_parse_rejects_invalid_input() {
        let err = InnerCommand::parse("   ".to_string()).unwrap_err();
        assert!(err.to_string().contains("--for command cannot be empty"), "{err}");

        let err = InnerCommand::parse("forge 'script".to_string()).unwrap_err();
        assert!(err.to_string().contains("unterminated"), "{err}");
    }

    #[test]
    fn session_scope_requires_selector_for_target_shortcut() {
        let target = address!("0x00000000000000000000000000000000000000aa");
        let err = session_scope(vec![], Some(target), vec![]).unwrap_err();

        assert!(err.to_string().contains("--target requires at least one --selector"), "{err}");
    }

    #[test]
    fn session_scope_preserves_explicit_scope_target_wildcard() {
        let target = address!("0x00000000000000000000000000000000000000aa");
        let scope = vec![CallScope { target, selector_rules: vec![] }];

        assert_eq!(session_scope(scope.clone(), None, vec![]).unwrap(), scope);
    }

    #[test]
    fn inner_command_clears_inherited_signer_env_for_session_child() {
        let session_id = B256::from([0x7a; 32]);
        let command = InnerCommand::parse("forge script Deploy".to_string()).unwrap();
        let child = command.command(session_id);

        for key in SESSION_CHILD_SIGNER_ENV {
            assert_eq!(
                command_env(&child, key),
                Some(None),
                "expected {key} to be removed from session child environment"
            );
        }

        let expected_session_id = format!("{session_id:?}");
        assert_eq!(
            command_env(&child, TEMPO_SESSION_ID_ENV),
            Some(Some(OsStr::new(&expected_session_id)))
        );
        assert_eq!(
            command_env(&child, "ETH_FROM"),
            None,
            "ETH_FROM is a sender hint and should not be stripped by session --for"
        );
    }

    #[cfg(unix)]
    #[test]
    fn inner_command_interrupt_terminates_child() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let session_id = B256::from([0x7b; 32]);
            let command = InnerCommand::parse("sh -c 'sleep 30'".to_string()).unwrap();
            let err = command
                .run_with_interrupt(session_id, std::future::ready(Ok("test interrupt")))
                .await
                .unwrap_err();

            assert!(err.to_string().contains("interrupted by test interrupt"), "{err}");
        });
    }

    fn command_env<'a>(command: &'a Command, key: &str) -> Option<Option<&'a OsStr>> {
        command.get_envs().find_map(|(name, value)| (name == key).then_some(value))
    }

    #[test]
    fn explicit_revoke_preflight_error_preserves_local_key_material_for_retry() {
        with_tempo_home(|| {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.block_on(async {
                let session_id = B256::from([0xd0; 32]);
                let entry = sample_session_entry(session_id, SessionStatus::Active);
                upsert_session_entry(entry).unwrap();

                let mut send_tx = empty_send_tx_opts();
                send_tx.eth.rpc.common.rpc_url = Some("http://127.0.0.1:9".to_string());
                let err =
                    run_revoke(session_id, false, TransactionOpts::parse_from(["cast"]), send_tx)
                        .await
                        .unwrap_err();

                let session = read_session_entry(session_id).unwrap().unwrap();
                assert_eq!(session.status, SessionStatus::Active, "{err:#}");
                assert!(session.key.is_some());
            });
        });
    }

    #[test]
    fn run_for_success_marks_session_revoking_before_revoke_preflight() {
        with_tempo_home(|| {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.block_on(async {
                let session_id = B256::from([0xd7; 32]);
                upsert_session_entry(sample_session_entry(session_id, SessionStatus::Active))
                    .unwrap();

                let mut send_tx = empty_send_tx_opts();
                send_tx.eth.rpc.common.rpc_url = Some("http://127.0.0.1:9".to_string());
                let err = cleanup_session_run(
                    session_id,
                    true,
                    TransactionOpts::parse_from(["cast"]),
                    send_tx,
                )
                .await
                .unwrap_err();

                let session = read_session_entry(session_id).unwrap().unwrap();
                assert_eq!(session.status, SessionStatus::Revoking, "{err:#}");
                assert!(session.key.is_none());
            });
        });
    }

    #[test]
    fn run_for_retire_local_session_before_revoke_preflight() {
        with_tempo_home(|| {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.block_on(async {
                let session_id = B256::from([0xd4; 32]);
                upsert_session_entry(sample_session_entry(session_id, SessionStatus::Active))
                    .unwrap();

                retire_session_run_locally(session_id).unwrap();

                let mut send_tx = empty_send_tx_opts();
                send_tx.eth.rpc.common.rpc_url = Some("http://127.0.0.1:9".to_string());
                let err =
                    run_revoke(session_id, false, TransactionOpts::parse_from(["cast"]), send_tx)
                        .await
                        .unwrap_err();

                let session = read_session_entry(session_id).unwrap().unwrap();
                assert_eq!(session.status, SessionStatus::Failed, "{err:#}");
                assert!(session.key.is_none());
            });
        });
    }

    #[test]
    fn run_for_unprovisioned_cleanup_remains_retryable() {
        with_tempo_home(|| {
            let session_id = B256::from([0xd6; 32]);
            upsert_session_entry(sample_session_entry(session_id, SessionStatus::Active)).unwrap();

            retire_session_run_locally(session_id).unwrap();
            let entry = read_session_entry(session_id).unwrap().unwrap();
            let err = handle_unprovisioned_revoke(session_id, &entry, UnprovisionedKeyPolicy::Fail)
                .unwrap_err();

            assert!(err.to_string().contains("pending transactions"), "{err}");
            let session = read_session_entry(session_id).unwrap().unwrap();
            assert_eq!(session.status, SessionStatus::Failed);
            assert!(session.key.is_none());
        });
    }

    #[test]
    fn run_for_retire_local_session_does_not_downgrade_revoked_status() {
        with_tempo_home(|| {
            let session_id = B256::from([0xd5; 32]);
            upsert_session_entry(sample_session_entry(session_id, SessionStatus::Revoked)).unwrap();

            retire_session_run_locally(session_id).unwrap();

            let session = read_session_entry(session_id).unwrap().unwrap();
            assert_eq!(session.status, SessionStatus::Revoked);
            assert!(session.key.is_none());
        });
    }

    #[test]
    fn revoke_error_does_not_downgrade_existing_revoked_status() {
        with_tempo_home(|| {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.block_on(async {
                let session_id = B256::from([0xd1; 32]);
                upsert_session_entry(sample_session_entry(session_id, SessionStatus::Revoking))
                    .unwrap();
                update_session_status(session_id, SessionStatus::Revoked).unwrap();

                let mut send_tx = empty_send_tx_opts();
                send_tx.eth.rpc.common.rpc_url = Some("http://127.0.0.1:9".to_string());
                let config = send_tx.eth.load_config().unwrap();
                let provider =
                    ProviderBuilder::<TempoNetwork>::from_config(&config).unwrap().build().unwrap();
                handle_revoke_error(
                    &provider,
                    session_id,
                    &sample_session_entry(session_id, SessionStatus::Revoking),
                )
                .await;

                assert_eq!(
                    read_session_entry(session_id).unwrap().unwrap().status,
                    SessionStatus::Revoked
                );
            });
        });
    }

    #[test]
    fn revoke_submit_error_keeps_revoking_session_retryable() {
        with_tempo_home(|| {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.block_on(async {
                let session_id = B256::from([0xd3; 32]);
                let entry = sample_session_entry(session_id, SessionStatus::Active);
                upsert_session_entry(entry.clone()).unwrap();
                assert!(
                    update_session_status_if(
                        session_id,
                        SessionStatus::Active,
                        SessionStatus::Revoking,
                    )
                    .unwrap()
                );

                let mut send_tx = empty_send_tx_opts();
                send_tx.eth.rpc.common.rpc_url = Some("http://127.0.0.1:9".to_string());
                let config = send_tx.eth.load_config().unwrap();
                let provider =
                    ProviderBuilder::<TempoNetwork>::from_config(&config).unwrap().build().unwrap();
                handle_revoke_error(&provider, session_id, &entry).await;

                let session = read_session_entry(session_id).unwrap().unwrap();
                assert_eq!(session.status, SessionStatus::Revoking);
                assert!(session.key.is_none());
            });
        });
    }

    #[test]
    fn revoke_retry_preflight_error_does_not_downgrade_revoked_status() {
        with_tempo_home(|| {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.block_on(async {
                let session_id = B256::from([0xd2; 32]);
                upsert_session_entry(sample_session_entry(session_id, SessionStatus::Revoked))
                    .unwrap();

                let mut send_tx = empty_send_tx_opts();
                send_tx.eth.rpc.common.rpc_url = Some("http://127.0.0.1:9".to_string());
                let _ =
                    run_revoke(session_id, false, TransactionOpts::parse_from(["cast"]), send_tx)
                        .await
                        .unwrap_err();

                assert_eq!(
                    read_session_entry(session_id).unwrap().unwrap().status,
                    SessionStatus::Revoked
                );
            });
        });
    }

    #[test]
    fn create_and_local_revoke_session_entry_round_trips() {
        with_tempo_home(|| {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.block_on(async {
                let root = address!("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
                let private_key = ROOT_PRIVATE_KEY.to_string();
                let wallet = WalletOpts {
                    raw: foundry_wallets::RawWalletOpts {
                        private_key: Some(private_key),
                        ..Default::default()
                    },
                    ..Default::default()
                };

                let entry = build_session_entry(
                    root,
                    4217,
                    600,
                    vec![CallScope {
                        target: address!("0x00000000000000000000000000000000000000aa"),
                        selector_rules: vec![],
                    }],
                    vec![],
                    wallet,
                )
                .await
                .unwrap();
                assert_eq!(entry.status, SessionStatus::Active);
                assert!(entry.key.is_some());

                let session_id = entry.session_id;
                let expiry = entry.expiry;
                upsert_session_entry(entry).unwrap();
                let record = foundry_common::tempo::read_session_record().unwrap();
                assert_eq!(record.sessions.len(), 1);
                assert_eq!(record.sessions[0].session_id, session_id);
                assert!(record.sessions[0].has_live_key_at(expiry - 1));

                assert!(update_session_status(session_id, SessionStatus::Revoked).unwrap());
                let record = foundry_common::tempo::read_session_record().unwrap();
                let session = record.get(session_id).unwrap();
                assert_eq!(session.status, SessionStatus::Revoked);
                assert!(session.key.is_none());
            });
        });
    }

    fn empty_send_tx_opts() -> SendTxOpts {
        SendTxOpts {
            cast_async: false,
            sync: false,
            confirmations: 1,
            timeout: None,
            poll_interval: None,
            eth: EthereumOpts::default(),
            browser: Default::default(),
        }
    }

    fn sample_session_entry(session_id: B256, status: SessionStatus) -> SessionEntry {
        let key = match status {
            SessionStatus::Revoking
            | SessionStatus::Revoked
            | SessionStatus::Expired
            | SessionStatus::Failed => None,
            _ => Some(foundry_common::tempo::SessionKeyMaterial {
                key_type: foundry_common::tempo::KeyType::Secp256k1,
                key: ROOT_PRIVATE_KEY.to_string(),
                key_authorization: None,
            }),
        };

        SessionEntry {
            session_id,
            root_account: address!("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"),
            chain_id: 4217,
            key_address: address!("0x00000000000000000000000000000000000000bb"),
            expiry: 200,
            scope: None,
            limits: None,
            status,
            key,
        }
    }
}
