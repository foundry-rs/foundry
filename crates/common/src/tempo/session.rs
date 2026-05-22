//! Tempo session registry and local lifecycle metadata.

use super::{KeyType, registry::*, tempo_home};
use alloy_primitives::{Address, B256, Selector};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scope: Vec<SessionCallScope>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub limits: Vec<SessionTokenLimit>,
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
            if session.status.is_live() && session.is_expired_at(now) {
                session.status = SessionStatus::Expired;
                session.key = None;
                updated += 1;
            }
        }
        updated
    }
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
    use std::{fs, str::FromStr};

    fn sample_entry(session_id: B256, expiry: u64, status: SessionStatus) -> SessionEntry {
        SessionEntry {
            session_id,
            root_account: Address::from_str("0x0000000000000000000000000000000000000001").unwrap(),
            chain_id: 4217,
            key_address: Address::from_str("0x0000000000000000000000000000000000000abc").unwrap(),
            expiry,
            scope: vec![SessionCallScope {
                target: Address::from_str("0x00000000000000000000000000000000000000aa").unwrap(),
                selector_rules: vec![SessionSelectorRule {
                    selector: Selector::from_slice(&[0x12, 0x34, 0x56, 0x78]),
                    recipients: vec![],
                }],
            }],
            limits: vec![SessionTokenLimit {
                currency: Address::from_str("0x00000000000000000000000000000000000000ff").unwrap(),
                limit: "0".to_string(),
            }],
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
        assert_eq!(decoded.scope.len(), 1);
        assert_eq!(decoded.limits.len(), 1);
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
