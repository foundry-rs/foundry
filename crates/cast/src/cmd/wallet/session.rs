use alloy_primitives::{Address, B256, U256};
use alloy_signer::Signer;
use clap::Parser;
use eyre::{Context, Result};
use foundry_common::{
    sh_println, shell,
    tempo::{
        GeneratedSessionKey, SessionAuthorizationRequest, SessionSpendLimit, remove_session_entry,
        upsert_session_entry,
    },
};
use foundry_wallets::{WalletOpts, WalletSigner};
use serde_json::json;
use std::{
    num::NonZeroU64,
    time::{SystemTime, UNIX_EPOCH},
};
use tempo_primitives::transaction::{CallScope, PrimitiveSignature, SelectorRule};

use crate::cmd::tempo_policy_args::{
    parse_period, parse_policy_token, parse_scope as parse_policy_scope,
};

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

    /// Revoke a local Tempo session entry.
    Revoke {
        /// Session identifier to revoke.
        #[arg(value_name = "SESSION_ID")]
        session_id: B256,
    },
}

impl SessionSubcommands {
    pub async fn run(self) -> Result<()> {
        match self {
            Self::Create { root_account, chain_id, expires, scope, spend_limits, wallet } => {
                run_create(root_account, chain_id, expires, scope, spend_limits, *wallet).await
            }
            Self::Revoke { session_id } => run_revoke(session_id),
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

/// Removes a session entry from the local registry.
fn run_revoke(session_id: B256) -> Result<()> {
    let removed = remove_session_entry(session_id)?;

    if shell::is_json() {
        sh_println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "session_id": session_id.to_string(),
                "status": if removed { "revoked" } else { "not_found" },
            }))?
        )?;
    } else if removed {
        sh_status!("Revoked Tempo session {}", session_id)?;
    } else {
        sh_status!("Tempo session {} was not found.", session_id)?;
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
    use foundry_common::tempo::SessionStatus;
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
            assert!(!remove_session_entry(session_id).unwrap());
        });
    }

    #[test]
    fn parse_spend_limit_accepts_path_usd_alias() {
        let limit = parse_spend_limit("PathUSD=0").unwrap();
        assert_eq!(limit.token, PATH_USD_ADDRESS);
        assert_eq!(limit.amount, U256::ZERO);
    }

    #[test]
    fn create_and_revoke_session_entry_round_trips() {
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

                assert!(remove_session_entry(session_id).unwrap());
                assert!(foundry_common::tempo::read_session_record().unwrap().is_empty());
            });
        });
    }
}
