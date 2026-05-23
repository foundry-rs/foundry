//! Tempo session registry and local lifecycle metadata.

use super::{KeyType, registry::*, tempo_home};
use alloy_primitives::{Address, B256, Selector, U256};
use alloy_signer::Signer;
use foundry_wallets::{TempoAccessKeyConfig, WalletSigner};
use serde::{Deserialize, Serialize};
use std::{num::NonZeroU64, path::PathBuf};
use tempo_primitives::transaction::SignedKeyAuthorization;

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

    /// Returns `true` if the session is still in-flight or usable.
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
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
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

    /// Mark expired live entries as expired. Returns the number updated.
    pub fn mark_expired(&mut self, now: u64) -> usize {
        let mut updated = 0;
        for session in &mut self.sessions {
            let should_expire = session.status.is_live() && session.is_expired_at(now);
            let should_clear_key =
                session.key.is_some() && (should_expire || session.status.is_terminal());

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
        validate_session_key_authorization(&session, key, auth)?;
    }
    let access_key = TempoAccessKeyConfig {
        wallet_address: session.root_account,
        key_address: session.key_address,
        key_authorization,
    };

    Ok(Some(ResolvedSessionSigner { session, signer, access_key }))
}

/// Ensures a session key authorization matches the stored key, chain, and root signer.
fn validate_session_key_authorization(
    session: &SessionEntry,
    key: &SessionKeyMaterial,
    authorization: &SignedKeyAuthorization,
) -> eyre::Result<()> {
    eyre::ensure!(
        authorization.authorization.key_id == session.key_address,
        "session {} key_authorization key_id is {}, expected {}",
        session.session_id,
        authorization.authorization.key_id,
        session.key_address
    );
    eyre::ensure!(
        authorization.authorization.chain_id == session.chain_id,
        "session {} key_authorization chain_id is {}, expected {}",
        session.session_id,
        authorization.authorization.chain_id,
        session.chain_id
    );
    let expected_key_type = key_type_to_signature_type(key.key_type);
    eyre::ensure!(
        authorization.authorization.key_type == expected_key_type,
        "session {} key_authorization key_type is {:?}, expected {:?}",
        session.session_id,
        authorization.authorization.key_type,
        expected_key_type
    );
    let recovered = authorization
        .recover_signer()
        .map_err(|err| eyre::eyre!("failed to recover session key_authorization signer: {err}"))?;
    eyre::ensure!(
        recovered == session.root_account,
        "session {} key_authorization signer is {}, expected {}",
        session.session_id,
        recovered,
        session.root_account
    );
    validate_session_authorization_policy(session, authorization)?;
    Ok(())
}

/// Ensures authorization expiry, limits, and call scope match the stored session policy.
fn validate_session_authorization_policy(
    session: &SessionEntry,
    authorization: &SignedKeyAuthorization,
) -> eyre::Result<()> {
    let auth = &authorization.authorization;

    let expected_expiry = NonZeroU64::new(session.expiry)
        .ok_or_else(|| eyre::eyre!("session {} has invalid zero expiry", session.session_id))?;
    eyre::ensure!(
        auth.expiry == Some(expected_expiry),
        "session {} key_authorization expiry is {:?}, expected {}",
        session.session_id,
        auth.expiry.map(NonZeroU64::get),
        session.expiry
    );

    let expected_limits = session_authorization_limits(session)?;
    let actual_limits = auth.limits.as_deref().map(authorization_limits);
    eyre::ensure!(
        actual_limits == expected_limits,
        "session {} key_authorization limits do not match session limits",
        session.session_id
    );

    let expected_scope = session_authorization_scope(session);
    let actual_scope = auth.allowed_calls.as_deref().map(authorization_scope);
    eyre::ensure!(
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
    session
        .limits
        .as_deref()
        .map(|limits| {
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
            Ok(limits)
        })
        .transpose()
}

/// Converts signed authorization limits into canonical form for session comparison.
fn authorization_limits(
    limits: &[tempo_primitives::transaction::TokenLimit],
) -> Vec<CanonicalTokenLimit> {
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
    session.scope.as_deref().map(|scope| {
        let mut scope = scope
            .iter()
            .map(|scope| CanonicalCallScope {
                target: scope.target,
                selector_rules: session_authorization_selector_rules(&scope.selector_rules),
            })
            .collect::<Vec<_>>();
        scope.sort();
        scope
    })
}

/// Converts signed authorization scope into canonical form for session comparison.
fn authorization_scope(
    scope: &[tempo_primitives::transaction::CallScope],
) -> Vec<CanonicalCallScope> {
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
fn authorization_selector_rules(
    rules: &[tempo_primitives::transaction::SelectorRule],
) -> Vec<CanonicalSelectorRule> {
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
const fn key_type_to_signature_type(
    key_type: KeyType,
) -> tempo_primitives::transaction::SignatureType {
    match key_type {
        KeyType::Secp256k1 => tempo_primitives::transaction::SignatureType::Secp256k1,
        KeyType::P256 => tempo_primitives::transaction::SignatureType::P256,
        KeyType::WebAuthn => tempo_primitives::transaction::SignatureType::WebAuthn,
    }
}

/// Atomically upsert a [`SessionEntry`] into the session registry.
pub fn upsert_session_entry(entry: SessionEntry) -> eyre::Result<()> {
    let path =
        session_registry_path().ok_or_else(|| eyre::eyre!("could not resolve tempo home"))?;
    let mut record = read_toml_file::<SessionRecord>(&path, "tempo sessions")?.unwrap_or_default();
    record.upsert(entry);

    write_toml_file_atomic(&path, &record, SESSIONS_HEADER)
}

/// Atomically remove a session from the registry.
pub fn remove_session_entry(session_id: B256) -> eyre::Result<bool> {
    let path =
        session_registry_path().ok_or_else(|| eyre::eyre!("could not resolve tempo home"))?;
    let mut record = read_toml_file::<SessionRecord>(&path, "tempo sessions")?.unwrap_or_default();
    let removed = record.remove(session_id);
    if removed {
        write_toml_file_atomic(&path, &record, SESSIONS_HEADER)?;
    }
    Ok(removed)
}

/// Mark expired live sessions in the registry and persist the status updates.
pub fn mark_expired_session_entries(now: u64) -> eyre::Result<usize> {
    let path =
        session_registry_path().ok_or_else(|| eyre::eyre!("could not resolve tempo home"))?;
    let Some(mut record) = read_toml_file::<SessionRecord>(&path, "tempo sessions")? else {
        return Ok(0);
    };

    let updated = record.mark_expired(now);
    if updated != 0 {
        write_toml_file_atomic(&path, &record, SESSIONS_HEADER)?;
    }
    Ok(updated)
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
    use tempo_primitives::transaction::{
        CallScope, KeyAuthorization, PrimitiveSignature, SelectorRule, SignatureType, TokenLimit,
    };

    const ROOT_PRIVATE_KEY: &str =
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
    const SESSION_PRIVATE_KEY: &str =
        "0x59c6995e998f97a5a004497e5da3b5d2b2b66a87f064d39c44da0b6d6e4f8ff0";

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
        .with_expiry(entry.expiry);
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
    fn session_entry_roundtrips_scope_limits_and_status() {
        let entry = sample_entry_with_key(B256::from([0x66; 32]), 1234, SessionStatus::Revoking);
        let toml = toml::to_string(&entry).unwrap();
        let decoded: SessionEntry = toml::from_str(&toml).unwrap();

        assert_eq!(decoded.session_id, entry.session_id);
        assert_eq!(decoded.scope.as_ref().unwrap().len(), 1);
        assert_eq!(decoded.limits.as_ref().unwrap().len(), 1);
        assert_eq!(decoded.status, SessionStatus::Revoking);
        assert_eq!(decoded.key.as_ref().unwrap().key, "0xdeadbeef");
        assert!(decoded.has_inline_key());
        assert!(decoded.is_expired_at(1234));
    }

    #[test]
    fn live_session_key_requires_key_material_live_status_and_unexpired_entry() {
        let live_id = B256::from([0x01; 32]);
        let expired_id = B256::from([0x02; 32]);
        let revoked_id = B256::from([0x03; 32]);
        let no_key_id = B256::from([0x04; 32]);
        let pending_id = B256::from([0x05; 32]);

        let record = SessionRecord {
            sessions: vec![
                sample_entry_with_key(live_id, 200, SessionStatus::Active),
                sample_entry_with_key(expired_id, 100, SessionStatus::Active),
                sample_entry_with_key(revoked_id, 200, SessionStatus::Revoked),
                sample_entry(no_key_id, 200, SessionStatus::Active),
                sample_entry_with_key(pending_id, 200, SessionStatus::Pending),
            ],
        };

        assert_eq!(record.live_key(live_id, 100).unwrap().session_id, live_id);
        assert!(record.live_key(expired_id, 100).is_none());
        assert!(record.live_key(revoked_id, 100).is_none());
        assert!(record.live_key(no_key_id, 100).is_none());
        assert!(record.live_key(pending_id, 100).is_none());
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
    fn mark_expired_session_entries_clears_terminal_session_keys() {
        with_tempo_home(|| {
            let expired_id = B256::from([0xbc; 32]);
            let revoked_id = B256::from([0xbd; 32]);
            let failed_id = B256::from([0xbe; 32]);

            upsert_session_entry(sample_entry_with_key(expired_id, 100, SessionStatus::Expired))
                .unwrap();
            upsert_session_entry(sample_entry_with_key(revoked_id, 200, SessionStatus::Revoked))
                .unwrap();
            upsert_session_entry(sample_entry_with_key(failed_id, 200, SessionStatus::Failed))
                .unwrap();

            assert_eq!(mark_expired_session_entries(100).unwrap(), 3);
            let record = read_session_record().unwrap();
            for session_id in [expired_id, revoked_id, failed_id] {
                assert!(record.get(session_id).unwrap().key.is_none());
            }
            assert_eq!(record.get(expired_id).unwrap().status, SessionStatus::Expired);
            assert_eq!(record.get(revoked_id).unwrap().status, SessionStatus::Revoked);
            assert_eq!(record.get(failed_id).unwrap().status, SessionStatus::Failed);
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
}
