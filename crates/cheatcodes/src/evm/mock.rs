use crate::{Cheatcode, Vm::*};
use alloy_primitives::{Address, Bytes, KECCAK256_EMPTY, U256, keccak256, map::HashMap};
use std::{
    cmp::Ordering,
    collections::{BTreeMap, VecDeque},
};

#[cfg(feature = "revm")]
use crate::{Cheatcodes, CheatsCtxt, Result};
#[cfg(feature = "revm")]
use foundry_evm_core::evm::FoundryEvmNetwork;
#[cfg(feature = "revm")]
use revm::{
    bytecode::Bytecode,
    context::{ContextTr, JournalTr},
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
    /// Whether the mocked call reverts.
    pub reverts: bool,
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
    #[cfg(feature = "revm")]
    fn apply<FEN: FoundryEvmNetwork>(&self, state: &mut Cheatcodes<FEN>) -> Result {
        state.mocked_calls.clear();
        Ok(Default::default())
    }

    fn apply_evm2(
        &self,
        state: &mut crate::ethereum::Cheatcodes,
        _: &mut evm2::interpreter::Interpreter<'_, '_, foundry_evm_core::ethereum::FoundryEvmTypes>,
    ) -> std::result::Result<Bytes, crate::ethereum::ApplyError> {
        state.mocked_calls.clear();
        Ok(Bytes::new())
    }
}

macro_rules! impl_mock_call {
    ($call:ident { $callee:ident, $data:ident, $returns:ident $(, $field:ident)* }, $input:expr, $value:expr, $outputs:expr, $reverts:expr, $inject:expr) => {
        impl Cheatcode for $call {
            #[cfg(feature = "revm")]
            fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
                let Self { $callee, $data, $returns $(, $field)* } = self;
                if $inject {
                    make_acc_non_empty($callee, ccx)?;
                }
                mock_calls(&mut ccx.state.mocked_calls, $callee, $input, $value, $outputs, $reverts);
                Ok(Default::default())
            }

            fn apply_evm2(
                &self,
                state: &mut crate::ethereum::Cheatcodes,
                interp: &mut evm2::interpreter::Interpreter<'_, '_, foundry_evm_core::ethereum::FoundryEvmTypes>,
            ) -> std::result::Result<Bytes, crate::ethereum::ApplyError> {
                let Self { $callee, $data, $returns $(, $field)* } = self;
                if $inject {
                    let mut account = interp.host().state_mut().account($callee)?;
                    if account.code_hash() == KECCAK256_EMPTY {
                        let code = evm2::bytecode::Bytecode::new_raw_checked(Bytes::from_static(&[0]))
                            .expect("STOP is valid bytecode");
                        account.set_code(keccak256(code.original_bytes()), code);
                    }
                }
                mock_calls(&mut state.mocked_calls, $callee, $input, $value, $outputs, $reverts);
                Ok(Bytes::new())
            }
        }
    };
}

impl_mock_call!(
    mockCall_0Call { callee, data, returnData },
    data,
    None,
    std::slice::from_ref(returnData),
    false,
    true
);
impl_mock_call!(
    mockCall_1Call { callee, data, returnData, msgValue },
    data,
    Some(msgValue),
    std::slice::from_ref(returnData),
    false,
    true
);
impl_mock_call!(
    mockCall_2Call { callee, data, returnData },
    &Bytes::from(*data),
    None,
    std::slice::from_ref(returnData),
    false,
    true
);
impl_mock_call!(
    mockCall_3Call { callee, data, returnData, msgValue },
    &Bytes::from(*data),
    Some(msgValue),
    std::slice::from_ref(returnData),
    false,
    true
);
impl_mock_call!(
    mockCall_4Call { callee, data, returnData, injectCode },
    data,
    None,
    std::slice::from_ref(returnData),
    false,
    *injectCode
);
impl_mock_call!(mockCalls_0Call { callee, data, returnData }, data, None, returnData, false, true);
impl_mock_call!(
    mockCalls_1Call { callee, data, returnData, msgValue },
    data,
    Some(msgValue),
    returnData,
    false,
    true
);
impl_mock_call!(
    mockCallRevert_0Call { callee, data, revertData },
    data,
    None,
    std::slice::from_ref(revertData),
    true,
    true
);
impl_mock_call!(
    mockCallRevert_1Call { callee, data, revertData, msgValue },
    data,
    Some(msgValue),
    std::slice::from_ref(revertData),
    true,
    true
);
impl_mock_call!(
    mockCallRevert_2Call { callee, data, revertData },
    &Bytes::from(*data),
    None,
    std::slice::from_ref(revertData),
    true,
    true
);
impl_mock_call!(
    mockCallRevert_3Call { callee, data, revertData, msgValue },
    &Bytes::from(*data),
    Some(msgValue),
    std::slice::from_ref(revertData),
    true,
    true
);

impl Cheatcode for mockFunctionCall {
    #[cfg(feature = "revm")]
    fn apply<FEN: FoundryEvmNetwork>(&self, state: &mut Cheatcodes<FEN>) -> Result {
        let Self { callee, target, data } = self;
        state.mocked_functions.entry(*callee).or_default().insert(data.clone(), *target);

        Ok(Default::default())
    }

    fn apply_evm2(
        &self,
        state: &mut crate::ethereum::Cheatcodes,
        _: &mut evm2::interpreter::Interpreter<'_, '_, foundry_evm_core::ethereum::FoundryEvmTypes>,
    ) -> std::result::Result<Bytes, crate::ethereum::ApplyError> {
        state
            .mocked_functions
            .entry(self.callee)
            .or_default()
            .insert(self.data.clone(), self.target);
        Ok(Bytes::new())
    }
}

fn mock_calls(
    mocks: &mut HashMap<Address, BTreeMap<MockCallDataContext, VecDeque<MockCallReturnData>>>,
    callee: &Address,
    cdata: &Bytes,
    value: Option<&U256>,
    rdata_vec: &[Bytes],
    reverts: bool,
) {
    mocks.entry(*callee).or_default().insert(
        MockCallDataContext { calldata: cdata.clone(), value: value.copied() },
        rdata_vec.iter().map(|rdata| MockCallReturnData { reverts, data: rdata.clone() }).collect(),
    );
}

// Etches a single byte onto the account if it is empty to circumvent the `extcodesize`
// check Solidity might perform.
#[cfg(feature = "revm")]
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
