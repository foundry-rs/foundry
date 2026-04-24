//! Persistent channel storage for MPP sessions.
//!
//! Stores open payment channel state in a SQLite database at
//! `$TEMPO_HOME/channels.db` (default: `~/.tempo/channels.db`).
//! This allows channel reuse across process invocations, avoiding the cost of
//! opening a new on-chain channel for every `cast` / `forge` command.

use alloy_primitives::{Address, B256};
use foundry_wallets::{Channel, ChannelDb};
use mpp::client::channel_ops::ChannelEntry;
use std::{
    collections::HashMap,
    sync::OnceLock,
    time::{SystemTime, UNIX_EPOCH},
};
use tracing::{debug, warn};

use crate::tempo::tempo_home;

/// Process-wide database handle.
fn global_db() -> Option<&'static ChannelDb> {
    static DB: OnceLock<Option<ChannelDb>> = OnceLock::new();
    DB.get_or_init(|| {
        let path = tempo_home()?.join("channels.db");
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Some(old) =
            tempo_home().map(|h| h.join("foundry/channels.json")).filter(|p| p.exists())
        {
            warn!(
                ?old,
                "found old channels.json — this file is no longer used; channels will be re-opened"
            );
        }

        match ChannelDb::open(&path) {
            Ok(db) => {
                debug!(?path, "opened channel database");
                Some(db)
            }
            Err(e) => {
                warn!(?path, %e, "failed to open channel database");
                None
            }
        }
    })
    .as_ref()
}

/// Reconstruct the composite HashMap key from a persisted `Channel`.
///
/// Mirrors `SessionProvider::channel_key()` in session.rs.
fn channel_key_from_persisted(ch: &Channel) -> String {
    let origin_hash = &alloy_primitives::keccak256(ch.origin.as_bytes()).to_string()[..18];
    format!(
        "{}:{}:{}:{}:{}:{}:{}",
        origin_hash,
        ch.chain_id,
        ch.payer,
        ch.authorized_signer,
        ch.payee,
        ch.token,
        ch.escrow_contract
    )
    .to_lowercase()
}

/// Whether a channel can still be used (active and not fully spent).
fn is_usable(ch: &Channel) -> bool {
    if ch.state != "active" {
        return false;
    }
    let cumulative: u128 = ch.cumulative_amount.parse().unwrap_or(u128::MAX);
    let deposit: u128 = ch.deposit.parse().unwrap_or(0);
    cumulative < deposit
}

/// Convert a persisted `Channel` to a `ChannelEntry`.
pub fn to_channel_entry(ch: &Channel) -> Option<ChannelEntry> {
    let channel_id: B256 = ch.channel_id.parse().ok()?;
    let salt: B256 = ch.salt.parse().ok()?;
    let escrow_contract: Address = ch.escrow_contract.parse().ok()?;
    let cumulative_amount: u128 = ch.cumulative_amount.parse().ok()?;

    Some(ChannelEntry {
        channel_id,
        salt,
        cumulative_amount,
        escrow_contract,
        chain_id: ch.chain_id as u64,
        opened: ch.state == "active",
    })
}

/// Create a `Channel` from a `ChannelEntry` with metadata.
pub fn from_channel_entry(
    entry: &ChannelEntry,
    deposit: u128,
    origin: &str,
    payer: &Address,
    payee: &Address,
    token: &Address,
    authorized_signer: &Address,
) -> Channel {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64;

    Channel {
        channel_id: entry.channel_id.to_string(),
        version: 1,
        origin: origin.to_string(),
        request_url: String::new(),
        chain_id: entry.chain_id as i64,
        escrow_contract: entry.escrow_contract.to_string(),
        token: token.to_string(),
        payee: payee.to_string(),
        payer: payer.to_string(),
        authorized_signer: authorized_signer.to_string(),
        salt: entry.salt.to_string(),
        deposit: deposit.to_string(),
        cumulative_amount: entry.cumulative_amount.to_string(),
        challenge_echo: String::new(),
        state: if entry.opened { "active" } else { "closed" }.to_string(),
        close_requested_at: 0,
        grace_ready_at: 0,
        created_at: now,
        last_used_at: now,
    }
}

/// Load channels from database, evicting spent/inactive entries.
pub fn load_channels() -> HashMap<String, Channel> {
    let Some(db) = global_db() else {
        return HashMap::new();
    };

    let channels = match db.load() {
        Ok(channels) => channels,
        Err(e) => {
            warn!(%e, "failed to load channels from database");
            return HashMap::new();
        }
    };

    let usable: HashMap<String, Channel> = channels
        .into_iter()
        .filter(is_usable)
        .map(|ch| {
            let key = channel_key_from_persisted(&ch);
            (key, ch)
        })
        .collect();

    debug!(count = usable.len(), "loaded persisted MPP channels");
    usable
}

/// Save channels to database.
pub fn save_channels(channels: &HashMap<String, Channel>) {
    let Some(db) = global_db() else {
        return;
    };

    for ch in channels.values() {
        if let Err(e) = db.upsert(ch) {
            warn!(%e, channel_id = %ch.channel_id, "failed to save channel");
        }
    }
    debug!(count = channels.len(), "saved MPP channels");
}

/// Delete a channel from the database by its channel ID.
pub fn delete_channel_from_db(channel_id: &str) {
    let Some(db) = global_db() else {
        return;
    };
    if let Err(e) = db.delete(channel_id) {
        warn!(%e, channel_id, "failed to delete channel from database");
    }
}

/// Look up a usable persisted channel by key.
pub fn find_channel(channels: &HashMap<String, Channel>, key: &str) -> Option<ChannelEntry> {
    channels.get(key).filter(|ch| is_usable(ch)).and_then(to_channel_entry)
}

/// Insert or update a channel entry in memory only (no DB write).
pub fn upsert_channel_in_memory(
    channels: &mut HashMap<String, Channel>,
    key: &str,
    entry: &ChannelEntry,
) {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64;

    if let Some(existing) = channels.get_mut(key) {
        existing.cumulative_amount = entry.cumulative_amount.to_string();
        existing.last_used_at = now;
        existing.state = if entry.opened { "active" } else { "closed" }.to_string();
    } else {
        warn!(key, "upsert_channel_in_memory called for unknown channel");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_channel(state: &str, cumulative: &str, deposit: &str) -> Channel {
        Channel {
            channel_id: format!("0x{}", "ab".repeat(32)),
            version: 1,
            origin: "https://rpc.mpp.moderato.tempo.xyz".to_string(),
            request_url: String::new(),
            chain_id: 42431,
            escrow_contract: "0xe1c4d3dce17bc111181ddf716f75bae49e61a336".to_string(),
            token: "0x20c0000000000000000000000000000000000000".to_string(),
            payee: "0x3333333333333333333333333333333333333333".to_string(),
            payer: "0x1111111111111111111111111111111111111111".to_string(),
            authorized_signer: "0x1111111111111111111111111111111111111111".to_string(),
            salt: format!("0x{}", "cd".repeat(32)),
            deposit: deposit.to_string(),
            cumulative_amount: cumulative.to_string(),
            challenge_echo: String::new(),
            state: state.to_string(),
            close_requested_at: 0,
            grace_ready_at: 0,
            created_at: 1000,
            last_used_at: 1000,
        }
    }

    #[test]
    fn usable() {
        assert!(is_usable(&test_channel("active", "5000", "100000")));
        assert!(!is_usable(&test_channel("active", "100000", "100000")));
        assert!(!is_usable(&test_channel("active", "200000", "100000")));
        assert!(!is_usable(&test_channel("closed", "0", "100000")));
        assert!(!is_usable(&test_channel("closing", "0", "100000")));
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

        let payer = Address::random();
        let payee = Address::random();
        let token = Address::random();
        let persisted =
            from_channel_entry(&entry, 100_000, "https://rpc.test", &payer, &payee, &token, &payer);
        let restored = to_channel_entry(&persisted).expect("should parse back");

        assert_eq!(restored.channel_id, entry.channel_id);
        assert_eq!(restored.salt, entry.salt);
        assert_eq!(restored.cumulative_amount, entry.cumulative_amount);
        assert_eq!(restored.escrow_contract, entry.escrow_contract);
        assert_eq!(restored.chain_id, entry.chain_id);
        assert!(restored.opened);
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
}
