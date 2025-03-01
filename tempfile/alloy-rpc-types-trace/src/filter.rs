//! `trace_filter` types and support.

use crate::parity::{
    Action, CallAction, CreateAction, CreateOutput, RewardAction, SelfdestructAction, TraceOutput,
    TransactionTrace,
};
use alloy_primitives::{map::AddressHashSet, Address};
use serde::{Deserialize, Serialize};

/// Trace filter.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct TraceFilter {
    /// From block
    #[serde(with = "alloy_serde::quantity::opt")]
    pub from_block: Option<u64>,
    /// To block
    #[serde(with = "alloy_serde::quantity::opt")]
    pub to_block: Option<u64>,
    /// From address
    #[serde(default)]
    pub from_address: Vec<Address>,
    /// To address
    #[serde(default)]
    pub to_address: Vec<Address>,
    /// How to apply `from_address` and `to_address` filters.
    #[serde(default)]
    pub mode: TraceFilterMode,
    /// Output offset
    pub after: Option<u64>,
    /// Output amount
    pub count: Option<u64>,
}

// === impl TraceFilter ===

impl TraceFilter {
    /// Sets the `from_block` field of the struct
    pub const fn from_block(mut self, block: u64) -> Self {
        self.from_block = Some(block);
        self
    }

    /// Sets the `to_block` field of the struct
    pub const fn to_block(mut self, block: u64) -> Self {
        self.to_block = Some(block);
        self
    }

    /// Sets the `from_address` field of the struct
    pub fn from_address(mut self, addresses: Vec<Address>) -> Self {
        self.from_address = addresses;
        self
    }

    /// Sets the `to_address` field of the struct
    pub fn to_address(mut self, addresses: Vec<Address>) -> Self {
        self.to_address = addresses;
        self
    }

    /// Sets the `after` field of the struct
    pub const fn after(mut self, after: u64) -> Self {
        self.after = Some(after);
        self
    }

    /// Sets the `count` field of the struct
    pub const fn count(mut self, count: u64) -> Self {
        self.count = Some(count);
        self
    }

    /// Sets the `from_address` field of the struct
    pub const fn mode(mut self, mode: TraceFilterMode) -> Self {
        self.mode = mode;
        self
    }

    /// Returns a `TraceFilterMatcher` for this filter.
    pub fn matcher(&self) -> TraceFilterMatcher {
        let from_addresses = self.from_address.iter().copied().collect();
        let to_addresses = self.to_address.iter().copied().collect();
        TraceFilterMatcher { mode: self.mode, from_addresses, to_addresses }
    }
}

/// How to apply `from_address` and `to_address` filters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TraceFilterMode {
    /// Return traces for transactions with matching `from` OR `to` addresses.
    #[default]
    Union,
    /// Only return traces for transactions with matching `from` _and_ `to` addresses.
    Intersection,
}

/// Address filter.
/// This is a set of addresses to match against.
/// An empty set matches all addresses.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AddressFilter(pub AddressHashSet);

impl FromIterator<Address> for AddressFilter {
    fn from_iter<I: IntoIterator<Item = Address>>(iter: I) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl From<Vec<Address>> for AddressFilter {
    fn from(addrs: Vec<Address>) -> Self {
        Self::from_iter(addrs)
    }
}

impl AddressFilter {
    /// Returns `true` if the given address is in the filter or the filter address set is empty.
    pub fn matches(&self, addr: &Address) -> bool {
        self.is_empty() || self.0.contains(addr)
    }

    /// Returns `true` if the address set is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// `TraceFilterMatcher` is a filter used for matching `TransactionTrace` based on it's action and
/// result (if available).
///
/// It allows filtering traces by their mode, from address set, and to address set, and empty
/// address set means match all addresses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceFilterMatcher {
    mode: TraceFilterMode,
    from_addresses: AddressFilter,
    to_addresses: AddressFilter,
}

impl TraceFilterMatcher {
    /// Returns `true` if the given `TransactionTrace` matches this filter.
    ///
    /// # Arguments
    ///
    /// - `trace`: A reference to a `TransactionTrace` to be evaluated against the filter.
    ///
    /// # Returns
    ///
    /// - `true` if the transaction trace matches the filter criteria; otherwise, `false`.
    ///
    /// # Behavior
    ///
    /// This function evaluates whether the `trace` matches based on its action type:
    /// - `Call`: Matches if either the `from` or `to` addresses in the call action match the
    ///   filter's address criteria.
    /// - `Create`: Matches if the `from` address in action matches, and the result's address (if
    ///   available) matches the filter's address criteria.
    /// - `Selfdestruct`: Matches if the `address` and `refund_address` matches the filter's address
    ///   criteria.
    /// - `Reward`: Matches if the `author` address matches the filter's `to_addresses` criteria.
    ///
    /// The overall result depends on the filter mode:
    /// - `Union` mode: The trace matches if either the `from` or `to` address matches. If either of
    ///   the from or to address set is empty, the trace matches only if the other address matches,
    ///   and if both are empty, the filter matches all traces.
    /// - `Intersection` mode: The trace matches only if both the `from` and `to` addresses match.
    pub fn matches(&self, trace: &TransactionTrace) -> bool {
        let (from_matches, to_matches) = match trace.action {
            Action::Call(CallAction { from, to, .. }) => {
                (self.from_addresses.matches(&from), self.to_addresses.matches(&to))
            }
            Action::Create(CreateAction { from, .. }) => (
                self.from_addresses.matches(&from),
                match trace.result {
                    Some(TraceOutput::Create(CreateOutput { address: to, .. })) => {
                        self.to_addresses.matches(&to)
                    }
                    _ => self.to_addresses.is_empty(),
                },
            ),
            Action::Selfdestruct(SelfdestructAction { address, refund_address, .. }) => {
                (self.from_addresses.matches(&address), self.to_addresses.matches(&refund_address))
            }
            Action::Reward(RewardAction { author, .. }) => {
                (self.from_addresses.is_empty(), self.to_addresses.matches(&author))
            }
        };

        match self.mode {
            TraceFilterMode::Union => {
                if self.from_addresses.is_empty() {
                    to_matches
                } else if self.to_addresses.is_empty() {
                    from_matches
                } else {
                    from_matches || to_matches
                }
            }
            TraceFilterMode::Intersection => from_matches && to_matches,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{Bytes, U256};
    use serde_json::json;
    use similar_asserts::assert_eq;

    #[test]
    fn test_parse_filter() {
        let s = r#"{"fromBlock":  "0x3","toBlock":  "0x5"}"#;
        let filter: TraceFilter = serde_json::from_str(s).unwrap();
        assert_eq!(filter.from_block, Some(3));
        assert_eq!(filter.to_block, Some(5));
    }

    #[test]
    fn test_filter_matcher_addresses_unspecified() {
        let filter_json = json!({ "fromBlock": "0x3", "toBlock": "0x5" });
        let matcher = serde_json::from_value::<TraceFilter>(filter_json).unwrap().matcher();
        let s = r#"{
            "action": {
                "from": "0x66e29f0b6b1b07071f2fde4345d512386cb66f5f",
                "callType": "call",
                "gas": "0x10bfc",
                "input": "0x",
                "to": "0x160f5f00288e9e1cc8655b327e081566e580a71d",
                "value": "0x244b"
            },
            "error": "Reverted",
            "result": {
                "gasUsed": "0x9daf",
                "output": "0x"
            },
            "subtraces": 3,
            "traceAddress": [],
            "type": "call"
        }"#;
        let trace = serde_json::from_str::<TransactionTrace>(s).unwrap();

        assert!(matcher.matches(&trace));
    }

    #[test]
    fn test_filter_matcher() {
        let addr0 = "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045".parse().unwrap();
        let addr1 = "0x160f5f00288e9e1cc8655b327e081566e580a71d".parse().unwrap();
        let addr2 = "0x160f5f00288e9e1cc8655b327e081566e580a71f".parse().unwrap();

        let m0 = TraceFilterMatcher {
            mode: TraceFilterMode::Union,
            from_addresses: Default::default(),
            to_addresses: Default::default(),
        };

        let m1 = TraceFilterMatcher {
            mode: TraceFilterMode::Union,
            from_addresses: AddressFilter::from(vec![addr0]),
            to_addresses: Default::default(),
        };

        let m2 = TraceFilterMatcher {
            mode: TraceFilterMode::Union,
            from_addresses: AddressFilter::from(vec![]),
            to_addresses: AddressFilter::from(vec![addr1]),
        };

        let m3 = TraceFilterMatcher {
            mode: TraceFilterMode::Union,
            from_addresses: AddressFilter::from(vec![addr0]),
            to_addresses: AddressFilter::from(vec![addr1]),
        };

        let m4 = TraceFilterMatcher {
            mode: TraceFilterMode::Intersection,
            from_addresses: Default::default(),
            to_addresses: Default::default(),
        };

        let m5 = TraceFilterMatcher {
            mode: TraceFilterMode::Intersection,
            from_addresses: AddressFilter::from(vec![addr0]),
            to_addresses: Default::default(),
        };

        let m6 = TraceFilterMatcher {
            mode: TraceFilterMode::Intersection,
            from_addresses: Default::default(),
            to_addresses: AddressFilter::from(vec![addr1]),
        };

        let m7 = TraceFilterMatcher {
            mode: TraceFilterMode::Intersection,
            from_addresses: AddressFilter::from(vec![addr0]),
            to_addresses: AddressFilter::from(vec![addr1]),
        };

        // normal call 0
        let trace = TransactionTrace {
            action: Action::Call(CallAction { from: addr0, to: addr1, ..Default::default() }),
            ..Default::default()
        };
        assert!(m0.matches(&trace));
        assert!(m1.matches(&trace));
        assert!(m2.matches(&trace));
        assert!(m3.matches(&trace));
        assert!(m4.matches(&trace));
        assert!(m5.matches(&trace));
        assert!(m6.matches(&trace));
        assert!(m7.matches(&trace));

        // normal call 1
        let trace = TransactionTrace {
            action: Action::Call(CallAction { from: addr0, to: addr2, ..Default::default() }),
            ..Default::default()
        };
        assert!(m0.matches(&trace));
        assert!(m1.matches(&trace));
        assert!(!m2.matches(&trace));
        assert!(m3.matches(&trace));
        assert!(m4.matches(&trace));
        assert!(m5.matches(&trace));
        assert!(!m6.matches(&trace));
        assert!(!m7.matches(&trace));

        // create success
        let trace = TransactionTrace {
            action: Action::Create(CreateAction {
                from: addr0,
                gas: 10240,
                init: Bytes::new(),
                value: U256::from(0),
                ..Default::default()
            }),
            result: Some(TraceOutput::Create(CreateOutput {
                address: addr1,
                code: Bytes::new(),
                gas_used: 1025,
            })),
            ..Default::default()
        };
        assert!(m0.matches(&trace));
        assert!(m1.matches(&trace));
        assert!(m2.matches(&trace));
        assert!(m3.matches(&trace));
        assert!(m4.matches(&trace));
        assert!(m5.matches(&trace));
        assert!(m6.matches(&trace));
        assert!(m7.matches(&trace));

        // create failure
        let trace = TransactionTrace {
            action: Action::Create(CreateAction {
                from: addr0,
                gas: 100,
                init: Bytes::new(),
                value: U256::from(0),
                ..Default::default()
            }),
            error: Some("out of gas".into()),
            ..Default::default()
        };
        assert!(m0.matches(&trace));
        assert!(m1.matches(&trace));
        assert!(!m2.matches(&trace));
        assert!(m3.matches(&trace));
        assert!(m4.matches(&trace));
        assert!(m5.matches(&trace));
        assert!(!m6.matches(&trace));
        assert!(!m7.matches(&trace));

        // selfdestruct
        let trace = TransactionTrace {
            action: Action::Selfdestruct(SelfdestructAction {
                address: addr0,
                refund_address: addr1,
                balance: U256::from(0),
            }),
            ..Default::default()
        };
        assert!(m0.matches(&trace));
        assert!(m1.matches(&trace));
        assert!(m2.matches(&trace));
        assert!(m3.matches(&trace));
        assert!(m4.matches(&trace));
        assert!(m5.matches(&trace));
        assert!(m6.matches(&trace));
        assert!(m7.matches(&trace));

        // reward
        let trace = TransactionTrace {
            action: Action::Reward(RewardAction {
                author: addr0,
                reward_type: crate::parity::RewardType::Block,
                value: U256::from(0),
            }),
            ..Default::default()
        };
        assert!(m0.matches(&trace));
        assert!(!m1.matches(&trace));
        assert!(!m2.matches(&trace));
        assert!(!m3.matches(&trace));
        assert!(m4.matches(&trace));
        assert!(!m5.matches(&trace));
        assert!(!m6.matches(&trace));
        assert!(!m7.matches(&trace));
    }
}
