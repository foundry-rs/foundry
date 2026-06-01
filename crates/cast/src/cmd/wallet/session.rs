use alloy_primitives::{Address, B256, U256};
use alloy_provider::Provider;
use alloy_signer::Signer;
use alloy_sol_types::SolCall;
use clap::Parser;
use eyre::{Context, Result};
use foundry_cli::{opts::TransactionOpts, utils::LoadConfig};
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
    time::{SystemTime, UNIX_EPOCH},
};
use tempo_alloy::{TempoNetwork, provider::TempoProviderExt};
use tempo_contracts::precompiles::IAccountKeychain;
use tempo_primitives::transaction::{CallScope, PrimitiveSignature, SelectorRule};

use crate::{
    cmd::{
        keychain::{
            KeychainTxOutcome, resolve_keychain_root_signer, send_keychain_tx_with_root_signer,
        },
        tempo_policy_args::{parse_period, parse_policy_token, parse_scope as parse_policy_scope},
    },
    tx::SendTxOpts,
};

const PRINT_SPONSOR_HASH_REVOKE_ERROR: &str = "--tempo.print-sponsor-hash only prints a sponsor hash and does not revoke the session on-chain";

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
        update_session_status(session_id, SessionStatus::Revoked)?;
        print_revoke_status(session_id, Some(&entry), SessionRevokeStatus::NotProvisioned)?;
        return Ok(());
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
            KeychainTxOutcome::PrintedSponsorHash => eyre::bail!(PRINT_SPONSOR_HASH_REVOKE_ERROR),
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
    } else if !matches!(
        read_session_entry(session_id).ok().flatten().map(|entry| entry.status),
        Some(SessionStatus::Revoked)
    ) {
        let _ =
            update_session_status_if(session_id, SessionStatus::Revoking, SessionStatus::Failed);
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
    let (token_str, amount_str) = if let Some(pair) = s.split_once(':') {
        pair
    } else if let Some(pair) = s.split_once('=') {
        pair
    } else {
        return Err(format!("invalid limit format: {s} (expected TOKEN:AMOUNT or TOKEN=AMOUNT)"));
    };

    let token = parse_policy_token(token_str.trim())?;
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
    use std::sync::Mutex;
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
    fn parse_spend_limit_accepts_path_usd_alias() {
        let limit = parse_spend_limit("PathUSD=0").unwrap();
        assert_eq!(limit.token, PATH_USD_ADDRESS);
        assert_eq!(limit.amount, U256::ZERO);
    }

    #[test]
    fn revoke_preflight_error_preserves_local_key_material() {
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
    fn revoke_submit_error_marks_revoking_session_failed() {
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
                assert_eq!(session.status, SessionStatus::Failed);
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
