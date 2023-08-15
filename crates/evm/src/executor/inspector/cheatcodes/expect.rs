use super::{bail, ensure, fmt_err, Cheatcodes, Result};
use crate::{abi::HEVMCalls, executor::backend::DatabaseExt, utils::h160_to_b160};
use ethers::{
    abi::{AbiDecode, RawLog},
    contract::Lazy,
    types::{Address, Bytes, H160, U256},
};
use foundry_utils::error::{ERROR_PREFIX, REVERT_PREFIX};
use revm::{
    interpreter::{return_ok, InstructionResult},
    primitives::Bytecode,
    EVMData,
};
use std::cmp::Ordering;

/// For some cheatcodes we may internally change the status of the call, i.e. in `expectRevert`.
/// Solidity will see a successful call and attempt to decode the return data. Therefore, we need
/// to populate the return with dummy bytes so the decode doesn't fail.
///
/// 8912 bytes was arbitrarily chosen because it is long enough for return values up to 256 words in
/// size.
pub static DUMMY_CALL_OUTPUT: Lazy<Bytes> = Lazy::new(|| Bytes::from_static(&[0u8; 8192]));

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

#[derive(Clone, Debug, Default)]
pub struct ExpectedRevertWithAddress {
    /// The address from which the revert is expected to be thrown
    pub address: H160,

    /// The expected data returned by the revert, None being any
    pub reason: Option<Bytes>,

    /// Tracks if the revert has been matched during EVM execution
    pub found: bool,

    /// Stores the latest revert data thrown by `address` during EVM execution. If None, `address`
    /// did not revert
    pub captured_revert_data: Option<Bytes>,
}

impl ExpectedRevertWithAddress {
    pub fn compare_revert(&mut self, other: H160, data: Bytes) -> bool {
        if self.address != other {
            return false
        }

        if let Some(ref expected_data) = self.reason {
            let raw_error_data = get_raw_error(data);
            let check = *expected_data == raw_error_data;

            self.captured_revert_data = Some(raw_error_data);

            return check
        }

        true
    }
}

fn expect_revert(state: &mut Cheatcodes, reason: Option<Bytes>, depth: u64) -> Result {
    ensure!(
        state.expected_revert.is_none(),
        "You must call another function prior to expecting a second revert."
    );
    state.expected_revert = Some(ExpectedRevert { reason, depth });
    Ok(Bytes::new())
}

macro_rules! success_return {
    ($is_create:expr) => {
        Ok(if $is_create {
            (Some(DUMMY_CREATE_ADDRESS), Bytes::new())
        } else {
            trace!("successfully handled expected revert");
            (None, DUMMY_CALL_OUTPUT.to_vec().into())
        })
    };
}

fn stringify(data: &[u8]) -> String {
    String::decode(data)
        .ok()
        .or_else(|| std::str::from_utf8(data).ok().map(ToOwned::to_owned))
        .unwrap_or_else(|| format!("0x{}", hex::encode(data)))
}

/// Checks that the revert error data has the [ERROR_PREFIX] or [REVERT_PREFIX] prefixes and removes
/// them to get the raw error data that will be used to verify if an expected revert happened as
/// defined by the user.
fn get_raw_error(data: Bytes) -> Bytes {
    if data.len() >= 4 && matches!(data[..4].try_into(), Ok(ERROR_PREFIX | REVERT_PREFIX)) {
        if let Ok(bytes) = Bytes::decode(&data[4..]) {
            return bytes
        }
    }

    data
}

/// Verifies that an EVM execution completed with a revert and that the revert matches the user
/// provided data if any.
pub fn handle_expect_revert(
    is_create: bool,
    expected_revert: Option<&Bytes>,
    status: InstructionResult,
    retdata: Bytes,
) -> Result<(Option<Address>, Bytes)> {
    trace!("handle expect revert");

    ensure!(!matches!(status, return_ok!()), "Call did not revert as expected");

    // If None, accept any revert
    let expected_revert = match expected_revert {
        Some(x) => x,
        None => return success_return!(is_create),
    };

    if !expected_revert.is_empty() && retdata.is_empty() {
        bail!("Call reverted as expected, but without data");
    }

    let actual_revert = get_raw_error(retdata);
    if actual_revert == *expected_revert {
        success_return!(is_create)
    } else {
        Err(fmt_err!(
            "Error != expected error: {} != {}",
            stringify(&actual_revert),
            stringify(expected_revert),
        ))
    }
}

pub fn build_expect_revert_with_address_failure_message(
    expected_revert: ExpectedRevertWithAddress,
) -> String {
    match expected_revert.captured_revert_data {
        Some(captured_revert_data) => {
            if captured_revert_data.is_empty() {
                format!(
                    "The expected revert address {:#?} reverted as expected, but without data",
                    expected_revert.address
                )
            } else if let Some(expected_revert_reason) = expected_revert.reason {
                format!(
                "The expected revert address {:#?} did not revert with the expected revert data. Expected: {} but found: {}",
                expected_revert.address,
                stringify(&expected_revert_reason),
                stringify(&captured_revert_data),
            )
            }
            // Technically this scenario shouldn't happen because if the expected revert data is
            // None and the address of the revert matches then the test should pass.
            else {
                format!(
                "The expected revert address {:#?} did not revert with the expected revert data.",
                expected_revert.address
            )
            }
        }
        None => {
            format!("The expected revert address {:#?} did not revert", expected_revert.address,)
        }
    }
}

/// Verifies that a revert matches one of the reverts associated to an address as expected by the
/// user and that it happened in the expected order
pub fn handle_expected_reverts_with_address(
    state: &mut Cheatcodes,
    current_contract: H160,
    current_revert_data: Bytes,
) {
    // If this is empty we don't need to match any revert
    if state.expected_reverts_with_address.is_empty() {
        return
    }

    // Take the first expected revert and...
    if let Some(mut first) = state.expected_reverts_with_address.pop_front() {
        // ... verify if it has been alredy matched.
        if !first.found {
            // if it matches with the current revert mark found as true and push the current
            // expected revert to the end of the queue to match any other pending expected
            // revert.
            if first.compare_revert(current_contract, current_revert_data) {
                first.found = true;
                state.expected_reverts_with_address.push_back(first);
            }
            // If the current expected revert does not match we push it to the front of the queue to
            // try to match it again with the next revert.
            else {
                state.expected_reverts_with_address.push_front(first);
            }
        }
        // If the first expected revert has already been matched it means that all the expected
        // reverts have been matched and we can safely clear the queque.
        else {
            state.matched_all_expected_reverts_with_address = true;
            state.expected_reverts_with_address.clear();
        }
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
    // Fill or check the expected emits.
    // We expect for emit checks to be filled as they're declared (from oldest to newest),
    // so we fill them and push them to the back of the queue.
    // If the user has properly filled all the emits, they'll end up in their original order.
    // If not, the queue will not be in the order the events will be intended to be filled,
    // and we'll be able to later detect this and bail.

    // First, we can return early if all events have been matched.
    // This allows a contract to arbitrarily emit more events than expected (additive behavior),
    // as long as all the previous events were matched in the order they were expected to be.
    if state.expected_emits.iter().all(|expected| expected.found) {
        return
    }

    // if there's anything to fill, we need to pop back.
    let event_to_fill_or_check =
        if state.expected_emits.iter().any(|expected| expected.log.is_none()) {
            state.expected_emits.pop_back()
        // Else, if there are any events that are unmatched, we try to match to match them
        // in the order declared, so we start popping from the front (like a queue).
        } else {
            state.expected_emits.pop_front()
        };

    let mut event_to_fill_or_check =
        event_to_fill_or_check.expect("We should have an emit to fill or check. This is a bug");

    match event_to_fill_or_check.log {
        Some(ref expected) => {
            let expected_topic_0 = expected.topics.get(0);
            let log_topic_0 = log.topics.get(0);

            // same topic0 and equal number of topics should be verified further, others are a no
            // match
            if expected_topic_0
                .zip(log_topic_0)
                .map_or(false, |(a, b)| a == b && expected.topics.len() == log.topics.len())
            {
                // Match topics
                event_to_fill_or_check.found = log
                    .topics
                    .iter()
                    .skip(1)
                    .enumerate()
                    .filter(|(i, _)| event_to_fill_or_check.checks[*i])
                    .all(|(i, topic)| topic == &expected.topics[i + 1]);

                // Maybe match source address
                if let Some(addr) = event_to_fill_or_check.address {
                    event_to_fill_or_check.found &= addr == *address;
                }

                // Maybe match data
                if event_to_fill_or_check.checks[3] {
                    event_to_fill_or_check.found &= expected.data == log.data;
                }
            }

            // If we found the event, we can push it to the back of the queue
            // and begin expecting the next event.
            if event_to_fill_or_check.found {
                state.expected_emits.push_back(event_to_fill_or_check);
            } else {
                // We did not match this event, so we need to keep waiting for the right one to
                // appear.
                state.expected_emits.push_front(event_to_fill_or_check);
            }
        }
        // Fill the event.
        None => {
            event_to_fill_or_check.log = Some(log);
            state.expected_emits.push_back(event_to_fill_or_check);
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
    #[allow(clippy::single_range_in_vec_init)]
    let offsets = state.allowed_mem_writes.entry(depth).or_insert_with(|| vec![0..0x60]);
    offsets.push(start..end);
    Ok(Bytes::new())
}

/// Handles expected calls specified by the `vm.expectCall` cheatcode.
///
/// It can handle calls in two ways:
/// - If the cheatcode was used with a `count` argument, it will expect the call to be made exactly
///   `count` times.
/// e.g. `vm.expectCall(address(0xc4f3), abi.encodeWithSelector(0xd34db33f), 4)` will expect the
/// call to address(0xc4f3) with selector `0xd34db33f` to be made exactly 4 times. If the amount of
/// calls is less or more than 4, the test will fail. Note that the `count` argument cannot be
/// overwritten with another `vm.expectCall`. If this is attempted, `expectCall` will revert.
/// - If the cheatcode was used without a `count` argument, it will expect the call to be made at
///   least the amount of times the cheatcode
/// was called. This means that `vm.expectCall` without a count argument can be called many times,
/// but cannot be called with a `count` argument after it was called without one. If the latter
/// happens, `expectCall` will revert. e.g `vm.expectCall(address(0xc4f3),
/// abi.encodeWithSelector(0xd34db33f))` will expect the call to address(0xc4f3) and selector
/// `0xd34db33f` to be made at least once. If the amount of calls is 0, the test will fail. If the
/// call is made more than once, the test will pass.
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

#[instrument(level = "error", name = "expect", target = "evm::cheatcodes", skip_all)]
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
        HEVMCalls::ExpectRevert3(inner) => {
            state
                .expected_reverts_with_address
                .push_back(ExpectedRevertWithAddress { address: inner.0, ..Default::default() });
            Ok(Bytes::new())
        }
        HEVMCalls::ExpectRevert4(inner) => {
            state.expected_reverts_with_address.push_back(ExpectedRevertWithAddress {
                address: inner.1,
                reason: Some(inner.0.clone()),
                ..Default::default()
            });
            Ok(Bytes::new())
        }
        HEVMCalls::ExpectRevert5(inner) => {
            state.expected_reverts_with_address.push_back(ExpectedRevertWithAddress {
                address: inner.1,
                reason: Some(inner.0.into()),
                ..Default::default()
            });

            Ok(Bytes::new())
        }
        HEVMCalls::ExpectEmit0(_) => {
            state.expected_emits.push_back(ExpectedEmit {
                depth: data.journaled_state.depth(),
                checks: [true, true, true, true],
                ..Default::default()
            });
            Ok(Bytes::new())
        }
        HEVMCalls::ExpectEmit1(inner) => {
            state.expected_emits.push_back(ExpectedEmit {
                depth: data.journaled_state.depth(),
                checks: [true, true, true, true],
                address: Some(inner.0),
                ..Default::default()
            });
            Ok(Bytes::new())
        }
        HEVMCalls::ExpectEmit2(inner) => {
            state.expected_emits.push_back(ExpectedEmit {
                depth: data.journaled_state.depth(),
                checks: [inner.0, inner.1, inner.2, inner.3],
                ..Default::default()
            });
            Ok(Bytes::new())
        }
        HEVMCalls::ExpectEmit3(inner) => {
            state.expected_emits.push_back(ExpectedEmit {
                depth: data.journaled_state.depth(),
                checks: [inner.0, inner.1, inner.2, inner.3],
                address: Some(inner.4),
                ..Default::default()
            });
            Ok(Bytes::new())
        }
        HEVMCalls::ExpectCall0(inner) => expect_call(
            state,
            inner.0,
            inner.1.to_vec(),
            None,
            None,
            None,
            1,
            ExpectedCallType::NonCount,
        ),
        HEVMCalls::ExpectCall1(inner) => expect_call(
            state,
            inner.0,
            inner.1.to_vec(),
            None,
            None,
            None,
            inner.2,
            ExpectedCallType::Count,
        ),
        HEVMCalls::ExpectCall2(inner) => expect_call(
            state,
            inner.0,
            inner.2.to_vec(),
            Some(inner.1),
            None,
            None,
            1,
            ExpectedCallType::NonCount,
        ),
        HEVMCalls::ExpectCall3(inner) => expect_call(
            state,
            inner.0,
            inner.2.to_vec(),
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
                inner.3.to_vec(),
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
                inner.3.to_vec(),
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
                inner.3.to_vec(),
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
                inner.3.to_vec(),
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
            if let Err(err) = data.journaled_state.load_account(h160_to_b160(inner.0), data.db) {
                return Some(Err(err.into()))
            }

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
