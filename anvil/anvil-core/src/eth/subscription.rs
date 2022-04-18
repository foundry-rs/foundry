//! Subscription types

use crate::eth::{block::Header, filter::Filter};
use ethers_core::types::{Log, TxHash};
use serde::{de::Error, Deserialize, Deserializer, Serialize};

/// Result of a subscription
#[derive(Debug, Clone, PartialEq)]
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
#[derive(Debug, PartialEq, Hash, Clone)]
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
