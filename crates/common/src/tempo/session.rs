//! Tempo temporary access-key lifecycle backed by the canonical Accounts store.

use super::KeyType;
use alloy_primitives::{Address, B256, Selector, U256};
use eyre::ensure;
use foundry_wallets::TempoAccountsWallet;
use serde::{Deserialize, Serialize};
use std::{fmt, num::NonZeroU64, time::SystemTime};
use tempo_alloy::accounts::TempoAccountsStore;
use tempo_primitives::transaction::{
    CallScope, KeyAuthorization, SelectorRule, SignatureType, SignedKeyAuthorization, TokenLimit,
};

/// Status derived from a managed access key's Accounts record.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    #[default]
    Pending,
    Active,
    Revoked,
    Expired,
}

/// Spending limit attached to a managed access key.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct SessionTokenLimit {
    pub currency: Address,
    pub limit: String,
}

/// Transient key material used while adding an access key to the Accounts store.
#[derive(Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct SessionKeyMaterial {
    #[serde(default)]
    pub key_type: KeyType,
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_authorization: Option<String>,
}

impl fmt::Debug for SessionKeyMaterial {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionKeyMaterial")
            .field("key_type", &self.key_type)
            .field("key", &super::redacted_debug(&self.key))
            .field(
                "key_authorization",
                &self.key_authorization.as_deref().map(super::redacted_debug),
            )
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct SessionSelectorRule {
    pub selector: Selector,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recipients: Vec<Address>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct SessionCallScope {
    pub target: Address,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub selector_rules: Vec<SessionSelectorRule>,
}

/// Foundry's command-facing view of one Accounts access key.
///
/// This is not a persistence schema. The source of truth is always
/// `$TEMPO_HOME/wallet/store.json`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct SessionEntry {
    pub session_id: B256,
    pub root_account: Address,
    pub chain_id: u64,
    pub key_address: Address,
    pub expiry: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<Vec<SessionCallScope>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits: Option<Vec<SessionTokenLimit>>,
    #[serde(default)]
    pub status: SessionStatus,
    /// Present only before the entry is written to the Accounts store.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<SessionKeyMaterial>,
}

impl SessionEntry {
    pub const fn is_expired_at(&self, now: u64) -> bool {
        now >= self.expiry
    }

    pub fn has_live_key_at(&self, now: u64) -> bool {
        self.status == SessionStatus::Active && !self.is_expired_at(now)
    }
}

/// A live managed key pinned into an Accounts wallet.
#[derive(Debug)]
pub struct ResolvedSessionSigner {
    pub session: SessionEntry,
    pub access_key: TempoAccountsWallet,
}

pub fn read_session_entry(session_id: B256) -> eyre::Result<Option<SessionEntry>> {
    let now =
        SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_secs();
    Ok(read_session_entries(now)?
        .and_then(|entries| entries.into_iter().find(|entry| entry.session_id == session_id)))
}

/// Resolve a live managed key from the Accounts store and pin it for signing.
pub fn resolve_live_session_signer(
    session_id: B256,
    now: u64,
) -> eyre::Result<Option<ResolvedSessionSigner>> {
    mark_expired_session_entries(now)?;
    let Some(session) = read_session_entries(now)?
        .and_then(|entries| entries.into_iter().find(|entry| entry.session_id == session_id))
    else {
        return Ok(None);
    };
    if !session.has_live_key_at(now) {
        return Ok(None);
    }

    let wallet = TempoAccountsWallet::from_default_store()?.with_chain_id(session.chain_id);
    let selected =
        wallet.access_key(session.root_account, session.chain_id, session.key_address)?;
    let access_key = TempoAccountsWallet::from_access_key(selected);
    Ok(Some(ResolvedSessionSigner { session, access_key }))
}

/// Validate that a signed authorization describes the command-facing managed key.
pub(crate) fn validate_signed_session_authorization(
    session: &SessionEntry,
    expected_key_type: SignatureType,
    authorization: &SignedKeyAuthorization,
) -> eyre::Result<()> {
    let auth = &authorization.authorization;
    ensure!(
        auth.key_id == session.key_address,
        "session {} key_authorization key_id is {}, expected {}",
        session.session_id,
        auth.key_id,
        session.key_address
    );
    ensure!(
        auth.chain_id == session.chain_id,
        "session {} key_authorization chain_id is {}, expected {}",
        session.session_id,
        auth.chain_id,
        session.chain_id
    );
    ensure!(
        auth.key_type == expected_key_type,
        "session {} key_authorization key_type is {:?}, expected {:?}",
        session.session_id,
        auth.key_type,
        expected_key_type
    );
    ensure!(!auth.is_admin(), "session access key cannot be an admin key");
    if let Some(account) = auth.account {
        ensure!(
            account == session.root_account,
            "session authorization is bound to account {account}, expected {}",
            session.root_account
        );
    }
    ensure!(
        auth.witness == Some(session.session_id),
        "session authorization witness is {:?}, expected {}",
        auth.witness,
        session.session_id
    );
    ensure!(
        authorization.recover_signer()? == session.root_account,
        "session authorization was not signed by {}",
        session.root_account
    );
    validate_session_authorization_policy(session, auth)
}

fn validate_session_authorization_policy(
    session: &SessionEntry,
    authorization: &KeyAuthorization,
) -> eyre::Result<()> {
    let expiry = NonZeroU64::new(session.expiry)
        .ok_or_else(|| eyre::eyre!("session expiry cannot be zero"))?;
    ensure!(authorization.expiry == Some(expiry), "session authorization expiry does not match");

    let expected_limits = canonical_session_limits(session)?;
    let actual_limits = authorization.limits.as_deref().map(canonical_authorization_limits);
    ensure!(actual_limits == expected_limits, "session authorization limits do not match");

    let expected_scope = canonical_session_scope(session);
    let actual_scope = authorization.allowed_calls.as_deref().map(canonical_authorization_scope);
    ensure!(actual_scope == expected_scope, "session authorization call scope does not match");
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct CanonicalTokenLimit {
    token: Address,
    limit: U256,
    period: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct CanonicalCallScope {
    target: Address,
    selector_rules: Vec<CanonicalSelectorRule>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct CanonicalSelectorRule {
    selector: [u8; 4],
    recipients: Vec<Address>,
}

fn canonical_session_limits(
    session: &SessionEntry,
) -> eyre::Result<Option<Vec<CanonicalTokenLimit>>> {
    let Some(limits) = session.limits.as_deref() else {
        return Ok(None);
    };
    let mut limits = limits
        .iter()
        .map(|limit| {
            Ok(CanonicalTokenLimit {
                token: limit.currency,
                limit: parse_limit(&limit.limit)?,
                period: 0,
            })
        })
        .collect::<eyre::Result<Vec<_>>>()?;
    limits.sort();
    Ok(Some(limits))
}

fn canonical_authorization_limits(limits: &[TokenLimit]) -> Vec<CanonicalTokenLimit> {
    let mut limits = limits
        .iter()
        .map(|limit| CanonicalTokenLimit {
            token: limit.token,
            limit: limit.limit,
            period: limit.period,
        })
        .collect::<Vec<_>>();
    limits.sort();
    limits
}

fn canonical_session_scope(session: &SessionEntry) -> Option<Vec<CanonicalCallScope>> {
    let mut scopes = session
        .scope
        .as_deref()?
        .iter()
        .map(|scope| CanonicalCallScope {
            target: scope.target,
            selector_rules: canonical_session_selector_rules(&scope.selector_rules),
        })
        .collect::<Vec<_>>();
    scopes.sort();
    Some(scopes)
}

fn canonical_authorization_scope(scopes: &[CallScope]) -> Vec<CanonicalCallScope> {
    let mut scopes = scopes
        .iter()
        .map(|scope| CanonicalCallScope {
            target: scope.target,
            selector_rules: canonical_authorization_selector_rules(&scope.selector_rules),
        })
        .collect::<Vec<_>>();
    scopes.sort();
    scopes
}

fn canonical_session_selector_rules(rules: &[SessionSelectorRule]) -> Vec<CanonicalSelectorRule> {
    let mut rules = rules
        .iter()
        .map(|rule| {
            let mut recipients = rule.recipients.clone();
            recipients.sort();
            CanonicalSelectorRule { selector: rule.selector.into(), recipients }
        })
        .collect::<Vec<_>>();
    rules.sort();
    rules
}

fn canonical_authorization_selector_rules(rules: &[SelectorRule]) -> Vec<CanonicalSelectorRule> {
    let mut rules = rules
        .iter()
        .map(|rule| {
            let mut recipients = rule.recipients.clone();
            recipients.sort();
            CanonicalSelectorRule { selector: rule.selector, recipients }
        })
        .collect::<Vec<_>>();
    rules.sort();
    rules
}

fn parse_limit(raw: &str) -> eyre::Result<U256> {
    let raw = raw.trim();
    if let Some(hex) = raw.strip_prefix("0x") { U256::from_str_radix(hex, 16) } else { raw.parse() }
        .map_err(|err| eyre::eyre!("invalid session spending limit `{raw}`: {err}"))
}

/// Add a generated access key to the canonical Accounts store.
pub fn upsert_session_entry(entry: SessionEntry) -> eyre::Result<()> {
    let key = entry
        .key
        .as_ref()
        .ok_or_else(|| eyre::eyre!("managed access key has no local signing material"))?;
    ensure!(key.key_type == KeyType::Secp256k1, "only secp256k1 managed access keys are supported");
    let signer = foundry_wallets::utils::create_local_signer(&key.key)?;
    ensure!(
        signer.address() == entry.key_address,
        "managed access key resolves to {}, expected {}",
        signer.address(),
        entry.key_address
    );
    let encoded = key
        .key_authorization
        .as_deref()
        .ok_or_else(|| eyre::eyre!("managed access key has no signed authorization"))?;
    let authorization = super::decode_key_authorization::<SignedKeyAuthorization>(encoded)?;
    validate_signed_session_authorization(&entry, SignatureType::Secp256k1, &authorization)?;
    TempoAccountsStore::default_path()?.upsert_secp256k1_access_key(
        entry.root_account,
        &signer,
        &authorization,
    )?;
    Ok(())
}

/// Retire local signing material in `store.json` for the selected managed key.
///
/// The non-secret account, chain, policy, and authorization witness remain available for
/// on-chain revoke retries.
pub fn retire_session_entry(session_id: B256) -> eyre::Result<bool> {
    let Some(entry) = read_session_entry(session_id)? else {
        return Ok(false);
    };
    TempoAccountsStore::default_path()?
        .retire_access_key(entry.root_account, entry.chain_id, entry.key_address)
        .map_err(Into::into)
}

/// Retire expired managed access keys and return the number changed.
pub fn mark_expired_session_entries(now: u64) -> eyre::Result<usize> {
    let Some(entries) = read_session_entries(now)? else {
        return Ok(0);
    };
    let store = TempoAccountsStore::default_path()?;
    let mut retired = 0;
    for entry in entries {
        if entry.status == SessionStatus::Expired
            && store.retire_access_key(entry.root_account, entry.chain_id, entry.key_address)?
        {
            retired += 1;
        }
    }
    Ok(retired)
}

fn read_session_entries(now: u64) -> eyre::Result<Option<Vec<SessionEntry>>> {
    let Some(store) = TempoAccountsStore::try_open_default()? else {
        return Ok(None);
    };
    let sessions = store
        .access_keys()?
        .into_iter()
        .filter_map(|key| {
            let authorization = key.key_authorization()?;
            let session_id = key.authorization_witness()?;
            let expiry = key.expiry()?;
            let status = if expiry <= now {
                SessionStatus::Expired
            } else if key.is_locally_signable() {
                SessionStatus::Active
            } else {
                SessionStatus::Revoked
            };
            Some(SessionEntry {
                session_id,
                root_account: key.account(),
                chain_id: key.chain_id(),
                key_address: key.address(),
                expiry,
                scope: authorization.allowed_calls.as_ref().map(|scopes| {
                    scopes
                        .iter()
                        .map(|scope| SessionCallScope {
                            target: scope.target,
                            selector_rules: scope
                                .selector_rules
                                .iter()
                                .map(|rule| SessionSelectorRule {
                                    selector: rule.selector.into(),
                                    recipients: rule.recipients.clone(),
                                })
                                .collect(),
                        })
                        .collect()
                }),
                limits: authorization.limits.as_ref().map(|limits| {
                    limits
                        .iter()
                        .map(|limit| SessionTokenLimit {
                            currency: limit.token,
                            limit: limit.limit.to_string(),
                        })
                        .collect()
                }),
                status,
                key: None,
            })
        })
        .collect();
    Ok(Some(sessions))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_redacts_transient_key_material() {
        let key = SessionKeyMaterial {
            key_type: KeyType::Secp256k1,
            key: "0xPRIVATE_KEY_MUST_NOT_LEAK".into(),
            key_authorization: Some("0xAUTH_MUST_NOT_LEAK".into()),
        };
        let rendered = format!("{key:?}");
        assert!(!rendered.contains("PRIVATE_KEY_MUST_NOT_LEAK"));
        assert!(!rendered.contains("AUTH_MUST_NOT_LEAK"));
        assert!(rendered.contains("<redacted>"));
    }
}
