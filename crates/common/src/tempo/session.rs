//! Tempo session registry and local lifecycle metadata.

use super::{KeyType, registry::*, tempo_home};
use alloy_primitives::{Address, B256, Selector, U256};
use alloy_signer::Signer;
use eyre::ensure;
use foundry_wallets::{TempoAccessKeyConfig, WalletSigner};
use serde::{Deserialize, Serialize};
use std::{fmt, num::NonZeroU64, path::PathBuf};
use tempo_primitives::transaction::{
    CallScope, KeyAuthorization, SelectorRule, SignatureType, SignedKeyAuthorization, TokenLimit,
};

/// Relative path from Tempo home to the session registry file.
pub const WALLET_SESSIONS_PATH: &str = "wallet/sessions.toml";

const SESSIONS_HEADER: &str =
    "# Tempo session registry — managed by Foundry / Tempo CLI.\n# Do not edit manually.";

/// Status of a local session entry.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    #[default]
    Pending,
    Active,
    /// Local use has stopped and key material has been erased, but on-chain revoke is still
    /// pending or retryable.
    Revoking,
    Revoked,
    Expired,
    Failed,
}

impl SessionStatus {
    /// Returns `true` if the session is no longer expected to be usable.
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Revoked | Self::Expired | Self::Failed)
    }

    /// Returns `true` if entering this status must erase local key material.
    const fn clears_key_material(self) -> bool {
        matches!(self, Self::Revoking) || self.is_terminal()
    }

    /// Returns `true` if the session is not terminal. This does not imply usable key material:
    /// [`Self::Revoking`] is in-flight cleanup state and has no local signing key.
    pub const fn is_live(self) -> bool {
        !self.is_terminal()
    }
}

/// Spending limit stored for a session entry.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct SessionTokenLimit {
    pub currency: Address,
    pub limit: String,
}

/// Private key material for a temporary session access key.
///
/// Session keys live with their lifecycle record in `wallet/sessions.toml`.
/// Persistent Tempo wallet login keys remain in `wallet/keys.toml`, so creating
/// or cleaning up a session cannot replace a user's long-lived access key.
#[derive(Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct SessionKeyMaterial {
    #[serde(default)]
    pub key_type: KeyType,
    /// Hex-encoded private key for the temporary session access key.
    pub key: String,
    /// RLP-encoded signed key authorization, if the key still needs inline
    /// provisioning on first use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_authorization: Option<String>,
}

// Manual `Debug` redacts the secret key material; propagates to containers.
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

impl SessionKeyMaterial {
    /// Returns `true` when the entry carries a non-empty private key.
    pub fn has_inline_key(&self) -> bool {
        !self.key.trim().is_empty()
    }
}

/// A single selector rule in a session scope.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct SessionSelectorRule {
    pub selector: Selector,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recipients: Vec<Address>,
}

/// A single target scope in a session entry.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct SessionCallScope {
    pub target: Address,
    /// Empty selector list means wildcard access for the target.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub selector_rules: Vec<SessionSelectorRule>,
}

/// Persisted metadata for one temporary session.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct SessionEntry {
    pub session_id: B256,
    pub root_account: Address,
    pub chain_id: u64,
    pub key_address: Address,
    /// Unix timestamp in seconds when the session expires.
    ///
    /// Tempo sessions are always bounded-lifetime. `0` is not a "never
    /// expires" sentinel; it is already expired.
    pub expiry: u64,
    /// Call scope policy for the session key. `None` means unrestricted;
    /// `Some([])` means no calls are allowed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<Vec<SessionCallScope>>,
    /// Spending limit policy for the session key. `None` means unrestricted;
    /// `Some([])` means no token spending is allowed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits: Option<Vec<SessionTokenLimit>>,
    #[serde(default)]
    pub status: SessionStatus,
    /// Session-scoped key material. This is intentionally separate from
    /// `wallet/keys.toml`, which stores persistent access keys.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<SessionKeyMaterial>,
}

impl SessionEntry {
    /// Returns `true` if the session has passed its expiry timestamp.
    pub const fn is_expired_at(&self, now: u64) -> bool {
        now >= self.expiry
    }

    /// Returns `true` if this session has usable local key material.
    pub fn has_inline_key(&self) -> bool {
        self.key.as_ref().is_some_and(SessionKeyMaterial::has_inline_key)
    }

    /// Returns `true` if this session is active, unexpired, and has key material.
    pub fn has_live_key_at(&self, now: u64) -> bool {
        self.status == SessionStatus::Active && !self.is_expired_at(now) && self.has_inline_key()
    }
}

/// Top-level registry persisted in `wallet/sessions.toml`.
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct SessionRecord {
    #[serde(default)]
    pub sessions: Vec<SessionEntry>,
}

impl SessionRecord {
    /// Returns `true` if the registry has no session entries.
    pub const fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    /// Insert or replace a session by `session_id`.
    pub fn upsert(&mut self, entry: SessionEntry) {
        self.sessions.retain(|session| session.session_id != entry.session_id);
        self.sessions.push(entry);
    }

    /// Remove a session by `session_id`. Returns `true` if an entry was removed.
    pub fn remove(&mut self, session_id: B256) -> bool {
        let before = self.sessions.len();
        self.sessions.retain(|session| session.session_id != session_id);
        self.sessions.len() != before
    }

    /// Returns a session by id.
    pub fn get(&self, session_id: B256) -> Option<&SessionEntry> {
        self.sessions.iter().find(|session| session.session_id == session_id)
    }

    /// Returns an active session with usable local key material by id.
    pub fn live_key(&self, session_id: B256, now: u64) -> Option<&SessionEntry> {
        self.get(session_id).filter(|session| session.has_live_key_at(now))
    }

    /// Update a session status by id. Cleanup and terminal statuses clear local key material.
    ///
    /// Returns `true` when the record changed. Missing sessions and idempotent
    /// updates return `false`.
    pub fn set_status(&mut self, session_id: B256, status: SessionStatus) -> bool {
        let Some(session) =
            self.sessions.iter_mut().find(|session| session.session_id == session_id)
        else {
            return false;
        };

        set_session_status(session, status)
    }

    /// Mark expired live entries as expired. Returns the number updated.
    pub fn mark_expired(&mut self, now: u64) -> usize {
        let mut updated = 0;
        for session in &mut self.sessions {
            let should_expire = session.status.is_live() && session.is_expired_at(now);
            let should_clear_key =
                session.key.is_some() && (should_expire || session.status.clears_key_material());

            if should_expire {
                session.status = SessionStatus::Expired;
            }
            if should_clear_key {
                session.key = None;
            }
            if should_expire || should_clear_key {
                updated += 1;
            }
        }
        updated
    }
}

fn set_session_status(session: &mut SessionEntry, status: SessionStatus) -> bool {
    let changed = session.status != status || status.clears_key_material() && session.key.is_some();
    if !changed {
        return false;
    }

    session.status = status;
    if status.clears_key_material() {
        session.key = None;
    }
    true
}

/// A live session key resolved into the signer and Tempo access-key metadata.
#[derive(Debug)]
pub struct ResolvedSessionSigner {
    pub session: SessionEntry,
    pub signer: WalletSigner,
    pub access_key: TempoAccessKeyConfig,
}

/// Returns the path to the Tempo session registry file.
pub fn session_registry_path() -> Option<PathBuf> {
    tempo_home().map(|home| home.join(WALLET_SESSIONS_PATH))
}

/// Read and parse the Tempo session registry.
///
/// Returns `None` if the file doesn't exist or can't be read/parsed.
/// Errors are logged as warnings.
pub fn read_session_record() -> Option<SessionRecord> {
    let path = session_registry_path()?;
    match read_toml_file(&path, "tempo sessions") {
        Ok(value) => value,
        Err(e) => {
            tracing::warn!(?path, %e, "failed to load tempo sessions file");
            None
        }
    }
}

/// Read a live session-scoped key entry by session id.
pub fn read_live_session_key(session_id: B256, now: u64) -> Option<SessionEntry> {
    read_session_record()?.live_key(session_id, now).cloned()
}

/// Read a session entry by id, returning parse/read errors to the caller.
pub fn read_session_entry(session_id: B256) -> eyre::Result<Option<SessionEntry>> {
    let path =
        session_registry_path().ok_or_else(|| eyre::eyre!("could not resolve tempo home"))?;
    Ok(read_toml_file::<SessionRecord>(&path, "tempo sessions")?
        .and_then(|record| record.get(session_id).cloned()))
}

/// Resolve a live session key into a signer and access-key configuration.
pub fn resolve_live_session_signer(
    session_id: B256,
    now: u64,
) -> eyre::Result<Option<ResolvedSessionSigner>> {
    mark_expired_session_entries(now)?;

    let path =
        session_registry_path().ok_or_else(|| eyre::eyre!("could not resolve tempo home"))?;
    let Some(record) = read_toml_file::<SessionRecord>(&path, "tempo sessions")? else {
        return Ok(None);
    };
    let Some(session) = record.live_key(session_id, now).cloned() else {
        return Ok(None);
    };
    let key =
        session.key.as_ref().ok_or_else(|| eyre::eyre!("live session has no key material"))?;

    let signer = foundry_wallets::utils::create_private_key_signer(&key.key)?;
    let signer_address = signer.address();
    if signer_address != session.key_address {
        eyre::bail!(
            "session {} key material resolves to {}, expected {}",
            session.session_id,
            signer_address,
            session.key_address
        );
    }

    let key_authorization = key
        .key_authorization
        .as_deref()
        .map(|raw| {
            super::decode_key_authorization::<SignedKeyAuthorization>(raw)
                .map_err(|err| eyre::eyre!("failed to decode session key_authorization: {err}"))
        })
        .transpose()?;
    if let Some(auth) = &key_authorization {
        validate_signed_session_authorization(
            &session,
            key_type_to_signature_type(key.key_type),
            auth,
        )?;
    }
    let access_key = TempoAccessKeyConfig {
        wallet_address: session.root_account,
        key_address: session.key_address,
        key_authorization,
    };

    Ok(Some(ResolvedSessionSigner { session, signer, access_key }))
}

/// Ensures a signed authorization matches stored session identity, key type, signer, and policy.
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
    // A session uses a limited access key; T6 admin keys must never be used as a session key.
    ensure!(
        !auth.is_admin(),
        "session {} key_authorization is an admin key, expected a limited access key",
        session.session_id
    );
    // A T6 account-bound authorization must target this session's root account (no cross-account
    // replay).
    if let Some(account) = auth.account {
        ensure!(
            account == session.root_account,
            "session {} key_authorization is bound to account {}, expected {}",
            session.session_id,
            account,
            session.root_account
        );
    }
    // `session_id` is local metadata; the signed binding lives in the authorization witness.
    ensure!(
        auth.witness == Some(session.session_id),
        "session {} key_authorization witness is {:?}, expected {}",
        session.session_id,
        auth.witness,
        session.session_id
    );
    let recovered = authorization
        .recover_signer()
        .map_err(|err| eyre::eyre!("failed to recover session key_authorization signer: {err}"))?;
    ensure!(
        recovered == session.root_account,
        "session {} key_authorization signer is {}, expected {}",
        session.session_id,
        recovered,
        session.root_account
    );
    validate_session_authorization_policy(session, auth)
}

/// Ensures authorization expiry, limits, and call scope match the stored session policy.
fn validate_session_authorization_policy(
    session: &SessionEntry,
    auth: &KeyAuthorization,
) -> eyre::Result<()> {
    let expected_expiry = NonZeroU64::new(session.expiry)
        .ok_or_else(|| eyre::eyre!("session {} has invalid zero expiry", session.session_id))?;
    ensure!(
        auth.expiry == Some(expected_expiry),
        "session {} key_authorization expiry is {:?}, expected {}",
        session.session_id,
        auth.expiry.map(NonZeroU64::get),
        session.expiry
    );

    let expected_limits = session_authorization_limits(session)?;
    let actual_limits = auth.limits.as_deref().map(authorization_limits);
    ensure!(
        actual_limits == expected_limits,
        "session {} key_authorization limits do not match session limits",
        session.session_id
    );

    let expected_scope = session_authorization_scope(session);
    let actual_scope = auth.allowed_calls.as_deref().map(authorization_scope);
    ensure!(
        actual_scope == expected_scope,
        "session {} key_authorization allowed_calls do not match session scope",
        session.session_id
    );

    Ok(())
}

/// Canonical spending limit used for order-independent policy comparisons.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct CanonicalTokenLimit {
    token: Address,
    limit: U256,
    period: u64,
}

/// Canonical target scope used for order-independent policy comparisons.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct CanonicalCallScope {
    target: Address,
    selector_rules: Vec<CanonicalSelectorRule>,
}

/// Canonical selector rule used for order-independent policy comparisons.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct CanonicalSelectorRule {
    selector: [u8; 4],
    recipients: Vec<Address>,
}

/// Converts stored session limits into canonical form for authorization comparison.
fn session_authorization_limits(
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
                limit: parse_session_limit(&limit.limit)?,
                period: 0,
            })
        })
        .collect::<eyre::Result<Vec<_>>>()?;
    limits.sort();
    Ok(Some(limits))
}

/// Converts signed authorization limits into canonical form for session comparison.
fn authorization_limits(limits: &[TokenLimit]) -> Vec<CanonicalTokenLimit> {
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

/// Parses a stored session spending limit from decimal or 0x-prefixed hex.
fn parse_session_limit(raw: &str) -> eyre::Result<U256> {
    let raw = raw.trim();
    if let Some(hex) = raw.strip_prefix("0x") { U256::from_str_radix(hex, 16) } else { raw.parse() }
        .map_err(|err| eyre::eyre!("invalid session spending limit `{raw}`: {err}"))
}

/// Converts stored session scope into canonical form for authorization comparison.
fn session_authorization_scope(session: &SessionEntry) -> Option<Vec<CanonicalCallScope>> {
    let mut scope = session
        .scope
        .as_deref()?
        .iter()
        .map(|scope| CanonicalCallScope {
            target: scope.target,
            selector_rules: session_authorization_selector_rules(&scope.selector_rules),
        })
        .collect::<Vec<_>>();
    scope.sort();
    Some(scope)
}

/// Converts signed authorization scope into canonical form for session comparison.
fn authorization_scope(scope: &[CallScope]) -> Vec<CanonicalCallScope> {
    let mut scope = scope
        .iter()
        .map(|scope| CanonicalCallScope {
            target: scope.target,
            selector_rules: authorization_selector_rules(&scope.selector_rules),
        })
        .collect::<Vec<_>>();
    scope.sort();
    scope
}

/// Converts stored selector rules into canonical form for authorization comparison.
fn session_authorization_selector_rules(
    rules: &[SessionSelectorRule],
) -> Vec<CanonicalSelectorRule> {
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

/// Converts signed authorization selector rules into canonical form for session comparison.
fn authorization_selector_rules(rules: &[SelectorRule]) -> Vec<CanonicalSelectorRule> {
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

/// Maps stored session key types to Tempo authorization signature types.
const fn key_type_to_signature_type(key_type: KeyType) -> SignatureType {
    match key_type {
        KeyType::Secp256k1 => SignatureType::Secp256k1,
        KeyType::P256 => SignatureType::P256,
        KeyType::WebAuthn => SignatureType::WebAuthn,
    }
}

fn mutate_session_record<R>(f: impl FnOnce(&mut SessionRecord) -> (R, bool)) -> eyre::Result<R> {
    let path =
        session_registry_path().ok_or_else(|| eyre::eyre!("could not resolve tempo home"))?;
    let mut record = read_toml_file::<SessionRecord>(&path, "tempo sessions")?.unwrap_or_default();
    let (result, changed) = f(&mut record);
    if changed {
        write_toml_file_atomic(&path, &record, SESSIONS_HEADER)?;
    }
    Ok(result)
}

/// Atomically upsert a [`SessionEntry`] into the session registry.
pub fn upsert_session_entry(entry: SessionEntry) -> eyre::Result<()> {
    mutate_session_record(|record| {
        record.upsert(entry);
        ((), true)
    })
}

/// Atomically update a session status in the registry.
///
/// Cleanup and terminal statuses (`revoking`, `revoked`, `expired`, `failed`) also clear the
/// session-scoped private key material. Returns `true` when an entry was found and changed.
pub fn update_session_status(session_id: B256, status: SessionStatus) -> eyre::Result<bool> {
    mutate_session_record(|record| {
        let changed = record.set_status(session_id, status);
        (changed, changed)
    })
}

/// Atomically update a session status only when the current status matches `current`.
///
/// Returns `true` when an entry was found with the expected current status. The
/// registry is only rewritten when the matched entry actually changes.
pub fn update_session_status_if(
    session_id: B256,
    current: SessionStatus,
    status: SessionStatus,
) -> eyre::Result<bool> {
    mutate_session_record(|record| {
        let Some(session) =
            record.sessions.iter_mut().find(|session| session.session_id == session_id)
        else {
            return (false, false);
        };
        if session.status != current {
            return (false, false);
        }

        let changed = set_session_status(session, status);
        (true, changed)
    })
}

/// Atomically remove a session from the registry.
pub fn remove_session_entry(session_id: B256) -> eyre::Result<bool> {
    mutate_session_record(|record| {
        let removed = record.remove(session_id);
        (removed, removed)
    })
}

/// Mark expired live sessions in the registry and persist the status updates.
pub fn mark_expired_session_entries(now: u64) -> eyre::Result<usize> {
    mutate_session_record(|record| {
        let updated = record.mark_expired(now);
        (updated, updated != 0)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tempo::with_tempo_home;
    use alloy_primitives::hex;
    use alloy_rlp::Encodable;
    use alloy_signer::SignerSync;
    use alloy_signer_local::PrivateKeySigner;
    use std::{fs, str::FromStr};
    use tempo_primitives::transaction::PrimitiveSignature;

    const ROOT_PRIVATE_KEY: &str =
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
    const SESSION_PRIVATE_KEY: &str =
        "0x59c6995e998f97a5a004497e5da3b5d2b2b66a87f064d39c44da0b6d6e4f8ff0";

    #[test]
    fn debug_redacts_session_key_material() {
        // Distinctive sentinels so a leak can't accidentally pass.
        let mut entry = sample_entry_with_key(B256::from([0x77; 32]), 200, SessionStatus::Active);
        let key = entry.key.as_mut().unwrap();
        key.key = "0xPRIVATE_KEY_MUST_NOT_LEAK".to_string();
        key.key_authorization = Some("0xKEY_AUTH_MUST_NOT_LEAK".to_string());

        let entry_dbg = format!("{entry:?}");
        let record_dbg = format!("{:?}", SessionRecord { sessions: vec![entry] });

        for rendered in [&entry_dbg, &record_dbg] {
            assert!(!rendered.contains("PRIVATE_KEY_MUST_NOT_LEAK"), "key leaked in: {rendered}");
            assert!(!rendered.contains("KEY_AUTH_MUST_NOT_LEAK"), "auth leaked in: {rendered}");
        }
        assert!(entry_dbg.contains("key: \"<redacted>\""), "got: {entry_dbg}");
        assert!(entry_dbg.contains("key_authorization: Some(\"<redacted>\")"), "got: {entry_dbg}");
        // Non-secret metadata is still visible for diagnostics.
        assert!(entry_dbg.contains("key_type"));
    }

    fn sample_entry(session_id: B256, expiry: u64, status: SessionStatus) -> SessionEntry {
        SessionEntry {
            session_id,
            root_account: Address::from_str("0x0000000000000000000000000000000000000001").unwrap(),
            chain_id: 4217,
            key_address: Address::from_str("0x0000000000000000000000000000000000000abc").unwrap(),
            expiry,
            scope: Some(vec![SessionCallScope {
                target: Address::from_str("0x00000000000000000000000000000000000000aa").unwrap(),
                selector_rules: vec![SessionSelectorRule {
                    selector: Selector::from_slice(&[0x12, 0x34, 0x56, 0x78]),
                    recipients: vec![],
                }],
            }]),
            limits: Some(vec![SessionTokenLimit {
                currency: Address::from_str("0x00000000000000000000000000000000000000ff").unwrap(),
                limit: "0".to_string(),
            }]),
            status,
            key: None,
        }
    }

    fn sample_entry_with_key(session_id: B256, expiry: u64, status: SessionStatus) -> SessionEntry {
        SessionEntry {
            key: Some(SessionKeyMaterial {
                key_type: KeyType::Secp256k1,
                key: "0xdeadbeef".to_string(),
                key_authorization: Some("0xfeed".to_string()),
            }),
            ..sample_entry(session_id, expiry, status)
        }
    }

    /// Builds a session entry with matching root and session key material.
    fn sample_entry_with_valid_key(
        session_id: B256,
        expiry: u64,
        status: SessionStatus,
    ) -> SessionEntry {
        let root_signer: PrivateKeySigner = ROOT_PRIVATE_KEY.parse().unwrap();
        let signer = foundry_wallets::utils::create_private_key_signer(SESSION_PRIVATE_KEY)
            .expect("valid test private key");
        SessionEntry {
            root_account: root_signer.address(),
            key_address: signer.address(),
            key: Some(SessionKeyMaterial {
                key_type: KeyType::Secp256k1,
                key: SESSION_PRIVATE_KEY.to_string(),
                key_authorization: None,
            }),
            ..sample_entry(session_id, expiry, status)
        }
    }

    /// Encodes a signed key authorization that matches the supplied session entry.
    fn signed_key_authorization_hex(entry: &SessionEntry) -> String {
        signed_key_authorization_hex_with(entry, std::convert::identity)
    }

    /// Encodes a signed key authorization after applying a test-specific mutation.
    fn signed_key_authorization_hex_with(
        entry: &SessionEntry,
        update: impl FnOnce(KeyAuthorization) -> KeyAuthorization,
    ) -> String {
        let root_signer: PrivateKeySigner = ROOT_PRIVATE_KEY.parse().unwrap();
        let auth = update(session_key_authorization(entry));
        let signature = root_signer.sign_hash_sync(&auth.signature_hash()).unwrap();
        let signed = auth.into_signed(PrimitiveSignature::Secp256k1(signature));
        let mut buf = Vec::new();
        signed.encode(&mut buf);
        hex::encode_prefixed(buf)
    }

    /// Builds a key authorization that mirrors the session entry policy.
    fn session_key_authorization(entry: &SessionEntry) -> KeyAuthorization {
        let mut authorization = KeyAuthorization::unrestricted(
            entry.chain_id,
            SignatureType::Secp256k1,
            entry.key_address,
        )
        .with_expiry(entry.expiry)
        .with_witness(entry.session_id);
        if let Some(limits) = &entry.limits {
            authorization = authorization.with_limits(
                limits
                    .iter()
                    .map(|limit| TokenLimit {
                        token: limit.currency,
                        limit: parse_session_limit(&limit.limit).unwrap(),
                        period: 0,
                    })
                    .collect(),
            );
        }
        if let Some(scope) = &entry.scope {
            authorization = authorization.with_allowed_calls(
                scope
                    .iter()
                    .map(|scope| CallScope {
                        target: scope.target,
                        selector_rules: scope
                            .selector_rules
                            .iter()
                            .map(|rule| SelectorRule {
                                selector: rule.selector.into(),
                                recipients: rule.recipients.clone(),
                            })
                            .collect(),
                    })
                    .collect(),
            );
        }
        authorization
    }

    #[test]
    fn session_registry_is_separate_from_keys_registry() {
        with_tempo_home(|| {
            let session_id = B256::from([0x11; 32]);
            upsert_session_entry(sample_entry(session_id, 100, SessionStatus::Pending)).unwrap();

            let session_path = session_registry_path().unwrap();
            let keys_path = crate::tempo::tempo_keys_path().unwrap();
            assert_eq!(session_path.file_name().and_then(|s| s.to_str()), Some("sessions.toml"));
            assert_eq!(keys_path.file_name().and_then(|s| s.to_str()), Some("keys.toml"));
            assert_ne!(session_path, keys_path);

            let record = read_session_record().unwrap();
            assert_eq!(record.sessions.len(), 1);
            assert_eq!(record.sessions[0].session_id, session_id);
        });
    }

    #[test]
    fn session_registry_upsert_replaces_matching_session_id() {
        with_tempo_home(|| {
            let session_id = B256::from([0x22; 32]);
            upsert_session_entry(sample_entry(session_id, 100, SessionStatus::Pending)).unwrap();
            upsert_session_entry(sample_entry(session_id, 200, SessionStatus::Active)).unwrap();

            let record = read_session_record().unwrap();
            assert_eq!(record.sessions.len(), 1);
            assert_eq!(record.sessions[0].expiry, 200);
            assert_eq!(record.sessions[0].status, SessionStatus::Active);
        });
    }

    #[test]
    fn session_registry_remove_deletes_entry() {
        with_tempo_home(|| {
            let session_id = B256::from([0x33; 32]);
            upsert_session_entry(sample_entry(session_id, 100, SessionStatus::Active)).unwrap();
            assert!(remove_session_entry(session_id).unwrap());
            assert!(read_session_record().unwrap().is_empty());
        });
    }

    #[test]
    fn session_record_marks_expired_live_entries() {
        let mut record = SessionRecord {
            sessions: vec![
                sample_entry(B256::from([0x44; 32]), 10, SessionStatus::Pending),
                sample_entry(B256::from([0x55; 32]), 10, SessionStatus::Revoked),
            ],
        };

        assert_eq!(record.mark_expired(11), 1);
        assert_eq!(record.sessions[0].status, SessionStatus::Expired);
        assert_eq!(record.sessions[1].status, SessionStatus::Revoked);
    }

    #[test]
    fn session_record_status_updates_clear_cleanup_and_terminal_keys() {
        let active_id = B256::from([0x67; 32]);
        let revoking_id = B256::from([0x68; 32]);
        let revoked_id = B256::from([0x69; 32]);
        let failed_id = B256::from([0x6a; 32]);
        let missing_id = B256::from([0x6b; 32]);
        let mut record = SessionRecord {
            sessions: vec![
                sample_entry_with_key(active_id, 200, SessionStatus::Pending),
                sample_entry_with_key(revoking_id, 200, SessionStatus::Active),
                sample_entry_with_key(revoked_id, 200, SessionStatus::Revoking),
                sample_entry_with_key(failed_id, 200, SessionStatus::Pending),
            ],
        };

        assert!(record.set_status(active_id, SessionStatus::Active));
        assert!(record.set_status(revoking_id, SessionStatus::Revoking));
        assert!(record.set_status(revoked_id, SessionStatus::Revoked));
        assert!(record.set_status(failed_id, SessionStatus::Failed));
        assert!(!record.set_status(missing_id, SessionStatus::Active));
        assert!(!record.set_status(active_id, SessionStatus::Active));

        assert_eq!(record.get(active_id).unwrap().status, SessionStatus::Active);
        assert!(record.get(active_id).unwrap().key.is_some());
        assert_eq!(record.get(revoking_id).unwrap().status, SessionStatus::Revoking);
        assert!(record.get(revoking_id).unwrap().key.is_none());
        assert_eq!(record.get(revoked_id).unwrap().status, SessionStatus::Revoked);
        assert!(record.get(revoked_id).unwrap().key.is_none());
        assert_eq!(record.get(failed_id).unwrap().status, SessionStatus::Failed);
        assert!(record.get(failed_id).unwrap().key.is_none());
    }

    #[test]
    fn session_entry_roundtrips_scope_limits_and_status() {
        let entry = sample_entry(B256::from([0x66; 32]), 1234, SessionStatus::Revoking);
        let toml = toml::to_string(&entry).unwrap();
        let decoded: SessionEntry = toml::from_str(&toml).unwrap();

        assert_eq!(decoded.session_id, entry.session_id);
        assert_eq!(decoded.scope.as_ref().unwrap().len(), 1);
        assert_eq!(decoded.limits.as_ref().unwrap().len(), 1);
        assert_eq!(decoded.status, SessionStatus::Revoking);
        assert!(decoded.key.is_none());
        assert!(!decoded.has_inline_key());
        assert!(decoded.is_expired_at(1234));
    }

    #[test]
    fn live_session_key_requires_key_material_live_status_and_unexpired_entry() {
        let live_id = B256::from([0x01; 32]);
        let expired_id = B256::from([0x02; 32]);
        let revoked_id = B256::from([0x03; 32]);
        let no_key_id = B256::from([0x04; 32]);
        let pending_id = B256::from([0x05; 32]);
        let revoking_id = B256::from([0x06; 32]);

        let record = SessionRecord {
            sessions: vec![
                sample_entry_with_key(live_id, 200, SessionStatus::Active),
                sample_entry_with_key(expired_id, 100, SessionStatus::Active),
                sample_entry_with_key(revoked_id, 200, SessionStatus::Revoked),
                sample_entry(no_key_id, 200, SessionStatus::Active),
                sample_entry_with_key(pending_id, 200, SessionStatus::Pending),
                sample_entry_with_key(revoking_id, 200, SessionStatus::Revoking),
            ],
        };

        assert_eq!(record.live_key(live_id, 100).unwrap().session_id, live_id);
        assert!(record.live_key(expired_id, 100).is_none());
        assert!(record.live_key(revoked_id, 100).is_none());
        assert!(record.live_key(no_key_id, 100).is_none());
        assert!(record.live_key(pending_id, 100).is_none());
        assert!(record.live_key(revoking_id, 100).is_none());
    }

    #[test]
    fn resolve_live_session_signer_returns_signer_and_access_key_config() {
        with_tempo_home(|| {
            let session_id = B256::from([0x06; 32]);
            let entry = sample_entry_with_valid_key(session_id, 200, SessionStatus::Active);
            upsert_session_entry(entry.clone()).unwrap();

            let resolved = resolve_live_session_signer(session_id, 100).unwrap().unwrap();

            assert_eq!(resolved.session, entry);
            assert_eq!(Signer::address(&resolved.signer), entry.key_address);
            assert_eq!(resolved.access_key.wallet_address, entry.root_account);
            assert_eq!(resolved.access_key.key_address, entry.key_address);
            assert!(resolved.access_key.key_authorization.is_none());
        });
    }

    #[test]
    fn resolve_live_session_signer_rejects_mismatched_private_key() {
        with_tempo_home(|| {
            let session_id = B256::from([0x07; 32]);
            let mut entry = sample_entry_with_valid_key(session_id, 200, SessionStatus::Active);
            entry.key_address =
                Address::from_str("0x0000000000000000000000000000000000000abc").unwrap();
            upsert_session_entry(entry).unwrap();

            let error = resolve_live_session_signer(session_id, 100).unwrap_err();

            assert!(error.to_string().contains("key material resolves to"));
        });
    }

    #[test]
    fn resolve_live_session_signer_expires_stale_entries_before_resolving() {
        with_tempo_home(|| {
            let session_id = B256::from([0x08; 32]);
            upsert_session_entry(sample_entry_with_valid_key(
                session_id,
                100,
                SessionStatus::Active,
            ))
            .unwrap();

            assert!(resolve_live_session_signer(session_id, 100).unwrap().is_none());

            let record = read_session_record().unwrap();
            let session = record.get(session_id).unwrap();
            assert_eq!(session.status, SessionStatus::Expired);
            assert!(session.key.is_none());
        });
    }

    #[test]
    fn resolve_live_session_signer_decodes_and_validates_key_authorization() {
        with_tempo_home(|| {
            let session_id = B256::from([0x09; 32]);
            let mut entry = sample_entry_with_valid_key(session_id, 200, SessionStatus::Active);
            let auth = signed_key_authorization_hex(&entry);
            entry.key.as_mut().unwrap().key_authorization = Some(auth);
            upsert_session_entry(entry.clone()).unwrap();

            let resolved = resolve_live_session_signer(session_id, 100).unwrap().unwrap();
            let key_authorization = resolved.access_key.key_authorization.unwrap();

            assert_eq!(key_authorization.authorization.key_id, entry.key_address);
            assert_eq!(key_authorization.authorization.chain_id, entry.chain_id);
            assert_eq!(key_authorization.authorization.key_type, SignatureType::Secp256k1);
            assert_eq!(key_authorization.authorization.expiry.unwrap().get(), entry.expiry);
            assert!(key_authorization.authorization.limits.is_some());
            assert!(key_authorization.authorization.allowed_calls.is_some());
            assert_eq!(key_authorization.recover_signer().unwrap(), entry.root_account);
        });
    }

    #[test]
    fn resolve_live_session_signer_accepts_unrestricted_authorization_when_policy_is_omitted() {
        with_tempo_home(|| {
            let session_id = B256::from([0x13; 32]);
            let mut entry = sample_entry_with_valid_key(session_id, 200, SessionStatus::Active);
            entry.limits = None;
            entry.scope = None;
            entry.key.as_mut().unwrap().key_authorization =
                Some(signed_key_authorization_hex(&entry));
            upsert_session_entry(entry.clone()).unwrap();

            let resolved = resolve_live_session_signer(session_id, 100).unwrap().unwrap();
            let key_authorization = resolved.access_key.key_authorization.unwrap();

            assert!(key_authorization.authorization.limits.is_none());
            assert!(key_authorization.authorization.allowed_calls.is_none());
        });
    }

    #[test]
    fn resolve_live_session_signer_rejects_unrestricted_authorization_when_policy_is_empty() {
        with_tempo_home(|| {
            let session_id = B256::from([0x14; 32]);
            let mut entry = sample_entry_with_valid_key(session_id, 200, SessionStatus::Active);
            let mut auth_entry = entry.clone();
            auth_entry.limits = None;
            auth_entry.scope = None;
            entry.limits = Some(vec![]);
            entry.scope = Some(vec![]);
            entry.key.as_mut().unwrap().key_authorization =
                Some(signed_key_authorization_hex(&auth_entry));
            upsert_session_entry(entry).unwrap();

            let error = resolve_live_session_signer(session_id, 100).unwrap_err();

            assert!(error.to_string().contains("limits"));
        });
    }

    #[test]
    fn resolve_live_session_signer_rejects_invalid_key_authorization() {
        with_tempo_home(|| {
            let session_id = B256::from([0x0a; 32]);
            let mut entry = sample_entry_with_valid_key(session_id, 200, SessionStatus::Active);
            entry.key.as_mut().unwrap().key_authorization = Some("0xdeadbeef".to_string());
            upsert_session_entry(entry).unwrap();

            let error = resolve_live_session_signer(session_id, 100).unwrap_err();

            assert!(error.to_string().contains("key_authorization"));
        });
    }

    #[test]
    fn resolve_live_session_signer_rejects_authorization_for_wrong_chain() {
        with_tempo_home(|| {
            let session_id = B256::from([0x0b; 32]);
            let mut entry = sample_entry_with_valid_key(session_id, 200, SessionStatus::Active);
            let mut auth_entry = entry.clone();
            auth_entry.chain_id += 1;
            entry.key.as_mut().unwrap().key_authorization =
                Some(signed_key_authorization_hex(&auth_entry));
            upsert_session_entry(entry).unwrap();

            let error = resolve_live_session_signer(session_id, 100).unwrap_err();

            assert!(error.to_string().contains("chain_id"));
        });
    }

    #[test]
    fn resolve_live_session_signer_rejects_authorization_without_session_expiry() {
        with_tempo_home(|| {
            let session_id = B256::from([0x0d; 32]);
            let mut entry = sample_entry_with_valid_key(session_id, 200, SessionStatus::Active);
            entry.key.as_mut().unwrap().key_authorization =
                Some(signed_key_authorization_hex_with(&entry, |mut auth| {
                    auth.expiry = None;
                    auth
                }));
            upsert_session_entry(entry).unwrap();

            let error = resolve_live_session_signer(session_id, 100).unwrap_err();

            assert!(error.to_string().contains("expiry"));
        });
    }

    #[test]
    fn resolve_live_session_signer_rejects_authorization_without_session_limits() {
        with_tempo_home(|| {
            let session_id = B256::from([0x0e; 32]);
            let mut entry = sample_entry_with_valid_key(session_id, 200, SessionStatus::Active);
            entry.key.as_mut().unwrap().key_authorization =
                Some(signed_key_authorization_hex_with(&entry, |mut auth| {
                    auth.limits = None;
                    auth
                }));
            upsert_session_entry(entry).unwrap();

            let error = resolve_live_session_signer(session_id, 100).unwrap_err();

            assert!(error.to_string().contains("limits"));
        });
    }

    #[test]
    fn resolve_live_session_signer_rejects_authorization_without_session_scope() {
        with_tempo_home(|| {
            let session_id = B256::from([0x0f; 32]);
            let mut entry = sample_entry_with_valid_key(session_id, 200, SessionStatus::Active);
            entry.key.as_mut().unwrap().key_authorization =
                Some(signed_key_authorization_hex_with(&entry, |mut auth| {
                    auth.allowed_calls = None;
                    auth
                }));
            upsert_session_entry(entry).unwrap();

            let error = resolve_live_session_signer(session_id, 100).unwrap_err();

            assert!(error.to_string().contains("allowed_calls"));
        });
    }

    #[test]
    fn resolve_live_session_signer_rejects_authorization_for_wrong_session_id() {
        with_tempo_home(|| {
            let session_id = B256::from([0x15; 32]);
            let mut entry = sample_entry_with_valid_key(session_id, 200, SessionStatus::Active);
            entry.key.as_mut().unwrap().key_authorization =
                Some(signed_key_authorization_hex_with(&entry, |auth| {
                    auth.with_witness(B256::from([0x16; 32]))
                }));
            upsert_session_entry(entry).unwrap();

            let error = resolve_live_session_signer(session_id, 100).unwrap_err();

            assert!(error.to_string().contains("witness"));
        });
    }

    #[test]
    fn resolve_rejects_admin_key_authorization() {
        with_tempo_home(|| {
            let session_id = B256::from([0x17; 32]);
            let mut entry = sample_entry_with_valid_key(session_id, 200, SessionStatus::Active);
            // A session must use a limited access key, never a T6 admin key.
            entry.key.as_mut().unwrap().key_authorization =
                Some(signed_key_authorization_hex_with(&entry, |mut auth| {
                    auth.is_admin = true;
                    auth
                }));
            upsert_session_entry(entry).unwrap();

            let error = resolve_live_session_signer(session_id, 100).unwrap_err();

            assert!(error.to_string().contains("admin key"), "got: {error}");
        });
    }

    #[test]
    fn resolve_rejects_account_bound_to_other_account() {
        with_tempo_home(|| {
            let session_id = B256::from([0x18; 32]);
            let mut entry = sample_entry_with_valid_key(session_id, 200, SessionStatus::Active);
            // An account-bound authorization minted for another account must not be replayable.
            entry.key.as_mut().unwrap().key_authorization =
                Some(signed_key_authorization_hex_with(&entry, |auth| {
                    auth.with_account(
                        Address::from_str("0x000000000000000000000000000000000000dead").unwrap(),
                    )
                }));
            upsert_session_entry(entry).unwrap();

            let error = resolve_live_session_signer(session_id, 100).unwrap_err();

            assert!(error.to_string().contains("bound to account"), "got: {error}");
        });
    }

    #[test]
    fn resolve_accepts_account_bound_to_root() {
        with_tempo_home(|| {
            let session_id = B256::from([0x19; 32]);
            let mut entry = sample_entry_with_valid_key(session_id, 200, SessionStatus::Active);
            let root_account = entry.root_account;
            // An account binding that targets the session root is valid (backward compatible).
            entry.key.as_mut().unwrap().key_authorization =
                Some(signed_key_authorization_hex_with(&entry, |auth| {
                    auth.with_account(root_account)
                }));
            upsert_session_entry(entry).unwrap();

            assert!(resolve_live_session_signer(session_id, 100).is_ok());
        });
    }

    #[test]
    fn resolve_live_session_signer_rejects_authorization_with_wider_session_limit() {
        with_tempo_home(|| {
            let session_id = B256::from([0x10; 32]);
            let mut entry = sample_entry_with_valid_key(session_id, 200, SessionStatus::Active);
            entry.key.as_mut().unwrap().key_authorization =
                Some(signed_key_authorization_hex_with(&entry, |mut auth| {
                    auth.limits.as_mut().unwrap()[0].limit = U256::from(1);
                    auth
                }));
            upsert_session_entry(entry).unwrap();

            let error = resolve_live_session_signer(session_id, 100).unwrap_err();

            assert!(error.to_string().contains("limits"));
        });
    }

    #[test]
    fn resolve_live_session_signer_rejects_authorization_with_wider_session_scope() {
        with_tempo_home(|| {
            let session_id = B256::from([0x12; 32]);
            let mut entry = sample_entry_with_valid_key(session_id, 200, SessionStatus::Active);
            entry.key.as_mut().unwrap().key_authorization =
                Some(signed_key_authorization_hex_with(&entry, |mut auth| {
                    auth.allowed_calls.as_mut().unwrap()[0].selector_rules.clear();
                    auth
                }));
            upsert_session_entry(entry).unwrap();

            let error = resolve_live_session_signer(session_id, 100).unwrap_err();

            assert!(error.to_string().contains("allowed_calls"));
        });
    }

    #[test]
    fn resolve_live_session_signer_fails_closed_when_session_file_is_corrupt() {
        with_tempo_home(|| {
            let path = session_registry_path().unwrap();
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, "sessions = [").unwrap();
            let original = fs::read_to_string(&path).unwrap();

            assert!(resolve_live_session_signer(B256::from([0x0c; 32]), 100).is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), original);
        });
    }

    #[test]
    fn session_key_storage_does_not_replace_persistent_keys_file() {
        with_tempo_home(|| {
            let keys_path = crate::tempo::tempo_keys_path().unwrap();
            fs::create_dir_all(keys_path.parent().unwrap()).unwrap();
            let original_keys = r#"[[keys]]
wallet_type = "local"
wallet_address = "0x0000000000000000000000000000000000000001"
chain_id = 4217
key_type = "secp256k1"
key_address = "0x0000000000000000000000000000000000000001"
key = "0x1111"
expiry = 999
"#;
            fs::write(&keys_path, original_keys).unwrap();

            let session_id = B256::from([0x99; 32]);
            upsert_session_entry(sample_entry_with_key(session_id, 200, SessionStatus::Active))
                .unwrap();

            assert_eq!(fs::read_to_string(&keys_path).unwrap(), original_keys);
            let session = read_live_session_key(session_id, 100).unwrap();
            assert_eq!(session.key.unwrap().key, "0xdeadbeef");
        });
    }

    #[test]
    fn removing_session_key_preserves_persistent_key() {
        with_tempo_home(|| {
            let keys_path = crate::tempo::tempo_keys_path().unwrap();
            fs::create_dir_all(keys_path.parent().unwrap()).unwrap();
            let original_keys = r#"[[keys]]
wallet_type = "local"
wallet_address = "0x0000000000000000000000000000000000000001"
chain_id = 4217
key_type = "secp256k1"
key_address = "0x0000000000000000000000000000000000000001"
key = "0x1111"
"#;
            fs::write(&keys_path, original_keys).unwrap();

            let session_id = B256::from([0xaa; 32]);
            upsert_session_entry(sample_entry_with_key(session_id, 200, SessionStatus::Active))
                .unwrap();
            assert!(remove_session_entry(session_id).unwrap());

            assert_eq!(fs::read_to_string(&keys_path).unwrap(), original_keys);
            assert!(read_session_record().unwrap().is_empty());
        });
    }

    #[test]
    fn mark_expired_session_entries_persists_status_without_touching_keys_file() {
        with_tempo_home(|| {
            let keys_path = crate::tempo::tempo_keys_path().unwrap();
            fs::create_dir_all(keys_path.parent().unwrap()).unwrap();
            let original_keys = "[[keys]]\nkey = \"0x1111\"\n";
            fs::write(&keys_path, original_keys).unwrap();

            let session_id = B256::from([0xbb; 32]);
            upsert_session_entry(sample_entry_with_key(session_id, 100, SessionStatus::Active))
                .unwrap();

            assert_eq!(mark_expired_session_entries(100).unwrap(), 1);
            let record = read_session_record().unwrap();
            let session = record.get(session_id).unwrap();
            assert_eq!(session.status, SessionStatus::Expired);
            assert!(session.key.is_none());
            assert!(read_live_session_key(session_id, 100).is_none());
            assert_eq!(fs::read_to_string(&keys_path).unwrap(), original_keys);
        });
    }

    #[test]
    fn mark_expired_session_entries_clears_unusable_session_keys() {
        with_tempo_home(|| {
            let expired_id = B256::from([0xbc; 32]);
            let revoked_id = B256::from([0xbd; 32]);
            let failed_id = B256::from([0xbe; 32]);
            let revoking_id = B256::from([0xbf; 32]);

            upsert_session_entry(sample_entry_with_key(expired_id, 100, SessionStatus::Expired))
                .unwrap();
            upsert_session_entry(sample_entry_with_key(revoked_id, 200, SessionStatus::Revoked))
                .unwrap();
            upsert_session_entry(sample_entry_with_key(failed_id, 200, SessionStatus::Failed))
                .unwrap();
            upsert_session_entry(sample_entry_with_key(revoking_id, 200, SessionStatus::Revoking))
                .unwrap();

            assert_eq!(mark_expired_session_entries(100).unwrap(), 4);
            let record = read_session_record().unwrap();
            for session_id in [expired_id, revoked_id, failed_id, revoking_id] {
                assert!(record.get(session_id).unwrap().key.is_none());
            }
            assert_eq!(record.get(expired_id).unwrap().status, SessionStatus::Expired);
            assert_eq!(record.get(revoked_id).unwrap().status, SessionStatus::Revoked);
            assert_eq!(record.get(failed_id).unwrap().status, SessionStatus::Failed);
            assert_eq!(record.get(revoking_id).unwrap().status, SessionStatus::Revoking);
        });
    }

    #[test]
    fn update_session_status_persists_lifecycle_state_and_key_cleanup() {
        with_tempo_home(|| {
            let session_id = B256::from([0xbf; 32]);
            upsert_session_entry(sample_entry_with_key(session_id, 200, SessionStatus::Pending))
                .unwrap();

            assert!(update_session_status(session_id, SessionStatus::Active).unwrap());
            let record = read_session_record().unwrap();
            let session = record.get(session_id).unwrap();
            assert_eq!(session.status, SessionStatus::Active);
            assert!(session.key.is_some());

            assert!(update_session_status(session_id, SessionStatus::Revoking).unwrap());
            let record = read_session_record().unwrap();
            let session = record.get(session_id).unwrap();
            assert_eq!(session.status, SessionStatus::Revoking);
            assert!(session.key.is_none());

            assert!(update_session_status(session_id, SessionStatus::Revoked).unwrap());
            let record = read_session_record().unwrap();
            let session = record.get(session_id).unwrap();
            assert_eq!(session.status, SessionStatus::Revoked);
            assert!(session.key.is_none());
            assert!(read_live_session_key(session_id, 100).is_none());

            assert!(!update_session_status(session_id, SessionStatus::Revoked).unwrap());
            assert!(!update_session_status(B256::from([0xc0; 32]), SessionStatus::Failed).unwrap());
        });
    }

    #[test]
    fn update_session_status_to_failed_clears_key_material() {
        with_tempo_home(|| {
            let session_id = B256::from([0xc1; 32]);
            upsert_session_entry(sample_entry_with_key(session_id, 200, SessionStatus::Active))
                .unwrap();

            assert!(update_session_status(session_id, SessionStatus::Failed).unwrap());

            let record = read_session_record().unwrap();
            let session = record.get(session_id).unwrap();
            assert_eq!(session.status, SessionStatus::Failed);
            assert!(session.key.is_none());
        });
    }

    #[test]
    fn update_session_status_if_only_updates_matching_current_status() {
        with_tempo_home(|| {
            let session_id = B256::from([0xc2; 32]);
            upsert_session_entry(sample_entry_with_key(session_id, 200, SessionStatus::Active))
                .unwrap();

            assert!(
                update_session_status_if(
                    session_id,
                    SessionStatus::Active,
                    SessionStatus::Revoking,
                )
                .unwrap()
            );
            let record = read_session_record().unwrap();
            let session = record.get(session_id).unwrap();
            assert_eq!(session.status, SessionStatus::Revoking);
            assert!(session.key.is_none());

            assert!(!update_session_status_if(
                session_id,
                SessionStatus::Active,
                SessionStatus::Failed,
            )
            .unwrap());
            assert_eq!(
                read_session_record().unwrap().get(session_id).unwrap().status,
                SessionStatus::Revoking
            );
        });
    }

    #[test]
    fn upsert_fails_closed_when_session_file_is_corrupt() {
        with_tempo_home(|| {
            let path = session_registry_path().unwrap();
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, "sessions = [").unwrap();
            let original = fs::read_to_string(&path).unwrap();

            let session_id = B256::from([0x77; 32]);
            let entry = sample_entry(session_id, 100, SessionStatus::Pending);

            assert!(read_session_record().is_none());
            assert!(upsert_session_entry(entry).is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), original);
        });
    }

    #[test]
    fn remove_fails_closed_when_session_file_is_corrupt() {
        with_tempo_home(|| {
            let path = session_registry_path().unwrap();
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, "sessions = [").unwrap();
            let original = fs::read_to_string(&path).unwrap();

            assert!(remove_session_entry(B256::from([0x88; 32])).is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), original);
        });
    }

    #[test]
    fn mark_expired_fails_closed_when_session_file_is_corrupt() {
        with_tempo_home(|| {
            let path = session_registry_path().unwrap();
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, "sessions = [").unwrap();
            let original = fs::read_to_string(&path).unwrap();

            assert!(mark_expired_session_entries(100).is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), original);
        });
    }

    #[test]
    fn update_session_status_fails_closed_when_session_file_is_corrupt() {
        with_tempo_home(|| {
            let path = session_registry_path().unwrap();
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, "sessions = [").unwrap();
            let original = fs::read_to_string(&path).unwrap();

            assert!(update_session_status(B256::from([0xc2; 32]), SessionStatus::Failed).is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), original);
        });
    }
}
