//! Ethereum types for pub-sub

use crate::{Filter, Header, Log, Transaction};
use alloc::{boxed::Box, format};
use alloy_primitives::B256;
use alloy_serde::WithOtherFields;

/// Subscription result.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(untagged))]
pub enum SubscriptionResult<T = Transaction> {
    /// New block header.
    Header(Box<WithOtherFields<Header>>),
    /// Log
    Log(Box<Log>),
    /// Transaction hash
    TransactionHash(B256),
    /// Full Transaction
    FullTransaction(Box<T>),
    /// SyncStatus
    SyncState(PubSubSyncStatus),
}

/// Response type for a SyncStatus subscription.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(untagged))]
pub enum PubSubSyncStatus {
    /// If not currently syncing, this should always be `false`.
    Simple(bool),
    /// Syncing metadata.
    Detailed(SyncStatusMetadata),
}

/// Sync status metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct SyncStatusMetadata {
    /// Whether the node is currently syncing.
    pub syncing: bool,
    /// The starting block.
    pub starting_block: u64,
    /// The current block.
    pub current_block: u64,
    /// The highest block.
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))]
    pub highest_block: Option<u64>,
}

#[cfg(feature = "serde")]
impl<T> serde::Serialize for SubscriptionResult<T>
where
    T: serde::Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match *self {
            Self::Header(ref header) => header.serialize(serializer),
            Self::Log(ref log) => log.serialize(serializer),
            Self::TransactionHash(ref hash) => hash.serialize(serializer),
            Self::FullTransaction(ref tx) => tx.serialize(serializer),
            Self::SyncState(ref sync) => sync.serialize(serializer),
        }
    }
}

/// Subscription kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub enum SubscriptionKind {
    /// New block headers subscription.
    ///
    /// Fires a notification each time a new header is appended to the chain, including chain
    /// reorganizations. In case of a chain reorganization the subscription will emit all new
    /// headers for the new chain. Therefore the subscription can emit multiple headers on the same
    /// height.
    NewHeads,
    /// Logs subscription.
    ///
    /// Returns logs that are included in new imported blocks and match the given filter criteria.
    /// In case of a chain reorganization previous sent logs that are on the old chain will be
    /// resent with the removed property set to true. Logs from transactions that ended up in the
    /// new chain are emitted. Therefore, a subscription can emit logs for the same transaction
    /// multiple times.
    Logs,
    /// New Pending Transactions subscription.
    ///
    /// Returns the hash or full tx for all transactions that are added to the pending state and
    /// are signed with a key that is available in the node. When a transaction that was
    /// previously part of the canonical chain isn't part of the new canonical chain after a
    /// reorganization its again emitted.
    NewPendingTransactions,
    /// Node syncing status subscription.
    ///
    /// Indicates when the node starts or stops synchronizing. The result can either be a boolean
    /// indicating that the synchronization has started (true), finished (false) or an object with
    /// various progress indicators.
    Syncing,
}

/// Any additional parameters for a subscription.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Params {
    /// No parameters passed.
    #[default]
    None,
    /// Log parameters.
    Logs(Box<Filter>),
    /// Boolean parameter for new pending transactions.
    Bool(bool),
}

impl Params {
    /// Returns true if it's a bool parameter.
    #[inline]
    pub const fn is_bool(&self) -> bool {
        matches!(self, Self::Bool(_))
    }

    /// Returns true if it's a log parameter.
    #[inline]
    pub const fn is_logs(&self) -> bool {
        matches!(self, Self::Logs(_))
    }
}

impl From<Filter> for Params {
    fn from(filter: Filter) -> Self {
        Self::Logs(Box::new(filter))
    }
}

impl From<bool> for Params {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for Params {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::None => (&[] as &[serde_json::Value]).serialize(serializer),
            Self::Logs(logs) => logs.serialize(serializer),
            Self::Bool(full) => full.serialize(serializer),
        }
    }
}

#[cfg(feature = "serde")]
impl<'a> serde::Deserialize<'a> for Params {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'a>,
    {
        use serde::de::Error;

        let v = serde_json::Value::deserialize(deserializer)?;

        if v.is_null() {
            return Ok(Self::None);
        }

        if let Some(val) = v.as_bool() {
            return Ok(val.into());
        }

        serde_json::from_value::<Filter>(v)
            .map(Into::into)
            .map_err(|e| D::Error::custom(format!("Invalid Pub-Sub parameters: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use similar_asserts::assert_eq;

    #[test]
    #[cfg(feature = "serde")]
    fn params_serde() {
        // Test deserialization of boolean parameter
        let s: Params = serde_json::from_str("true").unwrap();
        assert_eq!(s, Params::Bool(true));

        // Test deserialization of null (None) parameter
        let s: Params = serde_json::from_str("null").unwrap();
        assert_eq!(s, Params::None);

        // Test deserialization of log parameters
        let filter = Filter::default();
        let s: Params = serde_json::from_str(&serde_json::to_string(&filter).unwrap()).unwrap();
        assert_eq!(s, Params::Logs(Box::new(filter)));
    }

    #[test]
    fn params_is_bool() {
        // Check if the `is_bool` method correctly identifies boolean parameters
        let param = Params::Bool(true);
        assert!(param.is_bool());

        let param = Params::None;
        assert!(!param.is_bool());

        let param = Params::Logs(Box::default());
        assert!(!param.is_bool());
    }

    #[test]
    fn params_is_logs() {
        // Check if the `is_logs` method correctly identifies log parameters
        let param = Params::Logs(Box::default());
        assert!(param.is_logs());

        let param = Params::None;
        assert!(!param.is_logs());

        let param = Params::Bool(true);
        assert!(!param.is_logs());
    }

    #[test]
    fn params_from_filter() {
        let filter = Filter::default();
        let param: Params = filter.clone().into();
        assert_eq!(param, Params::Logs(Box::new(filter)));
    }

    #[test]
    fn params_from_bool() {
        let param: Params = true.into();
        assert_eq!(param, Params::Bool(true));

        let param: Params = false.into();
        assert_eq!(param, Params::Bool(false));
    }

    #[test]
    #[cfg(feature = "serde")]
    fn params_serialize_none() {
        let param = Params::None;
        let serialized = serde_json::to_string(&param).unwrap();
        assert_eq!(serialized, "[]");
    }

    #[test]
    #[cfg(feature = "serde")]
    fn params_serialize_bool() {
        let param = Params::Bool(true);
        let serialized = serde_json::to_string(&param).unwrap();
        assert_eq!(serialized, "true");

        let param = Params::Bool(false);
        let serialized = serde_json::to_string(&param).unwrap();
        assert_eq!(serialized, "false");
    }

    #[test]
    #[cfg(feature = "serde")]
    fn params_serialize_logs() {
        let filter = Filter::default();
        let param = Params::Logs(Box::new(filter.clone()));
        let serialized = serde_json::to_string(&param).unwrap();
        let expected = serde_json::to_string(&filter).unwrap();
        assert_eq!(serialized, expected);
    }
}
