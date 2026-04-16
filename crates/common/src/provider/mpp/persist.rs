//! Persistent channel storage for MPP sessions.
//!
//! Stores open payment channel state in a JSON file at
//! `$TEMPO_HOME/foundry/channels.json` (default: `~/.tempo/foundry/channels.json`).
//! This allows channel reuse across process invocations, avoiding the cost of
//! opening a new on-chain channel for every `cast` / `forge` command.

use alloy_primitives::{Address, B256};
use mpp::client::channel_ops::ChannelEntry;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
use tracing::{debug, warn};

use crate::tempo::tempo_home;

/// Relative path from Tempo home to the Foundry channels file.
const CHANNELS_PATH: &str = "foundry/channels.json";

/// Current schema version.
const SCHEMA_VERSION: u64 = 2;

/// On-disk representation of the channel store.
#[derive(Debug, Serialize, Deserialize)]
struct ChannelStore {
    version: u64,
    #[serde(default)]
    channels: HashMap<String, PersistedChannel>,
}

impl Default for ChannelStore {
    fn default() -> Self {
        Self { version: SCHEMA_VERSION, channels: HashMap::new() }
    }
}

/// A persisted channel entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedChannel {
    pub channel_id: String,
    pub salt: String,
    pub escrow_contract: String,
    pub chain_id: u64,
    pub cumulative_amount: String,
    pub deposit: String,
    pub status: String,
    pub origin: String,
    pub created_at: u64,
    pub last_used_at: u64,
}

impl PersistedChannel {
    /// Convert to an mpp `ChannelEntry` for use in the session provider.
    pub fn to_channel_entry(&self) -> Option<ChannelEntry> {
        let channel_id: B256 = self.channel_id.parse().ok()?;
        let salt: B256 = self.salt.parse().ok()?;
        let escrow_contract: Address = self.escrow_contract.parse().ok()?;
        let cumulative_amount: u128 = self.cumulative_amount.parse().ok()?;

        Some(ChannelEntry {
            channel_id,
            salt,
            cumulative_amount,
            escrow_contract,
            chain_id: self.chain_id,
            opened: self.status == "active",
        })
    }

    /// Create from a `ChannelEntry` with metadata.
    pub fn from_channel_entry(entry: &ChannelEntry, deposit: u128, origin: &str) -> Self {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();

        Self {
            channel_id: entry.channel_id.to_string(),
            salt: entry.salt.to_string(),
            escrow_contract: entry.escrow_contract.to_string(),
            chain_id: entry.chain_id,
            cumulative_amount: entry.cumulative_amount.to_string(),
            deposit: deposit.to_string(),
            status: if entry.opened { "active" } else { "closed" }.to_string(),
            origin: origin.to_string(),
            created_at: now,
            last_used_at: now,
        }
    }

    /// Whether this channel can still be used (active and not fully spent).
    fn is_usable(&self) -> bool {
        if self.status != "active" {
            return false;
        }
        let cumulative: u128 = self.cumulative_amount.parse().unwrap_or(u128::MAX);
        let deposit: u128 = self.deposit.parse().unwrap_or(0);
        cumulative < deposit
    }
}

/// Returns the path to the channels file.
fn channels_path() -> Option<PathBuf> {
    tempo_home().map(|home| home.join(CHANNELS_PATH))
}

/// Load channels from disk, evicting spent/inactive entries.
pub fn load_channels() -> HashMap<String, PersistedChannel> {
    let Some(path) = channels_path().filter(|p| p.exists()) else {
        return HashMap::new();
    };

    let Ok(contents) = std::fs::read_to_string(&path).inspect_err(|e| {
        warn!(?path, %e, "failed to read channels file");
    }) else {
        return HashMap::new();
    };

    let Ok(store) = serde_json::from_str::<ChannelStore>(&contents).inspect_err(|e| {
        warn!(?path, %e, "failed to parse channels file, starting fresh");
    }) else {
        return HashMap::new();
    };

    if store.version != SCHEMA_VERSION {
        warn!(
            version = store.version,
            expected = SCHEMA_VERSION,
            "channels file version mismatch, starting fresh"
        );
        return HashMap::new();
    }

    // Evict spent/inactive entries
    let usable: HashMap<String, PersistedChannel> =
        store.channels.into_iter().filter(|(_, ch)| ch.is_usable()).collect();

    debug!(count = usable.len(), "loaded persisted MPP channels");
    usable
}

/// Save channels to disk.
pub fn save_channels(channels: &HashMap<String, PersistedChannel>) {
    let Some(path) = channels_path() else {
        return;
    };

    if let Some(parent) = path.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        warn!(?path, %e, "failed to create channels directory");
        return;
    }

    let store = ChannelStore { version: SCHEMA_VERSION, channels: channels.clone() };

    match serde_json::to_string_pretty(&store) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, json) {
                warn!(?path, %e, "failed to write channels file");
            } else {
                debug!(?path, count = channels.len(), "saved MPP channels");
            }
        }
        Err(e) => warn!(%e, "failed to serialize channels"),
    }
}

/// Look up a usable persisted channel by key.
pub fn find_channel(
    channels: &HashMap<String, PersistedChannel>,
    key: &str,
) -> Option<ChannelEntry> {
    channels.get(key).filter(|ch| ch.is_usable()).and_then(|ch| ch.to_channel_entry())
}

/// Insert or update a channel entry in memory only (no disk write).
///
/// Use [`upsert_channel`] when you want to persist immediately, or call
/// [`save_channels`] separately after this.
pub fn upsert_channel_in_memory(
    channels: &mut HashMap<String, PersistedChannel>,
    key: &str,
    entry: &ChannelEntry,
    deposit: u128,
    origin: &str,
) {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();

    if let Some(existing) = channels.get_mut(key) {
        existing.cumulative_amount = entry.cumulative_amount.to_string();
        existing.last_used_at = now;
        existing.status = if entry.opened { "active" } else { "closed" }.to_string();
    } else {
        channels
            .insert(key.to_string(), PersistedChannel::from_channel_entry(entry, deposit, origin));
    }
}

/// Insert or update a channel entry and save to disk.
///
/// When updating an existing entry, `deposit` is ignored (preserved from the
/// original open). When inserting a new entry, `deposit` is recorded.
pub fn upsert_channel(
    channels: &mut HashMap<String, PersistedChannel>,
    key: &str,
    entry: &ChannelEntry,
    deposit: u128,
    origin: &str,
) {
    upsert_channel_in_memory(channels, key, entry, deposit, origin);
    save_channels(channels);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_channel(status: &str, cumulative: &str, deposit: &str) -> PersistedChannel {
        PersistedChannel {
            channel_id: format!("0x{}", "ab".repeat(32)),
            salt: format!("0x{}", "cd".repeat(32)),
            escrow_contract: "0xe1c4d3dce17bc111181ddf716f75bae49e61a336".to_string(),
            chain_id: 42431,
            cumulative_amount: cumulative.to_string(),
            deposit: deposit.to_string(),
            status: status.to_string(),
            origin: "https://rpc.mpp.moderato.tempo.xyz".to_string(),
            created_at: 1000,
            last_used_at: 1000,
        }
    }

    #[test]
    fn is_usable() {
        assert!(test_channel("active", "5000", "100000").is_usable());
        assert!(!test_channel("active", "100000", "100000").is_usable());
        assert!(!test_channel("active", "200000", "100000").is_usable());
        assert!(!test_channel("closed", "0", "100000").is_usable());
        assert!(!test_channel("closing", "0", "100000").is_usable());
    }

    #[test]
    fn channel_entry_round_trip() {
        let entry = ChannelEntry {
            channel_id: B256::random(),
            salt: B256::random(),
            cumulative_amount: 42000,
            escrow_contract: Address::random(),
            chain_id: 42431,
            opened: true,
        };

        let persisted = PersistedChannel::from_channel_entry(&entry, 100_000, "https://rpc.test");
        let restored = persisted.to_channel_entry().expect("should parse back");

        assert_eq!(restored.channel_id, entry.channel_id);
        assert_eq!(restored.salt, entry.salt);
        assert_eq!(restored.cumulative_amount, entry.cumulative_amount);
        assert_eq!(restored.escrow_contract, entry.escrow_contract);
        assert_eq!(restored.chain_id, entry.chain_id);
        assert!(restored.opened);
    }

    #[test]
    fn load_evicts_and_handles_edge_cases() {
        let dir = tempfile::tempdir().unwrap();
        let foundry_dir = dir.path().join("foundry");
        std::fs::create_dir_all(&foundry_dir).unwrap();

        let store = ChannelStore {
            version: SCHEMA_VERSION,
            channels: HashMap::from([
                ("active".into(), test_channel("active", "1000", "100000")),
                ("spent".into(), test_channel("active", "100000", "100000")),
                ("closed".into(), test_channel("closed", "0", "100000")),
            ]),
        };
        let json = serde_json::to_string(&store).unwrap();
        std::fs::write(foundry_dir.join("channels.json"), &json).unwrap();

        unsafe { std::env::set_var("TEMPO_HOME", dir.path()) };
        let loaded = load_channels();
        unsafe { std::env::remove_var("TEMPO_HOME") };

        assert_eq!(loaded.len(), 1);
        assert!(loaded.contains_key("active"));
    }

    #[test]
    fn load_missing_and_wrong_version() {
        let dir = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("TEMPO_HOME", dir.path()) };
        assert!(load_channels().is_empty());

        let foundry_dir = dir.path().join("foundry");
        std::fs::create_dir_all(&foundry_dir).unwrap();
        std::fs::write(foundry_dir.join("channels.json"), r#"{"version": 999, "channels": {}}"#)
            .unwrap();
        assert!(load_channels().is_empty());

        unsafe { std::env::remove_var("TEMPO_HOME") };
    }

    #[test]
    fn find_channel_filters_unusable() {
        let mut channels = HashMap::new();
        channels.insert("usable".into(), test_channel("active", "1000", "100000"));
        channels.insert("spent".into(), test_channel("active", "100000", "100000"));

        assert!(find_channel(&channels, "usable").is_some());
        assert!(find_channel(&channels, "spent").is_none());
        assert!(find_channel(&channels, "missing").is_none());
    }

    #[test]
    fn upsert_inserts_and_updates() {
        let dir = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("TEMPO_HOME", dir.path()) };

        let mut channels = HashMap::new();
        let entry = ChannelEntry {
            channel_id: B256::random(),
            salt: B256::random(),
            cumulative_amount: 1000,
            escrow_contract: Address::random(),
            chain_id: 42431,
            opened: true,
        };

        upsert_channel(&mut channels, "key1", &entry, 100_000, "https://rpc.test");
        assert_eq!(channels["key1"].cumulative_amount, "1000");
        assert_eq!(channels["key1"].deposit, "100000");
        let created_at = channels["key1"].created_at;

        let mut updated = entry.clone();
        updated.cumulative_amount = 5000;
        upsert_channel(&mut channels, "key1", &updated, 0, "https://rpc.test");
        assert_eq!(channels["key1"].cumulative_amount, "5000");
        assert_eq!(channels["key1"].deposit, "100000");
        assert_eq!(channels["key1"].created_at, created_at);

        unsafe { std::env::remove_var("TEMPO_HOME") };
    }
}
