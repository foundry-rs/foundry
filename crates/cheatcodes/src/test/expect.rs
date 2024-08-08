use crate::{Cheatcode, Cheatcodes, CheatsCtxt, DatabaseExt, Result, Vm::*};
use alloy_primitives::{address, hex, Address, Bytes, LogData as RawLog, U256};
use alloy_sol_types::{SolError, SolValue};
use revm::interpreter::{return_ok, InstructionResult};
use spec::Vm;
use std::collections::{hash_map::Entry, HashMap};

/// For some cheatcodes we may internally change the status of the call, i.e. in `expectRevert`.
/// Solidity will see a successful call and attempt to decode the return data. Therefore, we need
/// to populate the return with dummy bytes so the decode doesn't fail.
///
/// 8192 bytes was arbitrarily chosen because it is long enough for return values up to 256 words in
/// size.
static DUMMY_CALL_OUTPUT: Bytes = Bytes::from_static(&[0u8; 8192]);

/// Same reasoning as [DUMMY_CALL_OUTPUT], but for creates.
const DUMMY_CREATE_ADDRESS: Address = address!("0000000000000000000000000000000000000001");

/// Tracks the expected calls per address.
///
/// For each address, we track the expected calls per call data. We track it in such manner
/// so that we don't mix together calldatas that only contain selectors and calldatas that contain
/// selector and arguments (partial and full matches).
///
/// This then allows us to customize the matching behavior for each call data on the
/// `ExpectedCallData` struct and track how many times we've actually seen the call on the second
/// element of the tuple.
pub type ExpectedCallTracker = HashMap<Address, HashMap<Bytes, (ExpectedCallData, u64)>>;

#[derive(Clone, Debug)]
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
    /// The type of expected call.
    pub call_type: ExpectedCallType,
}

/// The type of expected call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExpectedCallType {
    /// The call is expected to be made at least once.
    NonCount,
    /// The exact number of calls expected.
    Count,
}

/// The type of expected revert.
#[derive(Clone, Debug)]
pub enum ExpectedRevertKind {
    /// Expects revert from the next non-cheatcode call.
    Default,
    /// Expects revert from the next cheatcode call.
    ///
    /// The `pending_processing` flag is used to track whether we have exited
    /// `expectCheatcodeRevert` context or not.
    /// We have to track it to avoid expecting `expectCheatcodeRevert` call to revert itself.
    Cheatcode { pending_processing: bool },
}

#[derive(Clone, Debug)]
pub struct ExpectedRevert {
    /// The expected data returned by the revert, None being any
    pub reason: Option<Vec<u8>>,
    /// The depth at which the revert is expected
    pub depth: u64,
    /// The type of expected revert.
    pub kind: ExpectedRevertKind,
}

#[derive(Clone, Debug)]
pub struct ExpectedEmit {
    /// The depth at which we expect this emit to have occurred
    pub depth: u64,
    /// The log we expect
    pub log: Option<RawLog>,
    /// The checks to perform:
    /// ```text
    /// ┌───────┬───────┬───────┬───────┬────┐
    /// │topic 0│topic 1│topic 2│topic 3│data│
    /// └───────┴───────┴───────┴───────┴────┘
    /// ```
    pub checks: [bool; 5],
    /// If present, check originating address against this
    pub address: Option<Address>,
    /// If present, relax the requirement that topic 0 must be present. This allows anonymous
    /// events with no indexed topics to be matched.
    pub anonymous: bool,
    /// Whether the log was actually found in the subcalls
    pub found: bool,
}

impl Cheatcode for expectCall_0Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { callee, data } = self;
        expect_call(state, callee, data, None, None, None, 1, ExpectedCallType::NonCount)
    }
}

impl Cheatcode for expectCall_1Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { callee, data, count } = self;
        expect_call(state, callee, data, None, None, None, *count, ExpectedCallType::Count)
    }
}

impl Cheatcode for expectCall_2Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { callee, msgValue, data } = self;
        expect_call(state, callee, data, Some(msgValue), None, None, 1, ExpectedCallType::NonCount)
    }
}

impl Cheatcode for expectCall_3Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { callee, msgValue, data, count } = self;
        expect_call(
            state,
            callee,
            data,
            Some(msgValue),
            None,
            None,
            *count,
            ExpectedCallType::Count,
        )
    }
}

impl Cheatcode for expectCall_4Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { callee, msgValue, gas, data } = self;
        expect_call(
            state,
            callee,
            data,
            Some(msgValue),
            Some(*gas),
            None,
            1,
            ExpectedCallType::NonCount,
        )
    }
}

impl Cheatcode for expectCall_5Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { callee, msgValue, gas, data, count } = self;
        expect_call(
            state,
            callee,
            data,
            Some(msgValue),
            Some(*gas),
            None,
            *count,
            ExpectedCallType::Count,
        )
    }
}

impl Cheatcode for expectCallMinGas_0Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { callee, msgValue, minGas, data } = self;
        expect_call(
            state,
            callee,
            data,
            Some(msgValue),
            None,
            Some(*minGas),
            1,
            ExpectedCallType::NonCount,
        )
    }
}

impl Cheatcode for expectCallMinGas_1Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { callee, msgValue, minGas, data, count } = self;
        expect_call(
            state,
            callee,
            data,
            Some(msgValue),
            None,
            Some(*minGas),
            *count,
            ExpectedCallType::Count,
        )
    }
}

impl Cheatcode for expectEmit_0Call {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { checkTopic1, checkTopic2, checkTopic3, checkData } = *self;
        expect_emit(
            ccx.state,
            ccx.ecx.journaled_state.depth(),
            [true, checkTopic1, checkTopic2, checkTopic3, checkData],
            None,
            false,
        )
    }
}

impl Cheatcode for expectEmit_1Call {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { checkTopic1, checkTopic2, checkTopic3, checkData, emitter } = *self;
        expect_emit(
            ccx.state,
            ccx.ecx.journaled_state.depth(),
            [true, checkTopic1, checkTopic2, checkTopic3, checkData],
            Some(emitter),
            false,
        )
    }
}

impl Cheatcode for expectEmit_2Call {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self {} = self;
        expect_emit(ccx.state, ccx.ecx.journaled_state.depth(), [true; 5], None, false)
    }
}

impl Cheatcode for expectEmit_3Call {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { emitter } = *self;
        expect_emit(ccx.state, ccx.ecx.journaled_state.depth(), [true; 5], Some(emitter), false)
    }
}

impl Cheatcode for expectEmitAnonymous_0Call {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { checkTopic0, checkTopic1, checkTopic2, checkTopic3, checkData } = *self;
        expect_emit(
            ccx.state,
            ccx.ecx.journaled_state.depth(),
            [checkTopic0, checkTopic1, checkTopic2, checkTopic3, checkData],
            None,
            true,
        )
    }
}

impl Cheatcode for expectEmitAnonymous_1Call {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { checkTopic0, checkTopic1, checkTopic2, checkTopic3, checkData, emitter } = *self;
        expect_emit(
            ccx.state,
            ccx.ecx.journaled_state.depth(),
            [checkTopic0, checkTopic1, checkTopic2, checkTopic3, checkData],
            Some(emitter),
            true,
        )
    }
}

impl Cheatcode for expectEmitAnonymous_2Call {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self {} = self;
        expect_emit(ccx.state, ccx.ecx.journaled_state.depth(), [true; 5], None, true)
    }
}

impl Cheatcode for expectEmitAnonymous_3Call {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { emitter } = *self;
        expect_emit(ccx.state, ccx.ecx.journaled_state.depth(), [true; 5], Some(emitter), true)
    }
}

impl Cheatcode for expectRevert_0Call {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self {} = self;
        expect_revert(ccx.state, None, ccx.ecx.journaled_state.depth(), false)
    }
}

impl Cheatcode for expectRevert_1Call {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { revertData } = self;
        expect_revert(ccx.state, Some(revertData.as_ref()), ccx.ecx.journaled_state.depth(), false)
    }
}

impl Cheatcode for expectRevert_2Call {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { revertData } = self;
        expect_revert(ccx.state, Some(revertData), ccx.ecx.journaled_state.depth(), false)
    }
}

impl Cheatcode for _expectCheatcodeRevert_0Call {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        expect_revert(ccx.state, None, ccx.ecx.journaled_state.depth(), true)
    }
}

impl Cheatcode for _expectCheatcodeRevert_1Call {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { revertData } = self;
        expect_revert(ccx.state, Some(revertData.as_ref()), ccx.ecx.journaled_state.depth(), true)
    }
}

impl Cheatcode for _expectCheatcodeRevert_2Call {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { revertData } = self;
        expect_revert(ccx.state, Some(revertData), ccx.ecx.journaled_state.depth(), true)
    }
}

impl Cheatcode for expectSafeMemoryCall {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { min, max } = *self;
        expect_safe_memory(ccx.state, min, max, ccx.ecx.journaled_state.depth())
    }
}

impl Cheatcode for stopExpectSafeMemoryCall {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self {} = self;
        ccx.state.allowed_mem_writes.remove(&ccx.ecx.journaled_state.depth());
        Ok(Default::default())
    }
}

impl Cheatcode for expectSafeMemoryCallCall {
    fn apply_stateful<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { min, max } = *self;
        expect_safe_memory(ccx.state, min, max, ccx.ecx.journaled_state.depth() + 1)
    }
}

/// Handles expected calls specified by the `expectCall` cheatcodes.
///
/// It can handle calls in two ways:
/// - If the cheatcode was used with a `count` argument, it will expect the call to be made exactly
///   `count` times. e.g. `vm.expectCall(address(0xc4f3), abi.encodeWithSelector(0xd34db33f), 4)`
///   will expect the call to address(0xc4f3) with selector `0xd34db33f` to be made exactly 4 times.
///   If the amount of calls is less or more than 4, the test will fail. Note that the `count`
///   argument cannot be overwritten with another `vm.expectCall`. If this is attempted,
///   `expectCall` will revert.
/// - If the cheatcode was used without a `count` argument, it will expect the call to be made at
///   least the amount of times the cheatcode was called. This means that `vm.expectCall` without a
///   count argument can be called many times, but cannot be called with a `count` argument after it
///   was called without one. If the latter happens, `expectCall` will revert. e.g
///   `vm.expectCall(address(0xc4f3), abi.encodeWithSelector(0xd34db33f))` will expect the call to
///   address(0xc4f3) and selector `0xd34db33f` to be made at least once. If the amount of calls is
///   0, the test will fail. If the call is made more than once, the test will pass.
#[allow(clippy::too_many_arguments)] // It is what it is
fn expect_call(
    state: &mut Cheatcodes,
    target: &Address,
    calldata: &Bytes,
    value: Option<&U256>,
    mut gas: Option<u64>,
    mut min_gas: Option<u64>,
    count: u64,
    call_type: ExpectedCallType,
) -> Result {
    let expecteds = state.expected_calls.entry(*target).or_default();

    if let Some(val) = value {
        if *val > U256::ZERO {
            // If the value of the transaction is non-zero, the EVM adds a call stipend of 2300 gas
            // to ensure that the basic fallback function can be called.
            let positive_value_cost_stipend = 2300;
            if let Some(gas) = &mut gas {
                *gas += positive_value_cost_stipend;
            }
            if let Some(min_gas) = &mut min_gas {
                *min_gas += positive_value_cost_stipend;
            }
        }
    }

    match call_type {
        ExpectedCallType::Count => {
            // Get the expected calls for this target.
            // In this case, as we're using counted expectCalls, we should not be able to set them
            // more than once.
            ensure!(
                !expecteds.contains_key(calldata),
                "counted expected calls can only bet set once"
            );
            expecteds.insert(
                calldata.clone(),
                (ExpectedCallData { value: value.copied(), gas, min_gas, count, call_type }, 0),
            );
        }
        ExpectedCallType::NonCount => {
            // Check if the expected calldata exists.
            // If it does, increment the count by one as we expect to see it one more time.
            match expecteds.entry(calldata.clone()) {
                Entry::Occupied(mut entry) => {
                    let (expected, _) = entry.get_mut();
                    // Ensure we're not overwriting a counted expectCall.
                    ensure!(
                        expected.call_type == ExpectedCallType::NonCount,
                        "cannot overwrite a counted expectCall with a non-counted expectCall"
                    );
                    expected.count += 1;
                }
                // If it does not exist, then create it.
                Entry::Vacant(entry) => {
                    entry.insert((
                        ExpectedCallData { value: value.copied(), gas, min_gas, count, call_type },
                        0,
                    ));
                }
            }
        }
    }

    Ok(Default::default())
}

fn expect_emit(
    state: &mut Cheatcodes,
    depth: u64,
    checks: [bool; 5],
    address: Option<Address>,
    anonymous: bool,
) -> Result {
    state.expected_emits.push_back(ExpectedEmit {
        depth,
        checks,
        address,
        found: false,
        log: None,
        anonymous,
    });
    Ok(Default::default())
}

pub(crate) fn handle_expect_emit(state: &mut Cheatcodes, log: &alloy_primitives::Log) {
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

    // If there's anything to fill, we need to pop back.
    // Otherwise, if there are any events that are unmatched, we try to match to match them
    // in the order declared, so we start popping from the front (like a queue).
    let mut event_to_fill_or_check =
        if state.expected_emits.iter().any(|expected| expected.log.is_none()) {
            state.expected_emits.pop_back()
        } else {
            state.expected_emits.pop_front()
        }
        .expect("we should have an emit to fill or check");

    let Some(expected) = &event_to_fill_or_check.log else {
        // Unless the caller is trying to match an anonymous event, the first topic must be
        // filled.
        // TODO: failing this check should probably cause a warning
        if event_to_fill_or_check.anonymous || log.topics().first().is_some() {
            event_to_fill_or_check.log = Some(log.data.clone());
        }
        state.expected_emits.push_back(event_to_fill_or_check);
        return
    };

    event_to_fill_or_check.found = || -> bool {
        // Topic count must match.
        if expected.topics().len() != log.topics().len() {
            return false
        }
        // Match topics according to the checks.
        if !log
            .topics()
            .iter()
            .enumerate()
            .filter(|(i, _)| event_to_fill_or_check.checks[*i])
            .all(|(i, topic)| topic == &expected.topics()[i])
        {
            return false
        }
        // Maybe match source address.
        if event_to_fill_or_check.address.map_or(false, |addr| addr != log.address) {
            return false;
        }
        // Maybe match data.
        if event_to_fill_or_check.checks[4] && expected.data.as_ref() != log.data.data.as_ref() {
            return false
        }

        true
    }();

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

fn expect_revert(
    state: &mut Cheatcodes,
    reason: Option<&[u8]>,
    depth: u64,
    cheatcode: bool,
) -> Result {
    ensure!(
        state.expected_revert.is_none(),
        "you must call another function prior to expecting a second revert"
    );
    state.expected_revert = Some(ExpectedRevert {
        reason: reason.map(<[_]>::to_vec),
        depth,
        kind: if cheatcode {
            ExpectedRevertKind::Cheatcode { pending_processing: true }
        } else {
            ExpectedRevertKind::Default
        },
    });
    Ok(Default::default())
}

pub(crate) fn handle_expect_revert(
    is_cheatcode: bool,
    is_create: bool,
    expected_revert: Option<&[u8]>,
    status: InstructionResult,
    retdata: Bytes,
) -> Result<(Option<Address>, Bytes)> {
    let success_return = || {
        if is_create {
            (Some(DUMMY_CREATE_ADDRESS), Bytes::new())
        } else {
            (None, DUMMY_CALL_OUTPUT.clone())
        }
    };

    ensure!(!matches!(status, return_ok!()), "next call did not revert as expected");

    // If None, accept any revert
    let Some(expected_revert) = expected_revert else {
        return Ok(success_return());
    };

    if !expected_revert.is_empty() && retdata.is_empty() {
        bail!("call reverted as expected, but without data");
    }

    let mut actual_revert: Vec<u8> = retdata.into();

    // Try decoding as known errors
    if matches!(
        actual_revert.get(..4).map(|s| s.try_into().unwrap()),
        Some(Vm::CheatcodeError::SELECTOR | alloy_sol_types::Revert::SELECTOR)
    ) {
        if let Ok(decoded) = Vec::<u8>::abi_decode(&actual_revert[4..], false) {
            actual_revert = decoded;
        }
    }

    if actual_revert == expected_revert ||
        (is_cheatcode && memchr::memmem::find(&actual_revert, expected_revert).is_some())
    {
        Ok(success_return())
    } else {
        let stringify = |data: &[u8]| {
            if let Ok(s) = String::abi_decode(data, true) {
                return s;
            }
            if data.is_ascii() {
                return std::str::from_utf8(data).unwrap().to_owned();
            }
            hex::encode_prefixed(data)
        };
        Err(fmt_err!(
            "Error != expected error: {} != {}",
            stringify(&actual_revert),
            stringify(expected_revert),
        ))
    }
}

fn expect_safe_memory(state: &mut Cheatcodes, start: u64, end: u64, depth: u64) -> Result {
    ensure!(start < end, "memory range start ({start}) is greater than end ({end})");
    #[allow(clippy::single_range_in_vec_init)] // Wanted behaviour
    let offsets = state.allowed_mem_writes.entry(depth).or_insert_with(|| vec![0..0x60]);
    offsets.push(start..end);
    Ok(Default::default())
}
