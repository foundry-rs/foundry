use alloy_primitives::{Address, B256, U256};
use alloy_provider::Provider;
use alloy_signer::Signer;
use alloy_sol_types::SolCall;
use clap::{Args, Parser};
use eyre::{Context, Result};
use foundry_cli::{
    opts::{TEMPO_SESSION_ID_ENV, TransactionOpts},
    utils::{LoadConfig, now, parse_fee_token_address},
};
use foundry_common::{
    provider::ProviderBuilder,
    sh_println, shell,
    tempo::{
        GeneratedSessionKey, SessionAuthorizationRequest, SessionEntry, SessionSpendLimit,
        read_session_entry, retire_session_entry, upsert_session_entry,
    },
};
use foundry_wallets::{WalletOpts, WalletSigner};
use serde_json::json;
use std::{
    num::NonZeroU64,
    process::{Command, ExitStatus},
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
        print_json_or,
        tempo_policy_args::{
            parse_period, parse_scope as parse_policy_scope, parse_selector_bytes,
        },
    },
    tempo,
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

    /// Skip the EIP-7702 authorization disclosure confirmation.
    #[arg(long)]
    pub force: bool,

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
            force,
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
        let session_id = entry.session_id;
        upsert_session_entry(entry)?;

        let child_result = command.run(session_id).await;

        // Always retire the local key material, then revoke on-chain; the on-chain error takes
        // precedence when both fail.
        let retire_result = retire_session_entry(session_id)
            .map(drop)
            .wrap_err_with(|| format!("failed to retire local Tempo session {session_id:?}"));
        let revoke_result =
            revoke(session_id, false, tx, send_tx, UnprovisionedKeyPolicy::Fail, force).await;
        let cleanup_result = match (retire_result, revoke_result) {
            (Ok(()), result) | (result, Ok(())) => result,
            (Err(retire_err), Err(revoke_err)) => Err(revoke_err
                .wrap_err(format!("also failed to retire local Tempo session: {retire_err}"))),
        };

        // The inner command's error takes precedence over cleanup failures.
        match (child_result, cleanup_result) {
            (Ok(()), Err(cleanup_err)) => {
                Err(cleanup_err.wrap_err("failed to clean up Tempo session after inner command"))
            }
            (Err(child_err), Err(cleanup_err)) => Err(child_err.wrap_err(format!(
                "also failed to clean up Tempo session {session_id:?}: {cleanup_err}"
            ))),
            (child_result, Ok(())) => child_result,
        }
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

        /// Skip the EIP-7702 authorization disclosure confirmation.
        #[arg(long)]
        force: bool,

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
                create(root_account, chain_id, expires, scope, spend_limits, *wallet).await
            }
            Self::Revoke { session_id, local, force, tx, send_tx } => {
                revoke(
                    session_id,
                    local,
                    *tx,
                    *send_tx,
                    UnprovisionedKeyPolicy::RevokeLocally,
                    force,
                )
                .await
            }
        }
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
                let interrupt = interrupt?;
                eyre::bail!("inner command `{}` interrupted by {interrupt}", self.raw);
            }
        };

        let _ = child.terminate_tree().await;

        self.check_status(status)
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

    fn check_status(&self, status: ExitStatus) -> Result<()> {
        if status.success() {
            return Ok(());
        }
        match status.code() {
            Some(code) => eyre::bail!("inner command `{}` exited with code {code}", self.raw),
            None => eyre::bail!("inner command `{}` terminated by a signal", self.raw),
        }
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
    match target {
        None if !selectors.is_empty() => eyre::bail!("--selector requires --target"),
        Some(_) if selectors.is_empty() => eyre::bail!(
            "--target requires at least one --selector; use --scope TARGET for target-wide access"
        ),
        Some(target) => {
            let selector_rules = selectors
                .iter()
                .map(|selector| {
                    parse_selector_bytes(selector)
                        .map(|selector| SelectorRule { selector, recipients: vec![] })
                        .map_err(|err| eyre::eyre!("{err}"))
                })
                .collect::<Result<Vec<_>>>()?;
            scope.push(CallScope { target, selector_rules });
        }
        None => {}
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

/// Creates a signed temporary access key in the Tempo Accounts store.
async fn create(
    root_account: Address,
    chain_id: u64,
    expires: u64,
    scope: Vec<CallScope>,
    spend_limits: Vec<SessionSpendLimit>,
    wallet: WalletOpts,
) -> Result<()> {
    let entry =
        build_session_entry(root_account, chain_id, expires, scope, spend_limits, wallet).await?;
    let json = json!({
        "session_id": entry.session_id.to_string(),
        "root_account": entry.root_account.to_string(),
        "chain_id": entry.chain_id,
        "key_address": entry.key_address.to_string(),
        "expiry": entry.expiry,
        "status": "active",
        "scope_count": entry.scope.as_ref().map_or(0, Vec::len),
        "spend_limit_count": entry.limits.as_ref().map_or(0, Vec::len),
    });
    let prose = format!(
        "Created Tempo session {}\nRoot:  {}\nChain: {}\nKey:   {}\nExpiry: {}",
        entry.session_id, entry.root_account, entry.chain_id, entry.key_address, entry.expiry
    );
    upsert_session_entry(entry)?;

    print_json_or(json, prose)
}

/// How to treat a session key that was never provisioned on-chain when revoking it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UnprovisionedKeyPolicy {
    /// Explicit `revoke`: mark the key revoked locally.
    RevokeLocally,
    /// Automatic `--for` cleanup: fail, since pending transactions may still provision it.
    Fail,
}

/// Revokes a session entry locally and on-chain when the key has been provisioned.
async fn revoke(
    session_id: B256,
    local: bool,
    tx: TransactionOpts,
    send_tx: SendTxOpts,
    unprovisioned_policy: UnprovisionedKeyPolicy,
    force: bool,
) -> Result<()> {
    let Some(entry) = read_session_entry(session_id)? else {
        return print_revoke_status(session_id, None, SessionRevokeStatus::NotFound);
    };

    if local {
        retire_session_entry(session_id)?;
        return print_revoke_status(session_id, Some(&entry), SessionRevokeStatus::Local);
    }

    if tx.tempo.print_sponsor_hash {
        eyre::bail!(PRINT_SPONSOR_HASH_REVOKE_ERROR);
    }

    let (_, provider) = tempo::tempo_provider(&send_tx.eth)?;
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
        retire_session_entry(session_id)?;
        return print_revoke_status(session_id, Some(&entry), SessionRevokeStatus::AlreadyRevoked);
    }
    if info.keyId == Address::ZERO {
        return match unprovisioned_policy {
            UnprovisionedKeyPolicy::RevokeLocally => {
                retire_session_entry(session_id)?;
                print_revoke_status(session_id, Some(&entry), SessionRevokeStatus::NotProvisioned)
            }
            UnprovisionedKeyPolicy::Fail => eyre::bail!(
                "session key is not provisioned on-chain yet; pending transactions from the \
                 wrapped command may still provision it. Wait for pending transactions to settle, \
                 then run `cast wallet session revoke {session_id}`."
            ),
        };
    }

    let root_signer =
        resolve_keychain_root_signer(&send_tx, Some(entry.root_account), false).await?;
    let calldata = IAccountKeychain::revokeKeyCall { keyId: entry.key_address }.abi_encode();
    let outcome =
        send_keychain_tx_with_root_signer(calldata, tx, &send_tx, root_signer, force, || {
            retire_session_entry(session_id).map(drop)
        })
        .await
        .and_then(|outcome| {
            if outcome == KeychainTxOutcome::PrintedSponsorHash {
                eyre::bail!(PRINT_SPONSOR_HASH_REVOKE_ERROR);
            }
            Ok(outcome)
        });
    let outcome = match outcome {
        Ok(outcome) => outcome,
        Err(err) => {
            // The key may have been revoked despite the error; retire the local copy if so.
            if provider
                .get_keychain_key(entry.root_account, entry.key_address)
                .await
                .is_ok_and(|info| info.isRevoked)
            {
                let _ = retire_session_entry(session_id);
            }
            return Err(err.wrap_err("failed to revoke Tempo session key on-chain"));
        }
    };

    if outcome == KeychainTxOutcome::Aborted {
        // Automatic cleanup uses `Fail` and must report an aborted on-chain revoke.
        if unprovisioned_policy == UnprovisionedKeyPolicy::Fail {
            eyre::bail!("EIP-7702 authorization disclosure was declined");
        }
        return Ok(());
    }

    retire_session_entry(session_id)?;
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SessionRevokeStatus {
    NotFound,
    Local,
    NotProvisioned,
    AlreadyRevoked,
}

impl SessionRevokeStatus {
    const fn reason(self) -> &'static str {
        match self {
            Self::NotFound => "not_found",
            Self::Local => "local",
            Self::NotProvisioned => "not_provisioned",
            Self::AlreadyRevoked => "already_revoked",
        }
    }
}

fn print_revoke_status(
    session_id: B256,
    entry: Option<&SessionEntry>,
    status: SessionRevokeStatus,
) -> Result<()> {
    if shell::is_json() {
        return sh_println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "session_id": session_id.to_string(),
                "status": if status == SessionRevokeStatus::NotFound { "not_found" } else { "revoked" },
                "reason": status.reason(),
                "root_account": entry.map(|entry| entry.root_account.to_string()),
                "chain_id": entry.map(|entry| entry.chain_id),
                "key_address": entry.map(|entry| entry.key_address.to_string()),
            }))?
        );
    }

    match status {
        SessionRevokeStatus::NotFound => sh_status!("Tempo session {session_id} was not found."),
        SessionRevokeStatus::Local => sh_status!("Revoked local Tempo session {session_id}"),
        SessionRevokeStatus::NotProvisioned => sh_status!(
            "Revoked Tempo session {session_id} locally; key was not provisioned on-chain"
        ),
        SessionRevokeStatus::AlreadyRevoked => sh_status!(
            "Revoked Tempo session {session_id} locally; key was already revoked on-chain"
        ),
    }
}

/// Builds an active session entry from CLI policy inputs and a root signature.
async fn build_session_entry(
    root_account: Address,
    chain_id: u64,
    expires: u64,
    scope: Vec<CallScope>,
    spend_limits: Vec<SessionSpendLimit>,
    wallet: WalletOpts,
) -> Result<SessionEntry> {
    if expires == 0 {
        eyre::bail!("--expires must be greater than 0");
    }
    if chain_id == 0 {
        eyre::bail!("--chain-id must be greater than 0");
    }
    if wallet.from.is_some_and(|from| from != root_account) {
        eyre::bail!("--from must match --root for cast wallet session create");
    }

    let signer = resolve_root_signer(wallet, root_account, chain_id).await?;
    let session_key = GeneratedSessionKey::random();
    let session_id = B256::random();
    let now_secs = now().as_secs();
    let expiry = now_secs
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
    let prepared = request.prepare(now_secs)?;
    let signature = signer.sign_hash(&prepared.authorization.signature_hash()).await?;
    let signed_authorization =
        prepared.authorization.clone().into_signed(PrimitiveSignature::Secp256k1(signature));
    prepared.into_active_entry(session_key, &signed_authorization)
}

async fn resolve_root_signer(
    wallet: WalletOpts,
    root_account: Address,
    chain_id: u64,
) -> Result<WalletSigner> {
    let (signer, tempo_access_key) = wallet.maybe_signer_for_chain(chain_id).await?;
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
    parse_policy_scope(s).map(CallScope::from)
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

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::address;
    use foundry_common::tempo::SessionStatus;
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
    fn session_scope_target_shortcut() {
        let target = address!("0x00000000000000000000000000000000000000aa");
        let err = session_scope(vec![], Some(target), vec![]).unwrap_err();
        assert!(err.to_string().contains("--target requires at least one --selector"), "{err}");

        // an explicit `--scope TARGET` keeps its target-wide wildcard
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
    fn local_revoke_is_idempotent_when_missing() {
        with_tempo_home(|| {
            assert!(!retire_session_entry(B256::from([0x42; 32])).unwrap());
        });
    }

    #[test]
    fn create_and_local_revoke_session_entry_round_trips() {
        with_tempo_home(|| {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.block_on(async {
                let root = address!("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
                let wallet = WalletOpts {
                    raw: foundry_wallets::RawWalletOpts {
                        private_key: Some(ROOT_PRIVATE_KEY.to_string()),
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
                let stored = read_session_entry(session_id).unwrap().unwrap();
                assert_eq!(stored.session_id, session_id);
                assert!(stored.has_live_key_at(expiry - 1));

                assert!(retire_session_entry(session_id).unwrap());
                let session = read_session_entry(session_id).unwrap().unwrap();
                assert_eq!(session.status, SessionStatus::Revoked);
                assert!(session.key.is_none());
            });
        });
    }
}
