use super::Cheatcodes;
use crate::{
    abi::HEVMCalls,
    executor::inspector::cheatcodes::util::{ERROR_PREFIX, REVERT_PREFIX},
};
use bytes::Bytes;
use ethers::{
    abi::{AbiDecode, AbiEncode, RawLog},
    types::{Address, H160, U256},
};
use revm::{return_ok, Database, EVMData, Return};
use std::cmp::Ordering;

/// For some cheatcodes we may internally change the status of the call, i.e. in `expectRevert`.
/// Solidity will see a successful call and attempt to decode the return data. Therefore, we need
/// to populate the return with dummy bytes so the decode doesn't fail.
///
/// 320 bytes was arbitrarily chosen because it is long enough for return values up to 10 words in
/// size.
static DUMMY_CALL_OUTPUT: [u8; 320] = [0u8; 320];

/// Same reasoning as [DUMMY_CALL_OUTPUT], but for creates.
static DUMMY_CREATE_ADDRESS: Address =
    H160([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);

#[derive(Clone, Debug, Default)]
pub struct ExpectedRevert {
    /// The expected data returned by the revert
    pub reason: Bytes,
    /// The depth at which the revert is expected
    pub depth: u64,
}

fn expect_revert(state: &mut Cheatcodes, reason: Bytes, depth: u64) -> Result<Bytes, Bytes> {
    if state.expected_revert.is_some() {
        Err("You must call another function prior to expecting a second revert."
            .to_string()
            .encode()
            .into())
    } else {
        state.expected_revert = Some(ExpectedRevert { reason, depth });
        Ok(Bytes::new())
    }
}

pub fn handle_expect_revert(
    is_create: bool,
    expected_revert: &Bytes,
    status: Return,
    retdata: Bytes,
) -> Result<(Option<Address>, Bytes), Bytes> {
    if matches!(status, return_ok!()) {
        return Err("Call did not revert as expected".to_string().encode().into())
    }

    if !expected_revert.is_empty() && retdata.is_empty() {
        return Err("Call reverted as expected, but without data".to_string().encode().into())
    }

    let string_data = match retdata {
        _ if retdata.len() >= REVERT_PREFIX.len() &&
            retdata[..REVERT_PREFIX.len()] == REVERT_PREFIX =>
        {
            Some(&retdata[4..])
        }
        _ if retdata.len() >= ERROR_PREFIX.len() &&
            &retdata[..ERROR_PREFIX.len()] == ERROR_PREFIX.as_slice() =>
        {
            Some(&retdata[ERROR_PREFIX.len()..])
        }
        _ => None,
    };

    let stringify = |data: &[u8]| {
        String::decode(data)
            .ok()
            .or_else(|| String::from_utf8(data.to_vec()).ok())
            .unwrap_or_else(|| format!("0x{}", hex::encode(data)))
    };

    let (err, actual_revert): (_, Bytes) = if let Some(data) = string_data {
        // It's a revert string, so we do some conversion to perform the check
        let decoded_data = ethers::prelude::Bytes::decode(data)
            .expect("String error code, but data can't be decoded as bytes");

        (
            format!(
                "Error != expected error: '{}' != '{}'",
                stringify(&decoded_data),
                stringify(expected_revert),
            )
            .encode()
            .into(),
            decoded_data.0,
        )
    } else {
        (
            format!(
                "Error != expected error: {} != {}",
                stringify(&retdata),
                stringify(expected_revert),
            )
            .encode()
            .into(),
            retdata,
        )
    };

    if actual_revert == expected_revert {
        Ok(if is_create {
            (Some(DUMMY_CREATE_ADDRESS), Bytes::new())
        } else {
            (None, DUMMY_CALL_OUTPUT.to_vec().into())
        })
    } else {
        Err(err)
    }
}

#[derive(Clone, Debug, Default)]
pub struct ExpectedEmit {
    /// The depth at which we expect this emit to have occurred
    pub depth: u64,
    /// The log we expect
    pub log: Option<RawLog>,
    /// The checks to perform:
    ///
    /// ┌───────┬───────┬───────┬────┐
    /// │topic 1│topic 2│topic 3│data│
    /// └───────┴───────┴───────┴────┘
    pub checks: [bool; 4],
    /// If present, check originating address against this
    pub address: Option<Address>,
    /// Whether the log was actually found in the subcalls
    pub found: bool,
}

pub fn handle_expect_emit(state: &mut Cheatcodes, log: RawLog, address: &Address) {
    // Fill or check the expected emits
    if let Some(next_expect_to_fill) =
        state.expected_emits.iter_mut().find(|expect| expect.log.is_none())
    {
        // We have unfilled expects, so we fill the first one
        next_expect_to_fill.log = Some(log);
    } else if let Some(next_expect) = state.expected_emits.iter_mut().find(|expect| !expect.found) {
        // We do not have unfilled expects, so we try to match this log with the first unfound
        // log that we expect
        let expected =
            next_expect.log.as_ref().expect("we should have a log to compare against here");

        let expected_topic_0 = expected.topics.get(0);
        let log_topic_0 = log.topics.get(0);

        // same topic0 and equal number of topics should be verified further, others are a no
        // match
        if expected_topic_0
            .zip(log_topic_0)
            .map_or(false, |(a, b)| a == b && expected.topics.len() == log.topics.len())
        {
            // Match topics
            next_expect.found = log
                .topics
                .iter()
                .skip(1)
                .enumerate()
                .filter(|(i, _)| next_expect.checks[*i])
                .all(|(i, topic)| topic == &expected.topics[i + 1]);

            // Maybe match source address
            if let Some(addr) = next_expect.address {
                next_expect.found &= addr == *address;
            }

            // Maybe match data
            if next_expect.checks[3] {
                next_expect.found &= expected.data == log.data;
            }
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ExpectedCallData {
    /// The expected calldata
    pub calldata: Bytes,
    /// The expected value sent in the call
    pub value: Option<U256>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MockCallDataContext {
    /// The partial calldata to match for mock
    pub calldata: Bytes,
    /// The value to match for mock
    pub value: Option<U256>,
}

impl Ord for MockCallDataContext {
    fn cmp(&self, other: &Self) -> Ordering {
        // Calldata matching is reversed to ensure that a tighter match is
        // returned if an exact match is not found. In case, there is
        // a partial match to calldata that is more specific than
        // a match to a msg.value, then the more specific calldata takes
        // precedence.
        self.calldata.cmp(&other.calldata).reverse().then(self.value.cmp(&other.value).reverse())
    }
}

impl PartialOrd for MockCallDataContext {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

pub fn apply<DB: Database>(
    state: &mut Cheatcodes,
    data: &mut EVMData<'_, DB>,
    call: &HEVMCalls,
) -> Option<Result<Bytes, Bytes>> {
    Some(match call {
        HEVMCalls::ExpectRevert0(_) => expect_revert(state, Bytes::new(), data.subroutine.depth()),
        HEVMCalls::ExpectRevert1(inner) => {
            expect_revert(state, inner.0.to_vec().into(), data.subroutine.depth())
        }
        HEVMCalls::ExpectRevert2(inner) => {
            expect_revert(state, inner.0.to_vec().into(), data.subroutine.depth())
        }
        HEVMCalls::ExpectEmit0(inner) => {
            state.expected_emits.push(ExpectedEmit {
                depth: data.subroutine.depth() - 1,
                checks: [inner.0, inner.1, inner.2, inner.3],
                ..Default::default()
            });
            Ok(Bytes::new())
        }
        HEVMCalls::ExpectEmit1(inner) => {
            state.expected_emits.push(ExpectedEmit {
                depth: data.subroutine.depth() - 1,
                checks: [inner.0, inner.1, inner.2, inner.3],
                address: Some(inner.4),
                ..Default::default()
            });
            Ok(Bytes::new())
        }
        HEVMCalls::ExpectCall0(inner) => {
            state
                .expected_calls
                .entry(inner.0)
                .or_default()
                .push(ExpectedCallData { calldata: inner.1.to_vec().into(), value: None });
            Ok(Bytes::new())
        }
        HEVMCalls::ExpectCall1(inner) => {
            state
                .expected_calls
                .entry(inner.0)
                .or_default()
                .push(ExpectedCallData { calldata: inner.2.to_vec().into(), value: Some(inner.1) });
            Ok(Bytes::new())
        }
        HEVMCalls::MockCall0(inner) => {
            state.mocked_calls.entry(inner.0).or_default().insert(
                MockCallDataContext { calldata: inner.1.to_vec().into(), value: None },
                inner.2.to_vec().into(),
            );
            Ok(Bytes::new())
        }
        HEVMCalls::MockCall1(inner) => {
            state.mocked_calls.entry(inner.0).or_default().insert(
                MockCallDataContext { calldata: inner.2.to_vec().into(), value: Some(inner.1) },
                inner.3.to_vec().into(),
            );
            Ok(Bytes::new())
        }
        HEVMCalls::ClearMockedCalls(_) => {
            state.mocked_calls = Default::default();
            Ok(Bytes::new())
        }
        _ => return None,
    })
}
