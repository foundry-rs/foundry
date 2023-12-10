//! Subscription types

use crate::eth::block::Header;
use ethers_core::{
    rand::{distributions::Alphanumeric, thread_rng, Rng},
    types::{Filter, Log, TxHash},
    utils::hex,
};
use std::fmt;

/// Result of a subscription
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(untagged))]
pub enum SubscriptionResult {
    /// New block header
    Header(Box<Header>),
    /// Log
    Log(Box<Log>),
    /// Transaction hash
    TransactionHash(TxHash),
    /// SyncStatus
    Sync(SyncStatus),
}

/// Sync status
#[derive(Debug, Eq, PartialEq, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct SyncStatus {
    pub syncing: bool,
}

/// Params for a subscription request
#[derive(Debug, PartialEq, Eq, Hash, Clone, Default)]
pub struct SubscriptionParams {
    /// holds the filter params field if present in the request
    pub filter: Option<Filter>,
}

#[cfg(feature = "serde")]
impl<'a> serde::Deserialize<'a> for SubscriptionParams {
    fn deserialize<D>(deserializer: D) -> Result<SubscriptionParams, D::Error>
    where
        D: serde::Deserializer<'a>,
    {
        use serde::de::Error;

        let val = serde_json::Value::deserialize(deserializer)?;
        if val.is_null() {
            return Ok(SubscriptionParams::default())
        }

        let filter: Filter = serde_json::from_value(val)
            .map_err(|e| D::Error::custom(format!("Invalid Subscription parameters: {e}")))?;
        Ok(SubscriptionParams { filter: Some(filter) })
    }
}

/// Subscription kind
#[derive(Debug, PartialEq, Eq, Hash, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub enum SubscriptionKind {
    /// subscribe to new heads
    NewHeads,
    /// subscribe to new logs
    Logs,
    /// subscribe to pending transactions
    NewPendingTransactions,
    /// syncing subscription
    Syncing,
}

/// Unique subscription id
#[derive(Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[cfg_attr(feature = "serde", serde(untagged))]
pub enum SubscriptionId {
    /// numerical sub id
    Number(u64),
    /// string sub id, a hash for example
    String(String),
}

// === impl SubscriptionId ===

impl SubscriptionId {
    /// Generates a new random hex identifier
    pub fn random_hex() -> Self {
        SubscriptionId::String(hex_id())
    }
}

impl fmt::Display for SubscriptionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SubscriptionId::Number(num) => num.fmt(f),
            SubscriptionId::String(s) => s.fmt(f),
        }
    }
}

impl fmt::Debug for SubscriptionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SubscriptionId::Number(num) => num.fmt(f),
            SubscriptionId::String(s) => s.fmt(f),
        }
    }
}

/// Provides random hex identifier with a certain length
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct HexIdProvider {
    len: usize,
}

// === impl  HexIdProvider ===

impl HexIdProvider {
    /// Generates a random hex encoded Id
    pub fn gen(&self) -> String {
        let id: String =
            (&mut thread_rng()).sample_iter(Alphanumeric).map(char::from).take(self.len).collect();
        let out = hex::encode(id);
        format!("0x{out}")
    }
}

impl Default for HexIdProvider {
    fn default() -> Self {
        Self { len: 16 }
    }
}

/// Returns a new random hex identifier
pub fn hex_id() -> String {
    HexIdProvider::default().gen()
}
