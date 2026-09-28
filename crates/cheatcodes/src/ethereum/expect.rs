//! Revert expectations for native Ethereum execution.

use super::{Error, EthereumCheatcodes, FoundryEvmTypes, InstrStop, Message, MessageResult, Vm};
use alloy_primitives::{Address, Bytes, address, hex};
use alloy_sol_types::{SolError, SolValue};
use std::borrow::Cow;

const DUMMY_CREATE_ADDRESS: Address = address!("0x0000000000000000000000000000000000000001");
static DUMMY_CALL_OUTPUT: Bytes = Bytes::from_static(&[0; 8192]);

/// One pending expectation, scoped to the frame that installed it.
#[derive(Clone, Debug)]
pub(super) struct ExpectedRevert {
    reason: Option<Bytes>,
    depth: usize,
    partial_match: bool,
    reverter: Option<Address>,
    reverted_by: Option<Address>,
    max_depth: usize,
    count: u64,
    actual_count: u64,
}

impl EthereumCheatcodes {
    pub(super) fn expect_revert(
        &mut self,
        message: &Message<FoundryEvmTypes>,
        reason: Option<Bytes>,
        partial_match: bool,
        reverter: Option<Address>,
        count: u64,
    ) -> (InstrStop, Bytes) {
        if self.expected_revert.is_some() {
            return (
                InstrStop::Revert,
                Error::encode("you must call another function prior to expecting a second revert"),
            );
        }
        let depth = usize::from(message.depth.saturating_sub(1));
        self.expected_revert = Some(ExpectedRevert {
            reason,
            depth,
            partial_match,
            reverter,
            reverted_by: None,
            max_depth: depth,
            count,
            actual_count: 0,
        });
        (InstrStop::Return, Bytes::new())
    }

    pub(super) fn observe_revert_depth(&mut self, message: &Message<FoundryEvmTypes>) {
        if let Some(expected) = &mut self.expected_revert {
            expected.max_depth = expected.max_depth.max(usize::from(message.depth));
        }
    }

    pub(super) fn finish_expected_revert(
        &mut self,
        message: &Message<FoundryEvmTypes>,
        result: &mut MessageResult<FoundryEvmTypes>,
        is_create: bool,
    ) {
        let Some(mut expected) = self.expected_revert.take() else { return };
        let failed = !result.stop.is_success();
        if failed && expected.reverter.is_some() {
            // CALL processing keeps the outermost reverting address, as in the REVM inspector.
            if !is_create || expected.reverted_by.is_none() {
                expected.reverted_by = Some(message.destination);
            }
        }
        let parent_depth = usize::from(message.depth.saturating_sub(1));
        if parent_depth > expected.depth
            || (!is_create && !failed && self.config.internal_expect_revert && message.depth != 0)
        {
            self.expected_revert = Some(expected);
            return;
        }

        let outcome = expected.matches(result, self.config.internal_expect_revert);
        match outcome {
            Ok(()) => {
                expected.actual_count += 1;
                if expected.actual_count < expected.count {
                    expected.reverted_by = None;
                    self.expected_revert = Some(expected);
                }
                result.stop = InstrStop::Return;
                if is_create {
                    result.created_address = Some(DUMMY_CREATE_ADDRESS);
                    result.output = Bytes::new();
                } else {
                    result.output = DUMMY_CALL_OUTPUT.clone();
                }
            }
            Err(reason) => {
                result.stop = InstrStop::Revert;
                result.output = Error::encode(reason);
                result.created_address = None;
            }
        }
    }
}

impl ExpectedRevert {
    fn matches(
        &self,
        result: &MessageResult<FoundryEvmTypes>,
        internal_expect_revert: bool,
    ) -> Result<(), String> {
        if !internal_expect_revert && self.max_depth <= self.depth {
            return Err("call didn't revert at a lower depth than cheatcode call depth".into());
        }
        if self.count == 0 {
            return if result.stop.is_success() {
                Ok(())
            } else {
                Err("call reverted when it was expected not to revert".into())
            };
        }
        if result.stop.is_success() {
            return Err("next call did not revert as expected".into());
        }
        if let Some(reverter) = self.reverter
            && self.reverted_by != Some(reverter)
        {
            return Err(format!(
                "Reverter != expected reverter: {} != {}",
                self.reverted_by.unwrap_or_default(),
                reverter
            ));
        }
        let Some(reason) = &self.reason else { return Ok(()) };
        if result.output.is_empty() && !reason.is_empty() && result.stop == InstrStop::Revert {
            return Err("call reverted as expected, but without data".into());
        }
        if self.partial_match
            && let (Some(actual), Some(expected)) = (result.output.get(..4), reason.get(..4))
            && actual == expected
        {
            return Ok(());
        }
        if result.output == *reason || decode_revert(&result.output) == reason.as_ref() {
            return Ok(());
        }
        Err(format!(
            "Error != expected error: {} != {}",
            stringify(&result.output),
            stringify(reason)
        ))
    }
}

fn decode_revert(revert: &[u8]) -> Cow<'_, [u8]> {
    if let Some(selector) = revert.get(..4)
        && (selector == Vm::CheatcodeError::SELECTOR
            || selector == alloy_sol_types::Revert::SELECTOR)
        && let Ok(decoded) = Vec::<u8>::abi_decode(&revert[4..])
    {
        return Cow::Owned(decoded);
    }
    Cow::Borrowed(revert)
}

fn stringify(data: &[u8]) -> String {
    let decoded = decode_revert(data);
    if matches!(&decoded, Cow::Owned(_)) {
        return String::from_utf8_lossy(&decoded).into_owned();
    }
    if let Ok(value) = String::abi_decode(data) {
        return value;
    }
    if data.is_ascii() {
        return String::from_utf8_lossy(data).into_owned();
    }
    hex::encode_prefixed(data)
}
