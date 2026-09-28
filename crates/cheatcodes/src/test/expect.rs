use std::fmt::{self, Display};

use crate::{
    Cheatcode, Cheatcodes, CheatsCtxt, Error, Result,
    Vm::*,
    expected_call::{self, ExpectedCallKind, ExpectedCallType},
    expected_emit::{self, EmitMismatch, ExpectedEmit},
};
use alloy_dyn_abi::{DynSolValue, EventExt};
use alloy_json_abi::Event;
use alloy_primitives::{Address, Bytes, LogData as RawLog, U256, keccak256};
use alloy_sol_types::{SolCall, SolValue};
use foundry_common::{abi::get_indexed_event, fmt::format_token};
use foundry_evm_core::evm::FoundryEvmNetwork;
use foundry_evm_traces::DecodedCallLog;
use revm::{
    context::{ContextTr, JournalTr},
    interpreter::interpreter_types::LoopControl,
};
use tempo_contracts::precompiles::ISignatureVerifier;
use tempo_precompiles::SIGNATURE_VERIFIER_ADDRESS;

use super::revert_handlers::RevertParameters;

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

impl EmitMismatch {
    pub fn to_error_msg<FEN: FoundryEvmNetwork>(
        &self,
        state: &Cheatcodes<FEN>,
        checks: [bool; 5],
        expected: Option<&RawLog>,
        anonymous: bool,
    ) -> String {
        match self {
            Self::Log { actual } => {
                let Some(expected) = expected else {
                    return "log != expected log".to_string();
                };
                let (expected_decoded, actual_decoded) = if anonymous {
                    (None, None)
                } else {
                    state
                        .signatures_identifier()
                        .map(|identifier| {
                            (decode_event(identifier, expected), decode_event(identifier, actual))
                        })
                        .unwrap_or_default()
                };
                expected_emit::get_emit_mismatch_message(
                    checks,
                    expected,
                    actual,
                    anonymous,
                    expected_decoded.as_ref(),
                    actual_decoded.as_ref(),
                )
            }
            Self::Emitter { expected, actual } => {
                format!("log emitter mismatch: expected={expected:#x}, got={actual:#x}")
            }
        }
    }
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
            Some(ExpectedCallKind::DelegateCall),
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
            ccx.ecx.journal().depth(),
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
            ccx.ecx.journal().depth(),
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
        expect_emit(ccx.state, ccx.ecx.journal().depth(), [true; 5], None, false, 1)
    }
}

impl Cheatcode for expectEmit_3Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { emitter } = *self;
        expect_emit(ccx.state, ccx.ecx.journal().depth(), [true; 5], Some(emitter), false, 1)
    }
}

impl Cheatcode for expectEmit_4Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { checkTopic1, checkTopic2, checkTopic3, checkData, count } = *self;
        expect_emit(
            ccx.state,
            ccx.ecx.journal().depth(),
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
            ccx.ecx.journal().depth(),
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
        expect_emit(ccx.state, ccx.ecx.journal().depth(), [true; 5], None, false, count)
    }
}

impl Cheatcode for expectEmit_7Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { emitter, count } = *self;
        expect_emit(ccx.state, ccx.ecx.journal().depth(), [true; 5], Some(emitter), false, count)
    }
}

impl Cheatcode for expectEmitAnonymous_0Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { checkTopic0, checkTopic1, checkTopic2, checkTopic3, checkData } = *self;
        expect_emit(
            ccx.state,
            ccx.ecx.journal().depth(),
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
            ccx.ecx.journal().depth(),
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
        expect_emit(ccx.state, ccx.ecx.journal().depth(), [true; 5], None, true, 1)
    }
}

impl Cheatcode for expectEmitAnonymous_3Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { emitter } = *self;
        expect_emit(ccx.state, ccx.ecx.journal().depth(), [true; 5], Some(emitter), true, 1)
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
        depth: ccx.ecx.journal().depth(),
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
        expect_revert(ccx.state, None, ccx.ecx.journal().depth(), false, false, None, 1)
    }
}

impl Cheatcode for expectRevert_1Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { revertData } = self;
        expect_revert(
            ccx.state,
            Some(revertData.as_ref()),
            ccx.ecx.journal().depth(),
            false,
            false,
            None,
            1,
        )
    }
}

impl Cheatcode for expectRevert_2Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { revertData } = self;
        expect_revert(ccx.state, Some(revertData), ccx.ecx.journal().depth(), false, false, None, 1)
    }
}

impl Cheatcode for expectRevert_3Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { reverter } = self;
        expect_revert(ccx.state, None, ccx.ecx.journal().depth(), false, false, Some(*reverter), 1)
    }
}

impl Cheatcode for expectRevert_4Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { revertData, reverter } = self;
        expect_revert(
            ccx.state,
            Some(revertData.as_ref()),
            ccx.ecx.journal().depth(),
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
        expect_revert(
            ccx.state,
            Some(revertData),
            ccx.ecx.journal().depth(),
            false,
            false,
            Some(*reverter),
            1,
        )
    }
}

impl Cheatcode for expectRevert_6Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { count } = self;
        expect_revert(ccx.state, None, ccx.ecx.journal().depth(), false, false, None, *count)
    }
}

impl Cheatcode for expectRevert_7Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { revertData, count } = self;
        expect_revert(
            ccx.state,
            Some(revertData.as_ref()),
            ccx.ecx.journal().depth(),
            false,
            false,
            None,
            *count,
        )
    }
}

impl Cheatcode for expectRevert_8Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { revertData, count } = self;
        expect_revert(
            ccx.state,
            Some(revertData),
            ccx.ecx.journal().depth(),
            false,
            false,
            None,
            *count,
        )
    }
}

impl Cheatcode for expectRevert_9Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { reverter, count } = self;
        expect_revert(
            ccx.state,
            None,
            ccx.ecx.journal().depth(),
            false,
            false,
            Some(*reverter),
            *count,
        )
    }
}

impl Cheatcode for expectRevert_10Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { revertData, reverter, count } = self;
        expect_revert(
            ccx.state,
            Some(revertData.as_ref()),
            ccx.ecx.journal().depth(),
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
            ccx.ecx.journal().depth(),
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
        expect_revert(
            ccx.state,
            Some(revertData.as_ref()),
            ccx.ecx.journal().depth(),
            false,
            true,
            None,
            1,
        )
    }
}

impl Cheatcode for expectPartialRevert_1Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { revertData, reverter } = self;
        expect_revert(
            ccx.state,
            Some(revertData.as_ref()),
            ccx.ecx.journal().depth(),
            false,
            true,
            Some(*reverter),
            1,
        )
    }
}

impl Cheatcode for _expectCheatcodeRevert_0Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        expect_revert(ccx.state, None, ccx.ecx.journal().depth(), true, false, None, 1)
    }
}

impl Cheatcode for _expectCheatcodeRevert_1Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { revertData } = self;
        expect_revert(
            ccx.state,
            Some(revertData.as_ref()),
            ccx.ecx.journal().depth(),
            true,
            false,
            None,
            1,
        )
    }
}

impl Cheatcode for _expectCheatcodeRevert_2Call {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { revertData } = self;
        expect_revert(ccx.state, Some(revertData), ccx.ecx.journal().depth(), true, false, None, 1)
    }
}

impl Cheatcode for expectSafeMemoryCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { min, max } = *self;
        expect_safe_memory(ccx.state, min, max, ccx.ecx.journal().depth().try_into()?)
    }
}

impl Cheatcode for stopExpectSafeMemoryCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self {} = self;
        ccx.state.allowed_mem_writes.remove(&ccx.ecx.journal().depth().try_into()?);
        Ok(Default::default())
    }
}

impl Cheatcode for expectSafeMemoryCallCall {
    fn apply_stateful<FEN: FoundryEvmNetwork>(&self, ccx: &mut CheatsCtxt<'_, '_, FEN>) -> Result {
        let Self { min, max } = *self;
        expect_safe_memory(ccx.state, min, max, (ccx.ecx.journal().depth() + 1).try_into()?)
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
    gas: Option<u64>,
    min_gas: Option<u64>,
    scheme: Option<ExpectedCallKind>,
    count: u64,
    call_type: ExpectedCallType,
) -> Result {
    expected_call::expect_call(
        &mut state.expected_calls,
        *target,
        calldata.clone(),
        value.copied(),
        gas,
        min_gas,
        scheme,
        count,
        call_type,
    )?;
    Ok(Default::default())
}

fn expect_emit<FEN: FoundryEvmNetwork>(
    state: &mut Cheatcodes<FEN>,
    depth: usize,
    checks: [bool; 5],
    address: Option<Address>,
    anonymous: bool,
    count: u64,
) -> Result {
    expected_emit::register(&mut state.expected_emits, depth, checks, address, anonymous, count);
    Ok(Default::default())
}

pub(crate) fn handle_expect_emit<FEN: FoundryEvmNetwork>(
    state: &mut Cheatcodes<FEN>,
    log: &alloy_primitives::Log,
    interpreter: Option<&mut revm::interpreter::Interpreter>,
) -> Option<&'static str> {
    let failure = expected_emit::observe(&mut state.expected_emits, log);
    if let (Some(failure), Some(interpreter)) = (failure, interpreter) {
        interpreter.bytecode.set_action(revm::interpreter::InterpreterAction::new_return(
            revm::interpreter::InstructionResult::Revert,
            Error::encode(failure),
            interpreter.gas,
        ));
        None
    } else {
        failure
    }
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

fn decode_event(
    identifier: &foundry_evm_traces::identifier::SignaturesIdentifier,
    log: &RawLog,
) -> Option<DecodedCallLog> {
    let topics = log.topics();
    if topics.is_empty() {
        return None;
    }
    let t0 = topics[0]; // event sig
    // Try to identify the event
    let event = foundry_common::block_on(
        identifier.identify_event_with_indexed_count(t0, topics.len().saturating_sub(1)),
    )?;

    // Check if event already has indexed information from signatures
    let has_indexed_info = event.inputs.iter().any(|p| p.indexed);
    // Only use get_indexed_event if the event doesn't have indexing info
    let indexed_event = if has_indexed_info { event } else { get_indexed_event(event, log) };

    // Try to decode the event
    if let Ok(decoded) = indexed_event.decode_log(log) {
        let params = reconstruct_params(&indexed_event, &decoded);

        let decoded_params = params
            .into_iter()
            .zip(indexed_event.inputs.iter())
            .map(|(param, input)| (input.name.clone(), format_token(&param)))
            .collect();

        return Some(DecodedCallLog {
            name: Some(indexed_event.name),
            params: Some(decoded_params),
        });
    }

    None
}

/// Restore the order of the params of a decoded event
fn reconstruct_params(event: &Event, decoded: &alloy_dyn_abi::DecodedEvent) -> Vec<DynSolValue> {
    let mut indexed = 0;
    let mut unindexed = 0;
    let mut inputs = vec![];
    for input in &event.inputs {
        if input.indexed && indexed < decoded.indexed.len() {
            inputs.push(decoded.indexed[indexed].clone());
            indexed += 1;
        } else if unindexed < decoded.body.len() {
            inputs.push(decoded.body[unindexed].clone());
            unindexed += 1;
        }
    }
    inputs
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
