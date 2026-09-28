//! Expected-call registration shared by EVM inspectors.

use crate::Result;
use alloy_primitives::{
    Address, Bytes, U256, hex,
    map::{HashMap, hash_map::Entry},
};

/// Calls expected per target, calldata prefix, and optional call kind.
pub type ExpectedCallTracker =
    HashMap<Address, HashMap<(Bytes, Option<ExpectedCallKind>), (ExpectedCallData, u64)>>;

/// Call kinds that an expectation can restrict.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ExpectedCallKind {
    Call,
    CallCode,
    DelegateCall,
    StaticCall,
}

/// Required properties and number of calls observed.
#[derive(Clone, Debug)]
pub struct ExpectedCallData {
    /// Expected transferred value, if constrained.
    pub value: Option<U256>,
    /// Exact gas supplied, if constrained.
    pub gas: Option<u64>,
    /// Minimum gas supplied, if constrained.
    pub min_gas: Option<u64>,
    /// Exact count or lower bound, according to `call_type`.
    pub count: u64,
    /// How to compare the observed count.
    pub call_type: ExpectedCallType,
}

/// Whether the expected count is exact or a lower bound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExpectedCallType {
    NonCount,
    Count,
}

/// Registers a call expectation with the same count and stipend rules for every engine.
#[expect(clippy::too_many_arguments)]
pub fn expect_call(
    tracker: &mut ExpectedCallTracker,
    target: Address,
    calldata: Bytes,
    value: Option<U256>,
    mut gas: Option<u64>,
    mut min_gas: Option<u64>,
    scheme: Option<ExpectedCallKind>,
    count: u64,
    call_type: ExpectedCallType,
) -> Result<()> {
    if value.is_some_and(|value| value > U256::ZERO) {
        if let Some(gas) = &mut gas {
            *gas += 2300;
        }
        if let Some(min_gas) = &mut min_gas {
            *min_gas += 2300;
        }
    }

    let expected = tracker.entry(target).or_default();
    let key = (calldata, scheme);
    match call_type {
        ExpectedCallType::Count => {
            ensure!(!expected.contains_key(&key), "counted expected calls can only bet set once");
            expected.insert(key, (ExpectedCallData { value, gas, min_gas, count, call_type }, 0));
        }
        ExpectedCallType::NonCount => match expected.entry(key) {
            Entry::Occupied(mut entry) => {
                let (expected, _) = entry.get_mut();
                ensure!(
                    expected.call_type == ExpectedCallType::NonCount,
                    "cannot overwrite a counted expectCall with a non-counted expectCall"
                );
                expected.count += 1;
            }
            Entry::Vacant(entry) => {
                entry.insert((ExpectedCallData { value, gas, min_gas, count, call_type }, 0));
            }
        },
    }
    Ok(())
}

/// Counts matching calls, including every matching calldata prefix.
pub fn observe_call(
    tracker: &mut ExpectedCallTracker,
    target: Address,
    input: &[u8],
    value: Option<U256>,
    gas: u64,
    kind: ExpectedCallKind,
) {
    if let Some(expected) = tracker.get_mut(&target) {
        for ((calldata, expected_kind), (requirements, seen)) in expected {
            if input.starts_with(calldata)
                && requirements.value.is_none_or(|expected| value == Some(expected))
                && requirements.gas.is_none_or(|expected| expected == gas)
                && requirements.min_gas.is_none_or(|minimum| minimum <= gas)
                && expected_kind.is_none_or(|expected| expected == kind)
            {
                *seen += 1;
            }
        }
    }
}

/// Returns the first unsatisfied expectation after a successful top-level call.
pub fn first_unmet_call(tracker: &ExpectedCallTracker) -> Option<String> {
    for (address, calls) in tracker {
        for ((calldata, kind), (expected, seen)) in calls {
            let failed = match expected.call_type {
                ExpectedCallType::Count => *seen != expected.count,
                ExpectedCallType::NonCount => *seen < expected.count,
            };
            if failed {
                let mut parts = vec![format!("data {}", hex::encode_prefixed(calldata))];
                if let Some(value) = expected.value {
                    parts.push(format!("value {value}"));
                }
                if let Some(gas) = expected.gas {
                    parts.push(format!("gas {gas}"));
                }
                if let Some(gas) = expected.min_gas {
                    parts.push(format!("minimum gas {gas}"));
                }
                if let Some(kind) = kind {
                    parts.push(format!("call type {kind:?}"));
                }
                let plural = if expected.count == 1 { "" } else { "s" };
                let seen_plural = if *seen == 1 { "" } else { "s" };
                return Some(format!(
                    "expected call to {address} with {} to be called {} time{plural}, but was called {seen} time{seen_plural}",
                    parts.join(", "),
                    expected.count,
                ));
            }
        }
    }
    None
}
