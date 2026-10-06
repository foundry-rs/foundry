use std::fmt::{self, Display};

use crate::{
    Cheatcode, Cheatcodes, CheatsCtxt, Error, Result,
    Vm::*,
    expected_emit::{ExpectedEmit, observe_log},
};
use alloy_primitives::{
    Address, Bytes, Log, LogData as RawLog, U256, hex, keccak256,
    map::{HashMap, hash_map::Entry},
};
use alloy_sol_types::{SolCall, SolValue};
use foundry_evm_core::evm::FoundryEvmNetwork;
use itertools::Itertools;
use revm::interpreter::{
    CallScheme, InstructionResult, Interpreter, InterpreterAction, interpreter_types::LoopControl,
};
use tempo_contracts::precompiles::ISignatureVerifier;
use tempo_precompiles::SIGNATURE_VERIFIER_ADDRESS;

use super::revert_handlers::RevertParameters;
/// Tracks the expected calls per address.
///
/// For each address, we track the expected calls per call data and optional call scheme. We track
/// it in such manner so that we don't mix together calldatas that only contain selectors and
/// calldatas that contain selector and arguments (partial and full matches), or unrestricted calls
/// and calls restricted to a specific scheme.
///
/// This then allows us to customize the matching behavior for each call data on the
/// `ExpectedCallData` struct and track how many times we've actually seen the call on the second
/// element of the tuple.
pub type ExpectedCallTracker = HashMap<Address, ExpectedCallsForTarget>;

/// Tracks calldata, scheme and count expectations for one target.
pub type ExpectedCallsForTarget = HashMap<(Bytes, Option<CallScheme>), (ExpectedCallData, u64)>;

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
    /// The expected data returned by the revert, None being any.
    pub reason: Option<Bytes>,
    /// The depth at which the revert is expected.
    pub depth: usize,
    /// The type of expected revert.
    pub kind: ExpectedRevertKind,
    /// If true then only the first 4 bytes of expected data returned by the revert are checked.
    pub partial_match: bool,
    /// Contract expected to revert next call.
    pub reverter: Option<Address>,
    /// Address that reverted the call.
    pub reverted_by: Option<Address>,
    /// Max call depth reached during next call execution.
    pub max_depth: usize,
    /// Number of times this revert is expected.
    pub count: u64,
    /// Actual number of times this revert has been seen.
    pub actual_count: u64,
}

#[derive(Clone, Debug)]
pub struct ExpectedCreate {
    /// The address that deployed the contract
    pub deployer: Address,
    /// Runtime bytecode of the contract
    pub bytecode: Bytes,
    /// Whether deployed with CREATE or CREATE2
    pub create_scheme: CreateScheme,
}

#[derive(Clone, Debug)]
pub enum CreateScheme {
    Create,
    Create2,
}

impl Display for CreateScheme {
    fn fmt(&self, f: &mut fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::Create => write!(f, "CREATE"),
            Self::Create2 => write!(f, "CREATE2"),
        }
    }
}

impl From<revm::context_interface::CreateScheme> for CreateScheme {
    fn from(scheme: revm::context_interface::CreateScheme) -> Self {
        match scheme {
            revm::context_interface::CreateScheme::Create => Self::Create,
            revm::context_interface::CreateScheme::Create2 { .. } => Self::Create2,
            _ => unimplemented!("Unsupported create scheme"),
        }
    }
}

impl CreateScheme {
    pub const fn eq(&self, create_scheme: Self) -> bool {
        matches!(
            (self, create_scheme),
            (Self::Create, Self::Create) | (Self::Create2, Self::Create2 { .. })
        )
    }
}

impl Cheatcode for expectCall_0Call {
    fn apply<FEN: FoundryEvmNetwork>(&self, state: &mut Cheatcodes<FEN>) -> Result {
        let Self { callee, data } = self;
        expect_call(state, callee, data, None, None, None, None, 1, ExpectedCallType::NonCount)
    }
}

impl Cheatcode for expectCall_1Call {
    fn apply<FEN: FoundryEvmNetwork>(&self, state: &mut Cheatcodes<FEN>) -> Result {
        let Self { callee, data, count } = self;
        expect_call(state, callee, data, None, None, None, None, *count, ExpectedCallType::Count)
    }
}

impl Cheatcode for expectCall_2Call {
    fn apply<FEN: FoundryEvmNetwork>(&self, state: &mut Cheatcodes<FEN>) -> Result {
        let Self { callee, msgValue, data } = self;
        expect_call(
            state,
            callee,
            data,
            Some(msgValue),
            None,
            None,
            None,
            1,
            ExpectedCallType::NonCount,
        )
    }
}

impl Cheatcode for expectCall_3Call {
    fn apply<FEN: FoundryEvmNetwork>(&self, state: &mut Cheatcodes<FEN>) -> Result {
        let Self { callee, msgValue, data, count } = self;
        expect_call(
            state,
            callee,
            data,
            Some(msgValue),
            None,
            None,
            None,
            *count,
            ExpectedCallType::Count,
        )
    }
}

impl Cheatcode for expectCall_4Call {
    fn apply<FEN: FoundryEvmNetwork>(&self, state: &mut Cheatcodes<FEN>) -> Result {
        let Self { callee, msgValue, gas, data } = self;
        expect_call(
            state,
            callee,
            data,
            Some(msgValue),
            Some(*gas),
            None,
            None,
            1,
            ExpectedCallType::NonCount,
        )
    }
}

impl Cheatcode for expectCall_5Call {
    fn apply<FEN: FoundryEvmNetwork>(&self, state: &mut Cheatcodes<FEN>) -> Result {
        let Self { callee, msgValue, gas, data, count } = self;
        expect_call(
            state,
            callee,
            data,
            Some(msgValue),
            Some(*gas),
            None,
            None,
            *count,
            ExpectedCallType::Count,
        )
    }
}

impl Cheatcode for expectDelegateCallCall {
    fn apply<FEN: FoundryEvmNetwork>(&self, state: &mut Cheatcodes<FEN>) -> Result {
        let Self { callee, data } = self;
        expect_call(
            state,
            callee,
            data,
            None,
            None,
            None,
            Some(CallScheme::DelegateCall),
            1,
            ExpectedCallType::NonCount,
        )
    }
}

impl Cheatcode for expectCallMinGas_0Call {
    fn apply<FEN: FoundryEvmNetwork>(&self, state: &mut Cheatcodes<FEN>) -> Result {
        let Self { callee, msgValue, minGas, data } = self;
        expect_call(
            state,
            callee,
            data,
            Some(msgValue),
            None,
            Some(*minGas),
            None,
            1,
            ExpectedCallType::NonCount,
        )
    }
}

impl Cheatcode for expectCallMinGas_1Call {
    fn apply<FEN: FoundryEvmNetwork>(&self, state: &mut Cheatcodes<FEN>) -> Result {
        let Self { callee, msgValue, minGas, data, count } = self;
        expect_call(
            state,
            callee,
            data,
            Some(msgValue),
            None,
            Some(*minGas),
            None,
            *count,
            ExpectedCallType::Count,
        )
    }
}

impl Cheatcode for expectEmit_0Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { checkTopic1, checkTopic2, checkTopic3, checkData } = *self;
        expect_emit(
            ccx.state,
            ccx.depth(),
            [true, checkTopic1, checkTopic2, checkTopic3, checkData],
            None,
            false,
            1,
        )
    }
}

impl Cheatcode for expectEmit_1Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { checkTopic1, checkTopic2, checkTopic3, checkData, emitter } = *self;
        expect_emit(
            ccx.state,
            ccx.depth(),
            [true, checkTopic1, checkTopic2, checkTopic3, checkData],
            Some(emitter),
            false,
            1,
        )
    }
}

impl Cheatcode for expectEmit_2Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self {} = self;
        expect_emit(ccx.state, ccx.depth(), [true; 5], None, false, 1)
    }
}

impl Cheatcode for expectEmit_3Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { emitter } = *self;
        expect_emit(ccx.state, ccx.depth(), [true; 5], Some(emitter), false, 1)
    }
}

impl Cheatcode for expectEmit_4Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { checkTopic1, checkTopic2, checkTopic3, checkData, count } = *self;
        expect_emit(
            ccx.state,
            ccx.depth(),
            [true, checkTopic1, checkTopic2, checkTopic3, checkData],
            None,
            false,
            count,
        )
    }
}

impl Cheatcode for expectEmit_5Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { checkTopic1, checkTopic2, checkTopic3, checkData, emitter, count } = *self;
        expect_emit(
            ccx.state,
            ccx.depth(),
            [true, checkTopic1, checkTopic2, checkTopic3, checkData],
            Some(emitter),
            false,
            count,
        )
    }
}

impl Cheatcode for expectEmit_6Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { count } = *self;
        expect_emit(ccx.state, ccx.depth(), [true; 5], None, false, count)
    }
}

impl Cheatcode for expectEmit_7Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { emitter, count } = *self;
        expect_emit(ccx.state, ccx.depth(), [true; 5], Some(emitter), false, count)
    }
}

impl Cheatcode for expectEmitAnonymous_0Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { checkTopic0, checkTopic1, checkTopic2, checkTopic3, checkData } = *self;
        expect_emit(
            ccx.state,
            ccx.depth(),
            [checkTopic0, checkTopic1, checkTopic2, checkTopic3, checkData],
            None,
            true,
            1,
        )
    }
}

impl Cheatcode for expectEmitAnonymous_1Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { checkTopic0, checkTopic1, checkTopic2, checkTopic3, checkData, emitter } = *self;
        expect_emit(
            ccx.state,
            ccx.depth(),
            [checkTopic0, checkTopic1, checkTopic2, checkTopic3, checkData],
            Some(emitter),
            true,
            1,
        )
    }
}

impl Cheatcode for expectEmitAnonymous_2Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self {} = self;
        expect_emit(ccx.state, ccx.depth(), [true; 5], None, true, 1)
    }
}

impl Cheatcode for expectEmitAnonymous_3Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { emitter } = *self;
        expect_emit(ccx.state, ccx.depth(), [true; 5], Some(emitter), true, 1)
    }
}

impl Cheatcode for expectCreateCall {
    fn apply<FEN: FoundryEvmNetwork>(&self, state: &mut Cheatcodes<FEN>) -> Result {
        let Self { bytecode, deployer } = self;
        expect_create(state, bytecode.clone(), *deployer, CreateScheme::Create)
    }
}

impl Cheatcode for expectCreate2Call {
    fn apply<FEN: FoundryEvmNetwork>(&self, state: &mut Cheatcodes<FEN>) -> Result {
        let Self { bytecode, deployer } = self;
        expect_create(state, bytecode.clone(), *deployer, CreateScheme::Create2)
    }
}

impl Cheatcode for expectTip20LogoURIUpdatedCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { token, updater, newLogoURI } = self;
        expect_logo_uri_updated(ccx, token, updater, newLogoURI)
    }
}

impl Cheatcode for expectKeychainVerifiedCall {
    fn apply<FEN: FoundryEvmNetwork>(&self, state: &mut Cheatcodes<FEN>) -> Result {
        let Self { account, digest, signature } = self;
        expect_keychain_verified(state, *account, *digest, signature.clone(), false)
    }
}

impl Cheatcode for expectKeychainAdminVerifiedCall {
    fn apply<FEN: FoundryEvmNetwork>(&self, state: &mut Cheatcodes<FEN>) -> Result {
        let Self { account, digest, signature } = self;
        expect_keychain_verified(state, *account, *digest, signature.clone(), true)
    }
}

impl Cheatcode for expectLogoURIUpdatedCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { token, updater, newLogoURI } = self;
        expect_logo_uri_updated(ccx, token, updater, newLogoURI)
    }
}

fn expect_keychain_verified<FEN: FoundryEvmNetwork>(
    state: &mut Cheatcodes<FEN>,
    account: Address,
    digest: alloy_primitives::B256,
    signature: Bytes,
    admin: bool,
) -> Result {
    let calldata = if admin {
        ISignatureVerifier::verifyKeychainAdminCall { account, hash: digest, signature }
            .abi_encode()
    } else {
        ISignatureVerifier::verifyKeychainCall { account, hash: digest, signature }.abi_encode()
    };
    expect_call(
        state,
        &SIGNATURE_VERIFIER_ADDRESS,
        &Bytes::from(calldata),
        None,
        None,
        None,
        None,
        1,
        ExpectedCallType::NonCount,
    )
}

fn expect_logo_uri_updated<FEN: FoundryEvmNetwork>(
    ccx: &mut CheatsCtxt<'_, '_, FEN>,
    token: &Address,
    updater: &Address,
    new_logo_uri: &str,
) -> Result {
    let expected_emit = ExpectedEmit {
        depth: ccx.depth(),
        log: Some(RawLog::new_unchecked(
            vec![keccak256("LogoURIUpdated(address,string)"), updater.into_word()],
            new_logo_uri.abi_encode().into(),
        )),
        checks: [true, true, false, false, true],
        address: Some(*token),
        anonymous: false,
        found: false,
        count: 1,
        mismatch_error: None,
    };
    ccx.state.expected_emits.push_back((expected_emit, Default::default()));
    Ok(Default::default())
}

impl Cheatcode for expectRevert_0Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self {} = self;
        expect_revert(ccx.state, None, ccx.depth(), false, false, None, 1)
    }
}

impl Cheatcode for expectRevert_1Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { revertData } = self;
        expect_revert(ccx.state, Some(revertData.as_ref()), ccx.depth(), false, false, None, 1)
    }
}

impl Cheatcode for expectRevert_2Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { revertData } = self;
        expect_revert(ccx.state, Some(revertData), ccx.depth(), false, false, None, 1)
    }
}

impl Cheatcode for expectRevert_3Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { reverter } = self;
        expect_revert(ccx.state, None, ccx.depth(), false, false, Some(*reverter), 1)
    }
}

impl Cheatcode for expectRevert_4Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { revertData, reverter } = self;
        expect_revert(
            ccx.state,
            Some(revertData.as_ref()),
            ccx.depth(),
            false,
            false,
            Some(*reverter),
            1,
        )
    }
}

impl Cheatcode for expectRevert_5Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { revertData, reverter } = self;
        expect_revert(ccx.state, Some(revertData), ccx.depth(), false, false, Some(*reverter), 1)
    }
}

impl Cheatcode for expectRevert_6Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { count } = self;
        expect_revert(ccx.state, None, ccx.depth(), false, false, None, *count)
    }
}

impl Cheatcode for expectRevert_7Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { revertData, count } = self;
        expect_revert(ccx.state, Some(revertData.as_ref()), ccx.depth(), false, false, None, *count)
    }
}

impl Cheatcode for expectRevert_8Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { revertData, count } = self;
        expect_revert(ccx.state, Some(revertData), ccx.depth(), false, false, None, *count)
    }
}

impl Cheatcode for expectRevert_9Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { reverter, count } = self;
        expect_revert(ccx.state, None, ccx.depth(), false, false, Some(*reverter), *count)
    }
}

impl Cheatcode for expectRevert_10Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { revertData, reverter, count } = self;
        expect_revert(
            ccx.state,
            Some(revertData.as_ref()),
            ccx.depth(),
            false,
            false,
            Some(*reverter),
            *count,
        )
    }
}

impl Cheatcode for expectRevert_11Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { revertData, reverter, count } = self;
        expect_revert(
            ccx.state,
            Some(revertData),
            ccx.depth(),
            false,
            false,
            Some(*reverter),
            *count,
        )
    }
}

impl Cheatcode for expectPartialRevert_0Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { revertData } = self;
        expect_revert(ccx.state, Some(revertData.as_ref()), ccx.depth(), false, true, None, 1)
    }
}

impl Cheatcode for expectPartialRevert_1Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { revertData, reverter } = self;
        expect_revert(
            ccx.state,
            Some(revertData.as_ref()),
            ccx.depth(),
            false,
            true,
            Some(*reverter),
            1,
        )
    }
}

impl Cheatcode for _expectCheatcodeRevert_0Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        expect_revert(ccx.state, None, ccx.depth(), true, false, None, 1)
    }
}

impl Cheatcode for _expectCheatcodeRevert_1Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { revertData } = self;
        expect_revert(ccx.state, Some(revertData.as_ref()), ccx.depth(), true, false, None, 1)
    }
}

impl Cheatcode for _expectCheatcodeRevert_2Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { revertData } = self;
        expect_revert(ccx.state, Some(revertData), ccx.depth(), true, false, None, 1)
    }
}

impl Cheatcode for expectSafeMemoryCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { min, max } = *self;
        expect_safe_memory(ccx.state, min, max, ccx.depth().try_into()?)
    }
}

impl Cheatcode for stopExpectSafeMemoryCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self {} = self;
        ccx.state.allowed_mem_writes.remove(&ccx.depth().try_into()?);
        Ok(Default::default())
    }
}

impl Cheatcode for expectSafeMemoryCallCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { min, max } = *self;
        expect_safe_memory(ccx.state, min, max, (ccx.depth() + 1).try_into()?)
    }
}

impl RevertParameters for ExpectedRevert {
    fn reverter(&self) -> Option<Address> {
        self.reverter
    }

    fn reason(&self) -> Option<&[u8]> {
        self.reason.as_ref().map(|b| &***b)
    }

    fn partial_match(&self) -> bool {
        self.partial_match
    }
}

impl ExpectedRevert {
    /// Returns whether a call ending at `depth`, at or above the expectation's depth, consumes
    /// this expectation.
    ///
    /// With `internal_expect_revert` enabled, a same-depth revert can satisfy it, but it must not
    /// be consumed by external calls that succeed (e.g. calls to non-contract addresses that
    /// return `Stop` before Solidity's own revert).
    pub(crate) const fn needs_processing(
        &self,
        cheatcode_call: bool,
        call_failed: bool,
        depth: usize,
        internal_expect_revert: bool,
    ) -> bool {
        let went_deeper = self.max_depth > self.depth;
        match self.kind {
            ExpectedRevertKind::Default => {
                // Cheatcode reverts propagate up; let the outer frame catch them.
                if cheatcode_call {
                    return false;
                }
                // Any failure satisfies the expectation.
                if call_failed {
                    return true;
                }
                // Traditional expectRevert: succeeded external call went deeper.
                if !internal_expect_revert && went_deeper {
                    return true;
                }
                // Test function returned: catch dangling expectations.
                if depth == 0 {
                    return true;
                }
                // Same-depth success with internal mode off is an error; with it on,
                // keep waiting for the actual revert.
                !internal_expect_revert
            }
            // `pending_processing == true` means we're in the `call_end` hook for
            // `vm.expectCheatcodeRevert` and shouldn't expect a revert here.
            ExpectedRevertKind::Cheatcode { pending_processing } => {
                cheatcode_call && !pending_processing
            }
        }
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
#[expect(clippy::too_many_arguments)] // It is what it is
fn expect_call<FEN: FoundryEvmNetwork>(
    state: &mut Cheatcodes<FEN>,
    target: &Address,
    calldata: &Bytes,
    value: Option<&U256>,
    mut gas: Option<u64>,
    mut min_gas: Option<u64>,
    scheme: Option<CallScheme>,
    count: u64,
    call_type: ExpectedCallType,
) -> Result {
    let expecteds = state.expected_calls.entry(*target).or_default();

    if let Some(val) = value
        && *val > U256::ZERO
    {
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

    match call_type {
        ExpectedCallType::Count => {
            // Get the expected calls for this target.
            // In this case, as we're using counted expectCalls, we should not be able to set them
            // more than once.
            let key = (calldata.clone(), scheme);
            ensure!(!expecteds.contains_key(&key), "counted expected calls can only bet set once");
            expecteds.insert(
                key,
                (ExpectedCallData { value: value.copied(), gas, min_gas, count, call_type }, 0),
            );
        }
        ExpectedCallType::NonCount => {
            // Check if the expected calldata exists.
            // If it does, increment the count by one as we expect to see it one more time.
            match expecteds.entry((calldata.clone(), scheme)) {
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

/// Counts a call against every matching expectation registered for one target.
pub(crate) fn observe_call(
    expected_calls_for_target: &mut ExpectedCallsForTarget,
    input: &[u8],
    value: Option<U256>,
    gas_limit: u64,
    scheme: CallScheme,
) {
    // Match every partial/full calldata.
    for ((calldata, expected_scheme), (expected, actual_count)) in expected_calls_for_target {
        // Increment actual times seen if all of the following hold.
        // The calldata is at most as big as this call's input.
        if calldata.len() <= input.len() &&
            // Both calldata match, taking the length of the assumed smaller one (which will have at least the selector).
            input.get(..calldata.len()) == Some(calldata.as_ref()) &&
            // The value matches, if provided.
            expected.value.is_none_or(|expected_value| Some(expected_value) == value) &&
            // The gas matches, if provided.
            expected.gas.is_none_or(|gas| gas == gas_limit) &&
            // The minimum gas matches, if provided.
            expected.min_gas.is_none_or(|min_gas| min_gas <= gas_limit) &&
            // The call scheme matches, if provided.
            expected_scheme.is_none_or(|expected_scheme| expected_scheme == scheme)
        {
            *actual_count += 1;
        }
    }
}

/// Returns the failure message for the smallest unmet call expectation, if any.
///
/// Expectations are ordered by address, calldata and call scheme.
/// `succeeded` is false for both reverts and halts.
pub(crate) fn first_unmet_call(tracker: &ExpectedCallTracker, succeeded: bool) -> Option<String> {
    let (address, calldata, scheme, expected, actual_count) = tracker
        .iter()
        .flat_map(|(address, calldatas)| {
            calldatas.iter().map(move |((calldata, scheme), (expected, actual_count))| {
                (address, calldata, scheme, expected, actual_count)
            })
        })
        .filter(|(_, _, _, expected, actual_count)| match expected.call_type {
            // Counted expectations require exactly the requested number of calls.
            ExpectedCallType::Count => expected.count != **actual_count,
            // Non-counted expectations require at least the requested number of calls.
            ExpectedCallType::NonCount => expected.count > **actual_count,
        })
        .min_by_key(|(address, calldata, scheme, ..)| {
            (*address, *calldata, call_scheme_rank(**scheme))
        })?;

    let ExpectedCallData { gas, min_gas, value, count, .. } = expected;
    let expected_values = [
        Some(format!("data {}", hex::encode_prefixed(calldata))),
        value.as_ref().map(|v| format!("value {v}")),
        gas.map(|g| format!("gas {g}")),
        min_gas.map(|g| format!("minimum gas {g}")),
        scheme.map(|scheme| format!("call type {scheme:?}")),
    ]
    .into_iter()
    .flatten()
    .join(", ");
    let but = if succeeded {
        let s = if *actual_count == 1 { "" } else { "s" };
        format!("was called {actual_count} time{s}")
    } else {
        "the call reverted instead; \
         ensure you're testing the happy path when using `expectCall`"
            .to_string()
    };
    let s = if *count == 1 { "" } else { "s" };
    Some(format!(
        "expected call to {address} with {expected_values} \
         to be called {count} time{s}, but {but}"
    ))
}

const fn call_scheme_rank(scheme: Option<CallScheme>) -> u8 {
    match scheme {
        None => 0,
        Some(CallScheme::Call) => 1,
        Some(CallScheme::CallCode) => 2,
        Some(CallScheme::DelegateCall) => 3,
        Some(CallScheme::StaticCall) => 4,
    }
}

fn expect_emit<FEN: FoundryEvmNetwork>(
    state: &mut Cheatcodes<FEN>,
    depth: usize,
    checks: [bool; 5],
    address: Option<Address>,
    anonymous: bool,
    count: u64,
) -> Result {
    let expected_emit = ExpectedEmit {
        depth,
        checks,
        address,
        found: false,
        log: None,
        anonymous,
        count,
        mismatch_error: None,
    };
    if let Some(found_emit_pos) = state.expected_emits.iter().position(|(emit, _)| emit.found) {
        // The order of emits already found (back of queue) should not be modified, hence push any
        // new emit before first found emit.
        state.expected_emits.insert(found_emit_pos, (expected_emit, Default::default()));
    } else {
        // If no expected emits then push new one at the back of queue.
        state.expected_emits.push_back((expected_emit, Default::default()));
    }

    Ok(Default::default())
}

/// Applies an `expectEmit` failure for `log` by reverting the current call, when an interpreter
/// is available.
///
/// Without an interpreter, the failure reason is returned to the caller instead.
pub(crate) fn handle_expect_emit<FEN: FoundryEvmNetwork>(
    state: &mut Cheatcodes<FEN>,
    log: &Log,
    interpreter: Option<&mut Interpreter>,
) -> Option<&'static str> {
    let reason = observe_log(&mut state.expected_emits, log)?;
    let Some(interpreter) = interpreter else { return Some(reason) };
    interpreter.bytecode.set_action(InterpreterAction::new_return(
        InstructionResult::Revert,
        Error::encode(reason),
        interpreter.gas,
    ));
    None
}

fn expect_create<FEN: FoundryEvmNetwork>(
    state: &mut Cheatcodes<FEN>,
    bytecode: Bytes,
    deployer: Address,
    create_scheme: CreateScheme,
) -> Result {
    let expected_create = ExpectedCreate { bytecode, deployer, create_scheme };
    state.expected_creates.push(expected_create);

    Ok(Default::default())
}

fn expect_revert<FEN: FoundryEvmNetwork>(
    state: &mut Cheatcodes<FEN>,
    reason: Option<&[u8]>,
    depth: usize,
    cheatcode: bool,
    partial_match: bool,
    reverter: Option<Address>,
    count: u64,
) -> Result {
    ensure!(
        state.expected_revert.is_none(),
        "you must call another function prior to expecting a second revert"
    );
    state.expected_revert = Some(ExpectedRevert {
        reason: reason.map(Bytes::copy_from_slice),
        depth,
        kind: if cheatcode {
            ExpectedRevertKind::Cheatcode { pending_processing: true }
        } else {
            ExpectedRevertKind::Default
        },
        partial_match,
        reverter,
        reverted_by: None,
        max_depth: depth,
        count,
        actual_count: 0,
    });
    Ok(Default::default())
}

fn expect_safe_memory<FEN: FoundryEvmNetwork>(
    state: &mut Cheatcodes<FEN>,
    start: u64,
    end: u64,
    depth: u64,
) -> Result {
    ensure!(start < end, "memory range start ({start}) is greater than end ({end})");
    #[expect(clippy::single_range_in_vec_init)] // Wanted behaviour
    let offsets = state.allowed_mem_writes.entry(depth).or_insert_with(|| vec![0..0x60]);
    offsets.push(start..end);
    Ok(Default::default())
}

/// Removes the first create expectation matched by a completed create.
///
/// `create_scheme` is only called for expectations with a matching deployer.
pub(crate) fn observe_create(
    expected_creates: &mut Vec<ExpectedCreate>,
    deployer: Address,
    create_scheme: impl Fn() -> CreateScheme,
    bytecode: &Bytes,
) {
    if let Some((index, _)) = expected_creates.iter().find_position(|expected_create| {
        expected_create.deployer == deployer
            && expected_create.create_scheme.eq(create_scheme())
            && expected_create.bytecode == *bytecode
    }) {
        expected_creates.swap_remove(index);
    }
}

/// Returns the failure message for the first unmet create expectation, if any.
pub(crate) fn first_unmet_create(expected_creates: &[ExpectedCreate]) -> Option<String> {
    let expected_create = expected_creates.first()?;
    Some(format!(
        "expected {} call by address {} for bytecode {} but not found",
        expected_create.create_scheme,
        hex::encode_prefixed(expected_create.deployer),
        hex::encode_prefixed(&expected_create.bytecode),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{address, bytes};

    const TARGET: Address = address!("0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f");

    fn expected(
        value: Option<U256>,
        gas: Option<u64>,
        min_gas: Option<u64>,
        count: u64,
        call_type: ExpectedCallType,
    ) -> ExpectedCallData {
        ExpectedCallData { value, gas, min_gas, count, call_type }
    }

    fn tracker(
        calldata: Bytes,
        scheme: Option<CallScheme>,
        data: ExpectedCallData,
    ) -> ExpectedCallTracker {
        let mut tracker = ExpectedCallTracker::default();
        tracker.entry(TARGET).or_default().insert((calldata, scheme), (data, 0));
        tracker
    }

    fn seen(tracker: &ExpectedCallTracker, calldata: &Bytes, scheme: Option<CallScheme>) -> u64 {
        tracker[&TARGET][&(calldata.clone(), scheme)].1
    }

    fn observe(tracker: &mut ExpectedCallTracker, input: &[u8], value: Option<U256>, gas: u64) {
        observe_call(tracker.get_mut(&TARGET).unwrap(), input, value, gas, CallScheme::Call);
    }

    #[test]
    fn observe_call_matches_calldata_prefix() {
        let selector = bytes!("771602f7");
        let full = bytes!("771602f7aabb");
        let mut t = tracker(
            selector.clone(),
            None,
            expected(None, None, None, 1, ExpectedCallType::NonCount),
        );
        t.entry(TARGET).or_default().insert(
            (full.clone(), None),
            (expected(None, None, None, 1, ExpectedCallType::NonCount), 0),
        );

        observe(&mut t, &full, None, 0);
        observe(&mut t, &selector, None, 0);
        observe(&mut t, &bytes!("12345678"), None, 0);

        assert_eq!(seen(&t, &selector, None), 2);
        assert_eq!(seen(&t, &full, None), 1);
    }

    #[test]
    fn observe_call_filters_value() {
        let calldata = bytes!("c290d691");
        let mut t = tracker(
            calldata.clone(),
            None,
            expected(Some(U256::from(1)), None, None, 1, ExpectedCallType::NonCount),
        );

        observe(&mut t, &calldata, Some(U256::from(2)), 0);
        observe(&mut t, &calldata, None, 0);
        assert_eq!(seen(&t, &calldata, None), 0);

        observe(&mut t, &calldata, Some(U256::from(1)), 0);
        assert_eq!(seen(&t, &calldata, None), 1);

        // An expected zero value does not match a call without a transfer value.
        let mut zero = tracker(
            calldata.clone(),
            None,
            expected(Some(U256::ZERO), None, None, 1, ExpectedCallType::NonCount),
        );
        observe(&mut zero, &calldata, None, 0);
        assert_eq!(seen(&zero, &calldata, None), 0);
        observe(&mut zero, &calldata, Some(U256::ZERO), 0);
        assert_eq!(seen(&zero, &calldata, None), 1);

        // No expected value matches any transfer value.
        let mut any = tracker(
            calldata.clone(),
            None,
            expected(None, None, None, 1, ExpectedCallType::NonCount),
        );
        observe(&mut any, &calldata, Some(U256::from(5)), 0);
        assert_eq!(seen(&any, &calldata, None), 1);
    }

    #[test]
    fn observe_call_filters_gas_and_min_gas() {
        let calldata = bytes!("771602f7");
        let mut gas = tracker(
            calldata.clone(),
            None,
            expected(None, Some(25_000), None, 1, ExpectedCallType::NonCount),
        );
        observe(&mut gas, &calldata, None, 24_999);
        observe(&mut gas, &calldata, None, 25_001);
        assert_eq!(seen(&gas, &calldata, None), 0);
        observe(&mut gas, &calldata, None, 25_000);
        assert_eq!(seen(&gas, &calldata, None), 1);

        let mut min_gas = tracker(
            calldata.clone(),
            None,
            expected(None, None, Some(50_000), 1, ExpectedCallType::NonCount),
        );
        observe(&mut min_gas, &calldata, None, 49_999);
        assert_eq!(seen(&min_gas, &calldata, None), 0);
        observe(&mut min_gas, &calldata, None, 50_000);
        observe(&mut min_gas, &calldata, None, 60_000);
        assert_eq!(seen(&min_gas, &calldata, None), 2);
    }

    #[test]
    fn observe_call_filters_scheme() {
        let calldata = bytes!("771602f7");
        let mut t = tracker(
            calldata.clone(),
            Some(CallScheme::DelegateCall),
            expected(None, None, None, 1, ExpectedCallType::NonCount),
        );

        observe_call(t.get_mut(&TARGET).unwrap(), &calldata, None, 0, CallScheme::Call);
        assert_eq!(seen(&t, &calldata, Some(CallScheme::DelegateCall)), 0);

        observe_call(t.get_mut(&TARGET).unwrap(), &calldata, None, 0, CallScheme::DelegateCall);
        assert_eq!(seen(&t, &calldata, Some(CallScheme::DelegateCall)), 1);
    }

    #[test]
    fn first_unmet_call_compares_counts() {
        let calldata = bytes!("771602f7");
        let mut count =
            tracker(calldata.clone(), None, expected(None, None, None, 2, ExpectedCallType::Count));
        let mut non_count =
            tracker(calldata, None, expected(None, None, None, 2, ExpectedCallType::NonCount));

        for seen in [1, 3] {
            count.get_mut(&TARGET).unwrap().values_mut().next().unwrap().1 = seen;
            assert!(first_unmet_call(&count, true).is_some(), "count with {seen} calls");
        }
        count.get_mut(&TARGET).unwrap().values_mut().next().unwrap().1 = 2;
        assert_eq!(first_unmet_call(&count, true), None);

        non_count.get_mut(&TARGET).unwrap().values_mut().next().unwrap().1 = 1;
        assert!(first_unmet_call(&non_count, true).is_some());
        for seen in [2, 3] {
            non_count.get_mut(&TARGET).unwrap().values_mut().next().unwrap().1 = seen;
            assert_eq!(first_unmet_call(&non_count, true), None, "non-count with {seen} calls");
        }
    }

    #[test]
    fn first_unmet_call_reports_smallest_unmet_key() {
        let low = address!("0000000000000000000000000000000000000001");
        let high = address!("0000000000000000000000000000000000000002");
        let ordered = [
            (low, bytes!("01"), None, ""),
            (low, bytes!("01"), Some(CallScheme::Call), ", call type Call"),
            (low, bytes!("01"), Some(CallScheme::CallCode), ", call type CallCode"),
            (low, bytes!("01"), Some(CallScheme::DelegateCall), ", call type DelegateCall"),
            (low, bytes!("01"), Some(CallScheme::StaticCall), ", call type StaticCall"),
            (low, bytes!("02"), None, ""),
            (high, bytes!("00"), None, ""),
            (high, bytes!("01"), None, ""),
        ];

        for offset in 0..ordered.len() {
            let mut t = ExpectedCallTracker::default();
            // A satisfied expectation must not hide a later failure.
            t.entry(Address::ZERO).or_default().insert(
                (Bytes::new(), None),
                (expected(None, None, None, 1, ExpectedCallType::NonCount), 1),
            );
            for index in (0..ordered.len()).rev() {
                let (address, calldata, scheme, _) = &ordered[(index + offset) % ordered.len()];
                t.entry(*address).or_default().insert(
                    (calldata.clone(), *scheme),
                    (expected(None, None, None, 1, ExpectedCallType::NonCount), 0),
                );
            }

            for (address, calldata, scheme, suffix) in &ordered {
                assert_eq!(
                    first_unmet_call(&t, true).unwrap(),
                    format!(
                        "expected call to {address} with data {}{suffix} to be called 1 time, \
                         but was called 0 times",
                        hex::encode_prefixed(calldata),
                    ),
                );
                t.get_mut(address).unwrap().remove(&(calldata.clone(), *scheme));
            }
            assert_eq!(first_unmet_call(&t, true), None);
        }
    }

    #[test]
    fn first_unmet_call_message() {
        let calldata = bytes!("771602f7");
        let mut t = tracker(
            calldata.clone(),
            Some(CallScheme::DelegateCall),
            expected(Some(U256::from(1)), Some(2), Some(3), 2, ExpectedCallType::Count),
        );
        t.get_mut(&TARGET).unwrap().values_mut().next().unwrap().1 = 1;

        assert_eq!(
            first_unmet_call(&t, true).unwrap(),
            "expected call to 0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f with data 0x771602f7, \
             value 1, gas 2, minimum gas 3, call type DelegateCall to be called 2 times, \
             but was called 1 time"
        );
        assert_eq!(
            first_unmet_call(&t, false).unwrap(),
            "expected call to 0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f with data 0x771602f7, \
             value 1, gas 2, minimum gas 3, call type DelegateCall to be called 2 times, \
             but the call reverted instead; ensure you're testing the happy path when using \
             `expectCall`"
        );

        let single =
            tracker(calldata, None, expected(None, None, None, 1, ExpectedCallType::NonCount));
        assert_eq!(
            first_unmet_call(&single, true).unwrap(),
            "expected call to 0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f with data 0x771602f7 \
             to be called 1 time, but was called 0 times"
        );
    }

    #[test]
    fn observe_create_converts_the_scheme_only_for_a_matching_deployer() {
        let bytecode = bytes!("6080");
        let mut expected_creates = vec![ExpectedCreate {
            deployer: Address::ZERO,
            bytecode: bytecode.clone(),
            create_scheme: CreateScheme::Create,
        }];

        observe_create(&mut expected_creates, TARGET, || unreachable!(), &bytecode);
        assert_eq!(expected_creates.len(), 1);

        observe_create(&mut expected_creates, Address::ZERO, || CreateScheme::Create, &bytecode);
        assert!(expected_creates.is_empty());
    }

    #[test]
    fn internal_expect_revert_waits_for_failure_before_root() {
        let expected_revert = ExpectedRevert {
            reason: None,
            depth: 1,
            kind: ExpectedRevertKind::Default,
            partial_match: false,
            reverter: None,
            reverted_by: None,
            max_depth: 1,
            count: 1,
            actual_count: 0,
        };

        assert!(expected_revert.needs_processing(false, true, 1, true));
        assert!(!expected_revert.needs_processing(false, false, 1, true));
    }
}
