use crate::{Cheatcode, Cheatcodes, CheatsCtxt, Result, Vm::*};
use alloy_primitives::{Address, Bytes, U256};
use foundry_evm_core::evm::FoundryEvmNetwork;
use revm::{
    bytecode::Bytecode,
    context::{ContextTr, JournalTr},
    interpreter::InstructionResult,
};
use std::{
    cmp::Ordering,
    collections::{BTreeMap, VecDeque},
};

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
    fn apply<FEN: FoundryEvmNetwork>(&self, state: &mut Cheatcodes<FEN>) -> Result {
        let Self {} = self;
        state.mocked_calls = Default::default();
        Ok(Default::default())
    }
}

impl Cheatcode for mockCall_0Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { callee, data, returnData } = self;
        let _ = make_acc_non_empty(callee, ccx)?;

        mock_call(ccx.state, callee, data, None, returnData, InstructionResult::Return);
        Ok(Default::default())
    }
}

impl Cheatcode for mockCall_1Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { callee, msgValue, data, returnData } = self;
        let _ = make_acc_non_empty(callee, ccx)?;

        mock_call(ccx.state, callee, data, Some(msgValue), returnData, InstructionResult::Return);
        Ok(Default::default())
    }
}

impl Cheatcode for mockCall_2Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { callee, data, returnData } = self;
        let _ = make_acc_non_empty(callee, ccx)?;

        mock_call(
            ccx.state,
            callee,
            &Bytes::from(*data),
            None,
            returnData,
            InstructionResult::Return,
        );
        Ok(Default::default())
    }
}

impl Cheatcode for mockCall_3Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { callee, msgValue, data, returnData } = self;
        let _ = make_acc_non_empty(callee, ccx)?;

        mock_call(
            ccx.state,
            callee,
            &Bytes::from(*data),
            Some(msgValue),
            returnData,
            InstructionResult::Return,
        );
        Ok(Default::default())
    }
}

impl Cheatcode for mockCall_4Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { callee, data, returnData, injectCode } = self;
        if *injectCode {
            let _ = make_acc_non_empty(callee, ccx)?;
        }

        mock_call(ccx.state, callee, data, None, returnData, InstructionResult::Return);
        Ok(Default::default())
    }
}

impl Cheatcode for mockCalls_0Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { callee, data, returnData } = self;
        let _ = make_acc_non_empty(callee, ccx)?;

        mock_calls(ccx.state, callee, data, None, returnData, InstructionResult::Return);
        Ok(Default::default())
    }
}

impl Cheatcode for mockCalls_1Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { callee, msgValue, data, returnData } = self;
        let _ = make_acc_non_empty(callee, ccx)?;

        mock_calls(ccx.state, callee, data, Some(msgValue), returnData, InstructionResult::Return);
        Ok(Default::default())
    }
}

impl Cheatcode for mockCallRevert_0Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { callee, data, revertData } = self;
        let _ = make_acc_non_empty(callee, ccx)?;

        mock_call(ccx.state, callee, data, None, revertData, InstructionResult::Revert);
        Ok(Default::default())
    }
}

impl Cheatcode for mockCallRevert_1Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { callee, msgValue, data, revertData } = self;
        let _ = make_acc_non_empty(callee, ccx)?;

        mock_call(ccx.state, callee, data, Some(msgValue), revertData, InstructionResult::Revert);
        Ok(Default::default())
    }
}

impl Cheatcode for mockCallRevert_2Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { callee, data, revertData } = self;
        let _ = make_acc_non_empty(callee, ccx)?;

        mock_call(
            ccx.state,
            callee,
            &Bytes::from(*data),
            None,
            revertData,
            InstructionResult::Revert,
        );
        Ok(Default::default())
    }
}

impl Cheatcode for mockCallRevert_3Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { callee, msgValue, data, revertData } = self;
        let _ = make_acc_non_empty(callee, ccx)?;

        mock_call(
            ccx.state,
            callee,
            &Bytes::from(*data),
            Some(msgValue),
            revertData,
            InstructionResult::Revert,
        );
        Ok(Default::default())
    }
}

impl Cheatcode for mockFunctionCall {
    fn apply<FEN: FoundryEvmNetwork>(&self, state: &mut Cheatcodes<FEN>) -> Result {
        let Self { callee, target, data } = self;
        state.mocked_functions.entry(*callee).or_default().insert(data.clone(), *target);

        Ok(Default::default())
    }
}

fn mock_call<FEN: FoundryEvmNetwork>(
    state: &mut Cheatcodes<FEN>,
    callee: &Address,
    cdata: &Bytes,
    value: Option<&U256>,
    rdata: &Bytes,
    ret_type: InstructionResult,
) {
    mock_calls(state, callee, cdata, value, std::slice::from_ref(rdata), ret_type)
}

fn mock_calls<FEN: FoundryEvmNetwork>(
    state: &mut Cheatcodes<FEN>,
    callee: &Address,
    cdata: &Bytes,
    value: Option<&U256>,
    rdata_vec: &[Bytes],
    ret_type: InstructionResult,
) {
    state.mocked_calls.entry(*callee).or_default().insert(
        MockCallDataContext { calldata: cdata.clone(), value: value.copied() },
        rdata_vec
            .iter()
            .map(|rdata| MockCallReturnData { ret_type, data: rdata.clone() })
            .collect::<VecDeque<_>>(),
    );
}

// Etches a single byte onto the account if it is empty to circumvent the `extcodesize`
// check Solidity might perform.
fn make_acc_non_empty<FEN: FoundryEvmNetwork>(
    callee: &Address,
    ccx: &mut CheatsCtxt<'_, '_, FEN>,
) -> Result {
    let empty_bytecode = {
        let acc = ccx.ecx.journal_mut().load_account(*callee)?;
        acc.info.code.as_ref().is_none_or(Bytecode::is_empty)
    };
    if empty_bytecode {
        let code = Bytecode::new_raw(Bytes::from_static(&[0u8]));
        ccx.ecx.journal_mut().set_code(*callee, code);
    }

    Ok(Default::default())
}

/// Finds the return data queue of the mock matching a call.
///
/// An exact calldata and value match wins. Otherwise, the first mock in map order whose calldata
/// prefixes `input` and whose value, if set, equals `value` is used.
pub(crate) fn find_mock_returns<'a, T>(
    mocks: &'a mut BTreeMap<MockCallDataContext, VecDeque<T>>,
    input: &Bytes,
    value: Option<U256>,
) -> Option<&'a mut VecDeque<T>> {
    let ctx = MockCallDataContext { calldata: input.clone(), value };
    // Reversed `Ord` puts all matches at or after `ctx`, with the exact key first.
    mocks
        .range_mut(ctx..)
        .find(|(mock, _)| {
            input.get(..mock.calldata.len()) == Some(&mock.calldata[..])
                && mock.value.is_none_or(|mock_value| Some(mock_value) == value)
        })
        .map(|(_, v)| v)
}

/// Consumes the front return data of a mock, keeping the last one for every later call.
pub(crate) fn advance_mock_returns<T>(queue: &mut VecDeque<T>) {
    if queue.len() > 1 {
        queue.pop_front();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock(
        calldata: &'static [u8],
        value: Option<u64>,
        result: u8,
    ) -> (MockCallDataContext, VecDeque<u8>) {
        (
            MockCallDataContext {
                calldata: Bytes::from_static(calldata),
                value: value.map(U256::from),
            },
            VecDeque::from([result]),
        )
    }

    #[test]
    fn mock_matching_precedence() {
        let mut mocks = BTreeMap::from([
            mock(b"call", Some(1), 1),
            mock(b"call", Some(4), 9),
            mock(b"call", None, 2),
            mock(b"cal", Some(1), 3),
            mock(b"cal", Some(0), 4),
            mock(b"cal", Some(2), 5),
            mock(b"cal", None, 6),
            mock(b"ca", Some(3), 7),
            mock(b"call-longer", Some(1), 8),
        ]);

        for (input, value, expected) in [
            (b"call", Some(1), 1),
            (b"call", Some(3), 2),
            (b"call", None, 2),
            (b"calx", Some(1), 3),
            (b"calx", Some(3), 6),
        ] {
            let queue =
                find_mock_returns(&mut mocks, &Bytes::from_static(input), value.map(U256::from))
                    .unwrap();
            assert_eq!(queue.front(), Some(&expected), "input: {input:?}, value: {value:?}");
        }
    }

    #[test]
    fn absent_transfer_value_does_not_match_zero() {
        let mut mocks = BTreeMap::from([mock(b"call", Some(0), 1)]);
        let input = Bytes::from_static(b"call");
        assert!(find_mock_returns(&mut mocks, &input, None).is_none());
        assert_eq!(
            find_mock_returns(&mut mocks, &input, Some(U256::ZERO)).unwrap().front(),
            Some(&1)
        );

        mocks.extend([mock(b"cal", None, 2)]);
        assert_eq!(find_mock_returns(&mut mocks, &input, None).unwrap().front(), Some(&2));
    }

    #[test]
    fn empty_matched_queue_is_distinct_from_no_match() {
        let mut mocks = BTreeMap::from([mock(b"cal", None, 1)]);
        mocks.insert(
            MockCallDataContext { calldata: Bytes::from_static(b"call"), value: None },
            VecDeque::new(),
        );

        let queue = find_mock_returns(&mut mocks, &Bytes::from_static(b"call"), None).unwrap();
        assert!(queue.is_empty());
        assert!(find_mock_returns(&mut mocks, &Bytes::from_static(b"other"), None).is_none());
    }

    #[test]
    fn advancing_returns_keeps_last_result() {
        let mut queue = VecDeque::from([1, 2, 3]);
        advance_mock_returns(&mut queue);
        assert_eq!(queue, VecDeque::from([2, 3]));
        advance_mock_returns(&mut queue);
        assert_eq!(queue, VecDeque::from([3]));
        advance_mock_returns(&mut queue);
        assert_eq!(queue, VecDeque::from([3]));

        queue.clear();
        advance_mock_returns(&mut queue);
        assert!(queue.is_empty());
    }
}
