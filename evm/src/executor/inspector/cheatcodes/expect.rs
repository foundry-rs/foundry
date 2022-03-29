use super::Cheatcodes;
use crate::abi::HEVMCalls;
use bytes::Bytes;
use ethers::{
    abi::{AbiEncode, RawLog},
    types::{Address, H160},
};
use revm::{return_ok, Database, EVMData, Return};

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

    let (err, actual_revert): (_, Bytes) = match retdata {
        _ if retdata.len() >= 4 && retdata[0..4] == [8, 195, 121, 160] => {
            // It's a revert string, so we do some conversion to perform the check
            let decoded_data: Bytes =
                ethers::abi::decode(&[ethers::abi::ParamType::Bytes], &retdata[4..])
                    .expect("String error code, but data is not a string")[0]
                    .clone()
                    .into_bytes()
                    .expect("Cannot fail as this is bytes")
                    .into();

            (
                format!(
                    "Error != expected error: '{}' != '{}'",
                    String::from_utf8(decoded_data.to_vec())
                        .ok()
                        .unwrap_or_else(|| hex::encode(&decoded_data)),
                    String::from_utf8(expected_revert.to_vec())
                        .ok()
                        .unwrap_or_else(|| hex::encode(&expected_revert))
                )
                .encode()
                .into(),
                decoded_data,
            )
        }
        _ => (
            format!(
                "Error != expected error: 0x{} != 0x{}",
                hex::encode(&retdata),
                hex::encode(&expected_revert)
            )
            .encode()
            .into(),
            retdata,
        ),
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
    /// Whether the log was actually found in the subcalls
    pub found: bool,
}

pub fn handle_expect_emit(state: &mut Cheatcodes, log: RawLog) {
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
        if expected.topics[0] == log.topics[0] {
            // Topic 0 can match, but the amount of topics can differ.
            if expected.topics.len() != log.topics.len() {
                next_expect.found = false;
            } else {
                let topics_match = log
                    .topics
                    .iter()
                    .skip(1)
                    .enumerate()
                    .filter(|(i, _)| next_expect.checks[*i])
                    .all(|(i, topic)| topic == &expected.topics[i + 1]);

                // Maybe check data
                next_expect.found = if next_expect.checks[3] {
                    expected.data == log.data && topics_match
                } else {
                    topics_match
                };
            }
        }
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
        HEVMCalls::ExpectEmit(inner) => {
            state.expected_emits.push(ExpectedEmit {
                depth: data.subroutine.depth(),
                checks: [inner.0, inner.1, inner.2, inner.3],
                ..Default::default()
            });
            Ok(Bytes::new())
        }
        HEVMCalls::ExpectCall(inner) => {
            state.expected_calls.entry(inner.0).or_default().push(inner.1.to_vec().into());
            Ok(Bytes::new())
        }
        HEVMCalls::MockCall(inner) => {
            state
                .mocked_calls
                .entry(inner.0)
                .or_default()
                .insert(inner.1.to_vec().into(), inner.2.to_vec().into());
            Ok(Bytes::new())
        }
        HEVMCalls::ClearMockedCalls(_) => {
            state.mocked_calls = Default::default();
            Ok(Bytes::new())
        }
        _ => return None,
    })
}
