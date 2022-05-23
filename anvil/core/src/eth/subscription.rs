//! Subscription types

use crate::eth::{block::Header, filter::Filter};
use ethers_core::{
    rand::{distributions::Alphanumeric, thread_rng, Rng},
    types::{Log, TxHash},
    utils::hex,
};
use serde::{de::Error, Deserialize, Deserializer, Serialize};
use std::fmt;

/// Result of a subscription
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
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
#[derive(Debug, Serialize, Eq, PartialEq, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub syncing: bool,
}

/// Params for a subscription request
#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub enum SubscriptionParams {
    /// no `params`
    None,
    /// `Filter` parameters.
    Logs(Filter),
}

impl Default for SubscriptionParams {
    fn default() -> Self {
        SubscriptionParams::None
    }
}

impl<'a> Deserialize<'a> for SubscriptionParams {
    fn deserialize<D>(deserializer: D) -> Result<SubscriptionParams, D::Error>
    where
        D: Deserializer<'a>,
    {
        let val = serde_json::Value::deserialize(deserializer)?;
        if val.is_null() {
            return Ok(SubscriptionParams::None)
        }

        let filter: Filter = serde_json::from_value(val)
            .map_err(|e| D::Error::custom(format!("Invalid Subscription parameters: {}", e)))?;
        Ok(SubscriptionParams::Logs(filter))
    }
}

/// Subscription kind
#[derive(Debug, Deserialize, PartialEq, Eq, Hash, Clone)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
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
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(untagged)]
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
        format!("0x{}", out)
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
