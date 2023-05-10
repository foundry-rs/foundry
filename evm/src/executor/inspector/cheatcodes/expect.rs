use super::{bail, ensure, err, Cheatcodes, Result};
use crate::{
    abi::HEVMCalls,
    error::{ERROR_PREFIX, REVERT_PREFIX},
    executor::backend::DatabaseExt,
    utils::h160_to_b160,
};
use ethers::{
    abi::{AbiDecode, RawLog},
    types::{Address, Bytes, H160, U256},
};
use revm::{
    interpreter::{return_ok, InstructionResult},
    primitives::Bytecode,
    EVMData,
};
use std::cmp::Ordering;
use tracing::{instrument, trace};

/// For some cheatcodes we may internally change the status of the call, i.e. in `expectRevert`.
/// Solidity will see a successful call and attempt to decode the return data. Therefore, we need
/// to populate the return with dummy bytes so the decode doesn't fail.
///
/// 512 bytes was arbitrarily chosen because it is long enough for return values up to 16 words in
/// size.
static DUMMY_CALL_OUTPUT: [u8; 512] = [0u8; 512];

/// Same reasoning as [DUMMY_CALL_OUTPUT], but for creates.
static DUMMY_CREATE_ADDRESS: Address =
    H160([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);

#[derive(Clone, Debug, Default)]
pub struct ExpectedRevert {
    /// The expected data returned by the revert, None being any
    pub reason: Option<Bytes>,
    /// The depth at which the revert is expected
    pub depth: u64,
}

fn expect_revert(state: &mut Cheatcodes, reason: Option<Bytes>, depth: u64) -> Result {
    ensure!(
        state.expected_revert.is_none(),
        "You must call another function prior to expecting a second revert."
    );
    state.expected_revert = Some(ExpectedRevert { reason, depth });
    Ok(Bytes::new())
}

#[instrument(skip_all, fields(expected_revert, status, retdata = hex::encode(&retdata)))]
pub fn handle_expect_revert(
    is_create: bool,
    expected_revert: Option<&Bytes>,
    status: InstructionResult,
    retdata: Bytes,
) -> Result<(Option<Address>, Bytes)> {
    trace!("handle expect revert");

    ensure!(!matches!(status, return_ok!()), "Call did not revert as expected");

    macro_rules! success_return {
        () => {
            Ok(if is_create {
                (Some(DUMMY_CREATE_ADDRESS), Bytes::new())
            } else {
                trace!("successfully handled expected revert");
                (None, DUMMY_CALL_OUTPUT.to_vec().into())
            })
        };
    }

    // If None, accept any revert
    let expected_revert = match expected_revert {
        Some(x) => x,
        None => return success_return!(),
    };

    if !expected_revert.is_empty() && retdata.is_empty() {
        bail!("Call reverted as expected, but without data");
    }

    let mut actual_revert = retdata;
    if actual_revert.len() >= 4 &&
        matches!(actual_revert[..4].try_into(), Ok(ERROR_PREFIX | REVERT_PREFIX))
    {
        if let Ok(bytes) = Bytes::decode(&actual_revert[4..]) {
            actual_revert = bytes;
        }
    }

    if actual_revert == *expected_revert {
        success_return!()
    } else {
        let stringify = |data: &[u8]| {
            String::decode(data)
                .ok()
                .or_else(|| std::str::from_utf8(data).ok().map(ToOwned::to_owned))
                .unwrap_or_else(|| format!("0x{}", hex::encode(data)))
        };
        Err(err!(
            "Error != expected error: {} != {}",
            stringify(&actual_revert),
            stringify(expected_revert),
        ))
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
    /// The expected value sent in the call
    pub value: Option<U256>,
    /// The expected gas supplied to the call
    pub gas: Option<u64>,
    /// The expected *minimum* gas supplied to the call
    pub min_gas: Option<u64>,
    /// The number of times the call is expected to be made.
    /// If the type of call is `NonCount`, this is the lower bound for the number of calls
    /// that must be seen.
    /// If the type of call is `Count`, this is the exact number of calls that must be seen.
    pub count: u64,
    /// The type of call
    pub call_type: ExpectedCallType,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ExpectedCallType {
    #[default]
    Count,
    NonCount,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MockCallDataContext {
    /// The partial calldata to match for mock
    pub calldata: Bytes,
    /// The value to match for mock
    pub value: Option<U256>,
}

#[derive(Clone, Debug)]
pub struct MockCallReturnData {
    /// The return type for the mocked call
    pub ret_type: InstructionResult,
    /// Return data or error
    pub data: Bytes,
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

fn expect_safe_memory(state: &mut Cheatcodes, start: u64, end: u64, depth: u64) -> Result {
    ensure!(start < end, "Invalid memory range: [{start}:{end}]");
    let offsets = state.allowed_mem_writes.entry(depth).or_insert_with(|| vec![0..0x60]);
    offsets.push(start..end);
    Ok(Bytes::new())
}

#[allow(clippy::too_many_arguments)]
fn expect_call(
    state: &mut Cheatcodes,
    target: H160,
    calldata: Vec<u8>,
    value: Option<U256>,
    gas: Option<u64>,
    min_gas: Option<u64>,
    count: u64,
    call_type: ExpectedCallType,
) -> Result {
    match call_type {
        ExpectedCallType::Count => {
            // Get the expected calls for this target.
            let expecteds = state.expected_calls.entry(target).or_default();
            // In this case, as we're using counted expectCalls, we should not be able to set them
            // more than once.
            ensure!(
                !expecteds.contains_key(&calldata),
                "Counted expected calls can only bet set once."
            );
            expecteds
                .insert(calldata, (ExpectedCallData { value, gas, min_gas, count, call_type }, 0));
            Ok(Bytes::new())
        }
        ExpectedCallType::NonCount => {
            let expecteds = state.expected_calls.entry(target).or_default();
            // Check if the expected calldata exists.
            // If it does, increment the count by one as we expect to see it one more time.
            if let Some(expected) = expecteds.get_mut(&calldata) {
                // Ensure we're not overwriting a counted expectCall.
                ensure!(
                    expected.0.call_type == ExpectedCallType::NonCount,
                    "Cannot overwrite a counted expectCall with a non-counted expectCall."
                );
                expected.0.count += 1;
            } else {
                // If it does not exist, then create it.
                expecteds.insert(
                    calldata,
                    (ExpectedCallData { value, gas, min_gas, count, call_type }, 0),
                );
            }
            Ok(Bytes::new())
        }
    }
}

pub fn apply<DB: DatabaseExt>(
    state: &mut Cheatcodes,
    data: &mut EVMData<'_, DB>,
    call: &HEVMCalls,
) -> Option<Result> {
    let result = match call {
        HEVMCalls::ExpectRevert0(_) => expect_revert(state, None, data.journaled_state.depth()),
        HEVMCalls::ExpectRevert1(inner) => {
            expect_revert(state, Some(inner.0.clone()), data.journaled_state.depth())
        }
        HEVMCalls::ExpectRevert2(inner) => {
            expect_revert(state, Some(inner.0.into()), data.journaled_state.depth())
        }
        HEVMCalls::ExpectEmit0(_) => {
            state.expected_emits.push(ExpectedEmit {
                depth: data.journaled_state.depth() - 1,
                checks: [true, true, true, true],
                ..Default::default()
            });
            Ok(Bytes::new())
        }
        HEVMCalls::ExpectEmit1(inner) => {
            state.expected_emits.push(ExpectedEmit {
                depth: data.journaled_state.depth() - 1,
                checks: [true, true, true, true],
                address: Some(inner.0),
                ..Default::default()
            });
            Ok(Bytes::new())
        }
        HEVMCalls::ExpectEmit2(inner) => {
            state.expected_emits.push(ExpectedEmit {
                depth: data.journaled_state.depth() - 1,
                checks: [inner.0, inner.1, inner.2, inner.3],
                ..Default::default()
            });
            Ok(Bytes::new())
        }
        HEVMCalls::ExpectEmit3(inner) => {
            state.expected_emits.push(ExpectedEmit {
                depth: data.journaled_state.depth() - 1,
                checks: [inner.0, inner.1, inner.2, inner.3],
                address: Some(inner.4),
                ..Default::default()
            });
            Ok(Bytes::new())
        }
        HEVMCalls::ExpectCall0(inner) => expect_call(
            state,
            inner.0,
            inner.1.to_vec().into(),
            None,
            None,
            None,
            1,
            ExpectedCallType::NonCount,
        ),
        HEVMCalls::ExpectCall1(inner) => expect_call(
            state,
            inner.0,
            inner.1.to_vec().into(),
            None,
            None,
            None,
            inner.2,
            ExpectedCallType::Count,
        ),
        HEVMCalls::ExpectCall2(inner) => expect_call(
            state,
            inner.0,
            inner.2.to_vec().into(),
            Some(inner.1),
            None,
            None,
            1,
            ExpectedCallType::NonCount,
        ),
        HEVMCalls::ExpectCall3(inner) => expect_call(
            state,
            inner.0,
            inner.2.to_vec().into(),
            Some(inner.1),
            None,
            None,
            inner.3,
            ExpectedCallType::Count,
        ),
        HEVMCalls::ExpectCall4(inner) => {
            let value = inner.1;
            // If the value of the transaction is non-zero, the EVM adds a call stipend of 2300 gas
            // to ensure that the basic fallback function can be called.
            let positive_value_cost_stipend = if value > U256::zero() { 2300 } else { 0 };

            expect_call(
                state,
                inner.0,
                inner.3.to_vec().into(),
                Some(value),
                Some(inner.2 + positive_value_cost_stipend),
                None,
                1,
                ExpectedCallType::NonCount,
            )
        }
        HEVMCalls::ExpectCall5(inner) => {
            let value = inner.1;
            // If the value of the transaction is non-zero, the EVM adds a call stipend of 2300 gas
            // to ensure that the basic fallback function can be called.
            let positive_value_cost_stipend = if value > U256::zero() { 2300 } else { 0 };

            expect_call(
                state,
                inner.0,
                inner.3.to_vec().into(),
                Some(value),
                Some(inner.2 + positive_value_cost_stipend),
                None,
                inner.4,
                ExpectedCallType::Count,
            )
        }
        HEVMCalls::ExpectCallMinGas0(inner) => {
            let value = inner.1;
            // If the value of the transaction is non-zero, the EVM adds a call stipend of 2300 gas
            // to ensure that the basic fallback function can be called.
            let positive_value_cost_stipend = if value > U256::zero() { 2300 } else { 0 };

            expect_call(
                state,
                inner.0,
                inner.3.to_vec().into(),
                Some(value),
                None,
                Some(inner.2 + positive_value_cost_stipend),
                1,
                ExpectedCallType::NonCount,
            )
        }
        HEVMCalls::ExpectCallMinGas1(inner) => {
            let value = inner.1;
            // If the value of the transaction is non-zero, the EVM adds a call stipend of 2300 gas
            // to ensure that the basic fallback function can be called.
            let positive_value_cost_stipend = if value > U256::zero() { 2300 } else { 0 };

            expect_call(
                state,
                inner.0,
                inner.3.to_vec().into(),
                Some(value),
                None,
                Some(inner.2 + positive_value_cost_stipend),
                inner.4,
                ExpectedCallType::Count,
            )
        }
        HEVMCalls::MockCall0(inner) => {
            // TODO: Does this increase gas usage?
            if let Err(err) = data.journaled_state.load_account(h160_to_b160(inner.0), data.db) {
                return Some(Err(err.into()))
            }

            // Etches a single byte onto the account if it is empty to circumvent the `extcodesize`
            // check Solidity might perform.
            let empty_bytecode = data
                .journaled_state
                .account(h160_to_b160(inner.0))
                .info
                .code
                .as_ref()
                .map_or(true, Bytecode::is_empty);
            if empty_bytecode {
                let code = Bytecode::new_raw(bytes::Bytes::from_static(&[0u8])).to_checked();
                data.journaled_state.set_code(h160_to_b160(inner.0), code);
            }
            state.mocked_calls.entry(inner.0).or_default().insert(
                MockCallDataContext { calldata: inner.1.clone(), value: None },
                MockCallReturnData { data: inner.2.clone(), ret_type: InstructionResult::Return },
            );
            Ok(Bytes::new())
        }
        HEVMCalls::MockCall1(inner) => {
            state.mocked_calls.entry(inner.0).or_default().insert(
                MockCallDataContext { calldata: inner.2.to_vec().into(), value: Some(inner.1) },
                MockCallReturnData {
                    data: inner.3.to_vec().into(),
                    ret_type: InstructionResult::Return,
                },
            );
            Ok(Bytes::new())
        }
        HEVMCalls::MockCallRevert0(inner) => {
            state.mocked_calls.entry(inner.0).or_default().insert(
                MockCallDataContext { calldata: inner.1.to_vec().into(), value: None },
                MockCallReturnData {
                    data: inner.2.to_vec().into(),
                    ret_type: InstructionResult::Revert,
                },
            );
            Ok(Bytes::new())
        }
        HEVMCalls::MockCallRevert1(inner) => {
            state.mocked_calls.entry(inner.0).or_default().insert(
                MockCallDataContext { calldata: inner.2.to_vec().into(), value: Some(inner.1) },
                MockCallReturnData {
                    data: inner.3.to_vec().into(),
                    ret_type: InstructionResult::Revert,
                },
            );
            Ok(Bytes::new())
        }
        HEVMCalls::ClearMockedCalls(_) => {
            state.mocked_calls = Default::default();
            Ok(Bytes::new())
        }
        HEVMCalls::ExpectSafeMemory(inner) => {
            expect_safe_memory(state, inner.0, inner.1, data.journaled_state.depth())
        }
        HEVMCalls::ExpectSafeMemoryCall(inner) => {
            expect_safe_memory(state, inner.0, inner.1, data.journaled_state.depth() + 1)
        }
        _ => return None,
    };
    Some(result)
}
