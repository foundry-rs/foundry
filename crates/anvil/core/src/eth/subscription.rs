//! Subscription types
use alloy_primitives::hex;
use rand::{distributions::Alphanumeric, thread_rng, Rng};
use std::fmt;

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

impl SubscriptionId {
    /// Generates a new random hex identifier
    pub fn random_hex() -> Self {
        Self::String(hex_id())
    }
}

impl fmt::Display for SubscriptionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Number(num) => num.fmt(f),
            Self::String(s) => s.fmt(f),
        }
    }
}

impl fmt::Debug for SubscriptionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Number(num) => num.fmt(f),
            Self::String(s) => s.fmt(f),
        }
    }
}

/// Provides random hex identifier with a certain length
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct HexIdProvider {
    len: usize,
}

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
