use super::{Cheatcode, Cheatcodes, CheatsCtxt, DatabaseExt, Result};
use crate::Vm::*;
use alloy_primitives::{Address, Bytes, U256};
use revm::interpreter::InstructionResult;
use std::cmp::Ordering;

/// Mocked call data.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct MockCallDataContext {
    /// The partial calldata to match for mock
    pub calldata: Bytes,
    /// The value to match for mock
    pub value: Option<U256>,
}

/// Mocked return data.
#[derive(Clone, Debug)]
pub struct MockCallReturnData {
    /// The return type for the mocked call
    pub ret_type: InstructionResult,
    /// Return data or error
    pub data: Bytes,
}

impl PartialOrd for MockCallDataContext {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
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

impl Cheatcode for clearMockedCallsCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self {} = self;
        state.mocked_calls = Default::default();
        Ok(Default::default())
    }
}

impl Cheatcode for mockCall_0Call {
    fn apply_full<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { callee, data, returnData } = self;
        ccx.data.journaled_state.load_account(*callee, ccx.data.db)?;
        mock_call(ccx.state, callee, data, None, returnData, InstructionResult::Return);
        Ok(Default::default())
    }
}

impl Cheatcode for mockCall_1Call {
    fn apply_full<DB: DatabaseExt>(&self, ccx: &mut CheatsCtxt<DB>) -> Result {
        let Self { callee, msgValue, data, returnData } = self;
        ccx.data.journaled_state.load_account(*callee, ccx.data.db)?;
        mock_call(ccx.state, callee, data, Some(msgValue), returnData, InstructionResult::Return);
        Ok(Default::default())
    }
}

impl Cheatcode for mockCallRevert_0Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { callee, data, revertData } = self;
        mock_call(state, callee, data, None, revertData, InstructionResult::Return);
        Ok(Default::default())
    }
}

impl Cheatcode for mockCallRevert_1Call {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { callee, msgValue, data, revertData } = self;
        mock_call(state, callee, data, Some(msgValue), revertData, InstructionResult::Return);
        Ok(Default::default())
    }
}

#[allow(clippy::ptr_arg)] // Not public API, doesn't matter
fn mock_call(
    state: &mut Cheatcodes,
    callee: &Address,
    cdata: &Vec<u8>,
    value: Option<&U256>,
    rdata: &Vec<u8>,
    ret_type: InstructionResult,
) {
    state.mocked_calls.entry(*callee).or_default().insert(
        MockCallDataContext { calldata: Bytes::copy_from_slice(cdata), value: value.cloned() },
        MockCallReturnData { ret_type, data: Bytes::copy_from_slice(rdata) },
    );
}
