use super::Cheatcodes;
use crate::abi::HEVMCalls;
use bytes::Bytes;
use ethers::{abi::AbiEncode, types::Address};
use once_cell::sync::Lazy;
use revm::{return_ok, Database, EVMData, Return};
use std::str::FromStr;

/// For some cheatcodes we may internally change the status of the call, i.e. in `expectRevert`.
/// Solidity will see a successful call and attempt to decode the return data. Therefore, we need
/// to populate the return with dummy bytes so the decode doesn't fail.
static DUMMY_CALL_OUTPUT: [u8; 320] = [0u8; 320];

/// Same reasoning as [DUMMY_CALL_OUTPUT], but for creates.
static DUMMY_CREATE_ADDRESS: Lazy<Address> =
    Lazy::new(|| Address::from_str("0000000000000000000000000000000000000001").unwrap());

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

    if retdata.is_empty() {
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
                    String::from_utf8_lossy(&decoded_data),
                    String::from_utf8_lossy(expected_revert)
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
            (Some(*DUMMY_CREATE_ADDRESS), Bytes::new())
        } else {
            (None, DUMMY_CALL_OUTPUT.to_vec().into())
        })
    } else {
        Err(err)
    }
}

pub fn apply<DB: Database>(
    state: &mut Cheatcodes,
    data: &mut EVMData<'_, DB>,
    call: &HEVMCalls,
) -> Option<Result<Bytes, Bytes>> {
    Some(match call {
        HEVMCalls::ExpectRevert0(inner) => {
            expect_revert(state, inner.0.to_vec().into(), data.subroutine.depth())
        }
        HEVMCalls::ExpectRevert1(inner) => {
            expect_revert(state, inner.0.to_vec().into(), data.subroutine.depth())
        }
        /*HEVMCalls::ExpectEmit(_) => {}
        HEVMCalls::ExpectCall(_) => {}*/
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
