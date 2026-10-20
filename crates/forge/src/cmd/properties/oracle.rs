//! The oracle check of `forge properties`: does a possible bug dispute a call outcome that a
//! passing project test asserts?
//!
//! A failing generated test disputes the outcome of its last call when the revert of that call
//! ends the test, or when the call succeeds and the test expected a revert. A passing project
//! test asserts the outcome of a call when it expects the call to revert and it reverts, or when
//! the call succeeds. Two tests make the same call in the same state when the call and every
//! state-changing call of the test contract before it, from the deployment on, are equal. The EVM
//! is deterministic, so the call then has the same outcome in both tests, and a project test that
//! asserts it contradicts the possible bug.
//!
//! The check is sound for this subset and silent outside it. It does not compare asserted
//! values, calls through internal library functions, or equal states that two tests reach by
//! different calls. It assumes that the target does not read the test contract's own code, state,
//! or gas, for example through a callback, and that a passing test does not ignore the result of
//! a low-level call.

use crate::result::{TestKind, TestResult, TestStatus};
use alloy_primitives::{B256, Bytes, keccak256};
use foundry_evm::{
    constants::{CHEATCODE_ADDRESS, HARDHAT_CONSOLE_ADDRESS},
    core::abi::Vm,
    traces::{CallTrace, TraceKind},
};
use serde::{Deserialize, Serialize};

/// Cheatcodes that change no state that a target reads. A prank changes only the caller of the
/// next calls, and the caller is part of each call's identity.
const NEUTRAL_CHEATCODES: [&str; 10] = [
    "label",
    "prank",
    "startPrank",
    "stopPrank",
    "expectRevert",
    "expectPartialRevert",
    "expectEmit",
    "expectEmitAnonymous",
    "expectCall",
    "expectCallMinGas",
];

/// One call of a test contract in one state.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub(super) struct CallKey {
    /// Digest of the state-changing calls of the test contract before the call.
    state: B256,
    /// Digest of the caller, the callee, the value, and the calldata.
    call: B256,
}

/// The outcome of a call.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum CallOutcome {
    /// The call reverts with this data.
    Revert(Bytes),
    /// The call succeeds.
    Success,
}

/// A call outcome that a test asserts or disputes.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct CallClaim {
    pub key: CallKey,
    pub outcome: CallOutcome,
}

/// A call of the test contract in the test function, and whether the test expected it to revert.
struct TestCall<'a> {
    key: CallKey,
    trace: &'a CallTrace,
    expects_revert: bool,
}

/// Returns the outcome of the last call of a failing test when that outcome makes the test fail.
pub(super) fn disputed_call(result: &TestResult) -> Option<CallClaim> {
    if result.status != TestStatus::Failure || matches!(result.kind, TestKind::Invariant { .. }) {
        return None;
    }
    let (_, execution) = result.traces.iter().rfind(|(kind, _)| kind.is_execution())?;
    let root = &execution.arena.nodes().first()?.trace;
    let calls = test_calls(result);
    let last = calls.last()?;
    let outcome = match (last.trace.success, last.expects_revert) {
        (true, true) => CallOutcome::Success,
        // The revert data of the test is the call's own, so the call's revert ends the test.
        (false, false) if !root.success && root.output == last.trace.output => {
            CallOutcome::Revert(last.trace.output.clone())
        }
        _ => return None,
    };
    Some(CallClaim { key: last.key, outcome })
}

/// Returns the call outcomes that a passing test asserts: expected reverts and successful calls.
pub(super) fn asserted_calls(result: &TestResult) -> Vec<CallClaim> {
    if result.status != TestStatus::Success || matches!(result.kind, TestKind::Invariant { .. }) {
        return vec![];
    }
    test_calls(result)
        .into_iter()
        .filter_map(|call| {
            let outcome = match (call.trace.success, call.expects_revert) {
                (true, false) => CallOutcome::Success,
                (false, true) => CallOutcome::Revert(call.trace.output.clone()),
                _ => return None,
            };
            Some(CallClaim { key: call.key, outcome })
        })
        .collect()
}

/// Returns the calls of the test contract in the test function, in order, each keyed by the
/// state-changing calls before it in the deployment, `setUp`, and test traces.
fn test_calls(result: &TestResult) -> Vec<TestCall<'_>> {
    let mut state = B256::ZERO;
    let mut calls = Vec::new();
    for (kind, arena) in &result.traces {
        let nodes = arena.arena.nodes();
        let Some(root) = nodes.first() else { continue };
        // The test contract's own code differs between tests; its address and deployer do not.
        if kind.is_deployment() {
            state = keccak256(
                [state.as_slice(), root.trace.caller.as_slice(), root.trace.address.as_slice()]
                    .concat(),
            );
        }
        let mut expects_revert = false;
        for call in root.children.iter().map(|&index| &nodes[index].trace) {
            if call.address == CHEATCODE_ADDRESS {
                let signature = call.data.get(..4).and_then(|selector| {
                    Vm::VmCalls::signature_by_selector(selector.try_into().ok()?)
                });
                let name =
                    signature.and_then(|signature| signature.split_once('(')).map(|(name, _)| name);
                // An expected revert count can also expect no revert.
                if matches!(name, Some("expectRevert" | "expectPartialRevert"))
                    && !signature.is_some_and(|signature| signature.contains("uint64"))
                {
                    expects_revert = true;
                }
                if !name.is_some_and(|name| NEUTRAL_CHEATCODES.contains(&name))
                    && changes_state(call)
                {
                    state = keccak256([state, call_digest(call)].concat());
                }
                continue;
            }
            if call.address == HARDHAT_CONSOLE_ADDRESS {
                continue;
            }
            if *kind == TraceKind::Execution {
                calls.push(TestCall {
                    key: CallKey { state, call: call_digest(call) },
                    trace: call,
                    expects_revert,
                });
            }
            expects_revert = false;
            if changes_state(call) {
                state = keccak256([state, call_digest(call)].concat());
            }
        }
    }
    calls
}

/// Whether a call can change state: a creation, or a call that succeeds and is not static.
const fn changes_state(call: &CallTrace) -> bool {
    call.kind.is_any_create() || (call.success && !call.kind.is_static_call())
}

/// Returns the digest of the kind, caller, callee, value, and calldata of a call.
fn call_digest(call: &CallTrace) -> B256 {
    keccak256(
        [
            &[u8::from(call.kind.is_any_create())],
            call.caller.as_slice(),
            call.address.as_slice(),
            &call.value.to_be_bytes::<32>(),
            &call.data,
        ]
        .concat(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{Address, address};
    use foundry_evm::traces::{CallKind, CallTraceArena, CallTraceNode, SparsedTraceArena};

    const TEST: Address = address!("0x7FA9385bE102ac3EAc297483Dd6233D62b3e1496");
    const VAULT: Address = address!("0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f");
    /// The selector of `ZeroAmount()`.
    const ZERO_AMOUNT: [u8; 4] = [0x1f, 0x2a, 0x20, 0x05];

    fn call(address: Address, data: Vec<u8>, success: bool, output: &[u8]) -> CallTrace {
        CallTrace {
            caller: TEST,
            address,
            kind: CallKind::Call,
            data: data.into(),
            success,
            output: Bytes::copy_from_slice(output),
            ..Default::default()
        }
    }

    fn cheatcode(signature: &str, args: &[u8]) -> CallTrace {
        let index = Vm::VmCalls::SIGNATURES.iter().position(|known| *known == signature).unwrap();
        call(CHEATCODE_ADDRESS, [&Vm::VmCalls::SELECTORS[index][..], args].concat(), true, &[])
    }

    /// `deposit(uint256)` with a one-byte argument.
    fn deposit(amount: u8) -> Vec<u8> {
        vec![0xb6, 0xb5, 0x5f, 0x25, amount]
    }

    fn arena(root: CallTrace, calls: Vec<CallTrace>) -> SparsedTraceArena {
        let mut nodes = vec![CallTraceNode {
            trace: root,
            children: (1..=calls.len()).collect(),
            ..Default::default()
        }];
        nodes.extend(calls.into_iter().enumerate().map(|(index, trace)| CallTraceNode {
            parent: Some(0),
            idx: index + 1,
            trace,
            ..Default::default()
        }));
        let mut arena = CallTraceArena::default();
        *arena.nodes_mut() = nodes;
        SparsedTraceArena { arena, ignored: Default::default(), diagnostics: Default::default() }
    }

    /// A test whose `setUp` deploys the vault and whose test function makes `calls`.
    fn result(status: TestStatus, output: &[u8], calls: Vec<CallTrace>) -> TestResult {
        let root = |success, output: &[u8]| CallTrace {
            address: TEST,
            success,
            output: Bytes::copy_from_slice(output),
            ..Default::default()
        };
        let create = |address| CallTrace {
            caller: TEST,
            address,
            kind: CallKind::Create,
            success: true,
            ..Default::default()
        };
        TestResult {
            status,
            traces: vec![
                (TraceKind::Deployment, arena(create(TEST), vec![])),
                (TraceKind::Setup, arena(root(true, &[]), vec![create(VAULT)])),
                (TraceKind::Execution, arena(root(status == TestStatus::Success, output), calls)),
            ],
            ..Default::default()
        }
    }

    // The generated test expects `deposit(0)` to succeed, and a project test expects its revert in
    // the same state. A label changes no state.
    #[test]
    fn expected_revert_contradicts_disputed_revert() {
        let bug = result(
            TestStatus::Failure,
            &ZERO_AMOUNT,
            vec![
                cheatcode("label(address,string)", &[]),
                call(VAULT, deposit(0), false, &ZERO_AMOUNT),
            ],
        );
        let project = result(
            TestStatus::Success,
            &[],
            vec![
                cheatcode("expectRevert(bytes4)", &ZERO_AMOUNT),
                call(VAULT, deposit(0), false, &ZERO_AMOUNT),
            ],
        );
        let dispute = disputed_call(&bug).unwrap();
        assert_eq!(dispute.outcome, CallOutcome::Revert(ZERO_AMOUNT.into()));
        assert_eq!(asserted_calls(&project), vec![dispute]);
    }

    // The generated test expects `deposit(5)` to revert, and a project test makes it succeed.
    #[test]
    fn successful_call_contradicts_expected_revert() {
        let bug = result(
            TestStatus::Failure,
            b"next call did not revert as expected",
            vec![cheatcode("expectRevert()", &[]), call(VAULT, deposit(5), true, &[])],
        );
        let project = result(TestStatus::Success, &[], vec![call(VAULT, deposit(5), true, &[])]);
        let dispute = disputed_call(&bug).unwrap();
        assert_eq!(dispute.outcome, CallOutcome::Success);
        assert_eq!(asserted_calls(&project), vec![dispute]);
    }

    // A deposit or a deal before the call changes the state, so the calls are not the same.
    #[test]
    fn earlier_state_change_separates_calls() {
        let bug = result(
            TestStatus::Failure,
            &ZERO_AMOUNT,
            vec![call(VAULT, deposit(0), false, &ZERO_AMOUNT)],
        );
        let dispute = disputed_call(&bug).unwrap();
        for earlier in
            [call(VAULT, deposit(5), true, &[]), cheatcode("deal(address,uint256)", &[0; 64])]
        {
            let project = result(
                TestStatus::Success,
                &[],
                vec![
                    earlier,
                    cheatcode("expectRevert(bytes4)", &ZERO_AMOUNT),
                    call(VAULT, deposit(0), false, &ZERO_AMOUNT),
                ],
            );
            let asserted = asserted_calls(&project);
            assert!(asserted.iter().any(|claim| claim.outcome == dispute.outcome));
            assert!(!asserted.contains(&dispute));
        }
    }

    // A test that fails on an assertion, or that expects a revert count, disputes no call, and a
    // passing test asserts no call whose revert it catches.
    #[test]
    fn other_failures_dispute_no_call() {
        let assertion = result(
            TestStatus::Failure,
            b"assertion failed",
            vec![call(VAULT, deposit(5), true, &[])],
        );
        assert_eq!(disputed_call(&assertion), None);
        let count = result(
            TestStatus::Failure,
            b"next call did not revert as expected",
            vec![cheatcode("expectRevert(uint64)", &[0; 32]), call(VAULT, deposit(5), true, &[])],
        );
        assert_eq!(disputed_call(&count), None);
        let caught =
            result(TestStatus::Success, &[], vec![call(VAULT, deposit(0), false, &ZERO_AMOUNT)]);
        assert_eq!(asserted_calls(&caught), vec![]);
    }
}
