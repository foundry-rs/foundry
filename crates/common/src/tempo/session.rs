//! Tempo session registry and local lifecycle metadata.

use super::{registry::*, tempo_home};
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
}

impl SessionEntry {
    /// Returns `true` if the session has passed its expiry timestamp.
    pub const fn is_expired_at(&self, now: u64) -> bool {
        now >= self.expiry
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

    /// Mark expired live entries as expired. Returns the number updated.
    pub fn mark_expired(&mut self, now: u64) -> usize {
        let mut updated = 0;
        for session in &mut self.sessions {
            if session.status.is_live() && session.is_expired_at(now) {
                session.status = SessionStatus::Expired;
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
        let entry = sample_entry(B256::from([0x66; 32]), 1234, SessionStatus::Revoking);
        let toml = toml::to_string(&entry).unwrap();
        let decoded: SessionEntry = toml::from_str(&toml).unwrap();

        assert_eq!(decoded.session_id, entry.session_id);
        assert_eq!(decoded.scope.len(), 1);
        assert_eq!(decoded.limits.len(), 1);
        assert_eq!(decoded.status, SessionStatus::Revoking);
        assert!(decoded.is_expired_at(1234));
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
}
