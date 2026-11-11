use crate::{CheatcodesExecutor, CheatsCtxt, Result, Vm::*};
use alloy_primitives::{Address, B256, Bytes, I256, U256, U512, keccak256, uint};
use alloy_sol_types::SolCall;
use evm2::interpreter::{InstrStop, op, opcode::OpCode};
use foundry_evm_core::{
    abi::console::{format_units_int, format_units_uint},
    backend::GLOBAL_FAIL_SLOT,
    constants::CHEATCODE_ADDRESS,
    decode::ASSERTION_FAILED_PREFIX,
    evm::{FoundryEvmNetwork, InterpreterFor, MessageResultFor},
};
use itertools::Itertools;
use std::{borrow::Cow, fmt};

const EQ_REL_DELTA_RESOLUTION: U256 = uint!(18_U256);

struct ComparisonAssertionError<'a, T> {
    kind: AssertionKind,
    left: &'a T,
    right: &'a T,
}

#[derive(Clone, Copy)]
enum AssertionKind {
    Eq,
    Ne,
    Gt,
    Ge,
    Lt,
    Le,
}

impl AssertionKind {
    const fn inverse(self) -> Self {
        match self {
            Self::Eq => Self::Ne,
            Self::Ne => Self::Eq,
            Self::Gt => Self::Le,
            Self::Ge => Self::Lt,
            Self::Lt => Self::Ge,
            Self::Le => Self::Gt,
        }
    }

    const fn to_str(self) -> &'static str {
        match self {
            Self::Eq => "==",
            Self::Ne => "!=",
            Self::Gt => ">",
            Self::Ge => ">=",
            Self::Lt => "<",
            Self::Le => "<=",
        }
    }
}

impl<T> ComparisonAssertionError<'_, T> {
    fn format_values<D: fmt::Display>(&self, f: impl Fn(&T) -> D) -> String {
        format!("{} {} {}", f(self.left), self.kind.inverse().to_str(), f(self.right))
    }
}

impl<T: fmt::Display> ComparisonAssertionError<'_, T> {
    fn format_for_values(&self) -> String {
        self.format_values(T::to_string)
    }
}

impl<T: fmt::Display> ComparisonAssertionError<'_, Vec<T>> {
    fn format_for_arrays(&self) -> String {
        self.format_values(|v| format!("[{}]", v.iter().format(", ")))
    }
}

impl ComparisonAssertionError<'_, U256> {
    fn format_with_decimals(&self, decimals: &U256) -> String {
        self.format_values(|v| format_units_uint(v, decimals))
    }
}

impl ComparisonAssertionError<'_, I256> {
    fn format_with_decimals(&self, decimals: &U256) -> String {
        self.format_values(|v| format_units_int(v, decimals))
    }
}

#[derive(thiserror::Error, Debug)]
#[error("{left} !~= {right} (max delta: {max_delta}, real delta: {real_delta})")]
struct EqAbsAssertionError<T, D> {
    left: T,
    right: T,
    max_delta: D,
    real_delta: D,
}

impl EqAbsAssertionError<U256, U256> {
    fn format_with_decimals(&self, decimals: &U256) -> String {
        format!(
            "{} !~= {} (max delta: {}, real delta: {})",
            format_units_uint(&self.left, decimals),
            format_units_uint(&self.right, decimals),
            format_units_uint(&self.max_delta, decimals),
            format_units_uint(&self.real_delta, decimals),
        )
    }
}

impl EqAbsAssertionError<I256, U256> {
    fn format_with_decimals(&self, decimals: &U256) -> String {
        format!(
            "{} !~= {} (max delta: {}, real delta: {})",
            format_units_int(&self.left, decimals),
            format_units_int(&self.right, decimals),
            format_units_uint(&self.max_delta, decimals),
            format_units_uint(&self.real_delta, decimals),
        )
    }
}

fn format_delta_percent(delta: &U256) -> String {
    format!("{}%", format_units_uint(delta, &(EQ_REL_DELTA_RESOLUTION - U256::from(2))))
}

#[derive(Debug)]
enum EqRelDelta {
    Defined(U256),
    Undefined,
}

impl fmt::Display for EqRelDelta {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Defined(delta) => write!(f, "{}", format_delta_percent(delta)),
            Self::Undefined => write!(f, "undefined"),
        }
    }
}

#[derive(thiserror::Error, Debug)]
#[error(
    "{left} !~= {right} (max delta: {}, real delta: {})",
    format_delta_percent(max_delta),
    real_delta
)]
struct EqRelAssertionFailure<T> {
    left: T,
    right: T,
    max_delta: U256,
    real_delta: EqRelDelta,
}

#[derive(thiserror::Error, Debug)]
enum EqRelAssertionError<T> {
    #[error(transparent)]
    Failure(Box<EqRelAssertionFailure<T>>),
    #[error("overflow in delta calculation")]
    Overflow,
}

impl EqRelAssertionError<U256> {
    fn format_with_decimals(&self, decimals: &U256) -> String {
        match self {
            Self::Failure(f) => format!(
                "{} !~= {} (max delta: {}, real delta: {})",
                format_units_uint(&f.left, decimals),
                format_units_uint(&f.right, decimals),
                format_delta_percent(&f.max_delta),
                f.real_delta,
            ),
            Self::Overflow => self.to_string(),
        }
    }
}

impl EqRelAssertionError<I256> {
    fn format_with_decimals(&self, decimals: &U256) -> String {
        match self {
            Self::Failure(f) => format!(
                "{} !~= {} (max delta: {}, real delta: {})",
                format_units_int(&f.left, decimals),
                format_units_int(&f.right, decimals),
                format_delta_percent(&f.max_delta),
                f.real_delta,
            ),
            Self::Overflow => self.to_string(),
        }
    }
}

type ComparisonResult<'a, T> = Result<(), ComparisonAssertionError<'a, T>>;

#[cold]
fn handle_assertion_result<FEN: FoundryEvmNetwork, E>(
    ccx: &mut CheatsCtxt<'_, '_, FEN>,
    executor: &mut dyn CheatcodesExecutor<FEN>,
    selector: [u8; 4],
    err: E,
    error_formatter: Option<&dyn Fn(&E) -> String>,
    error_msg: Option<&str>,
) -> Result {
    let error_msg = error_msg.unwrap_or(ASSERTION_FAILED_PREFIX);
    let msg = if let Some(error_formatter) = error_formatter {
        Cow::Owned(format!("{error_msg}: {}", error_formatter(&err)))
    } else {
        Cow::Borrowed(error_msg)
    };
    let reverts = ccx.state.core.config.assertions_revert;
    if let Some(trace) = &mut ccx.state.failure_trace {
        let revert = reverts.then(|| keccak256(crate::Error::encode(msg.to_string())));
        trace.record_assertion(selector, error_msg, revert);
    }
    if let Some(provenance) = &mut ccx.state.failure_provenance {
        provenance.pending_assertion = Some(keccak256([&selector[..], msg.as_bytes()].concat()));
    }
    handle_assertion_result_mono(ccx, executor, msg)
}

fn handle_assertion_result_mono<FEN: FoundryEvmNetwork>(
    ccx: &mut CheatsCtxt<'_, '_, FEN>,
    executor: &mut dyn CheatcodesExecutor<FEN>,
    msg: Cow<'_, str>,
) -> Result {
    ccx.state.failed_assertion.get_or_insert_with(|| msg.to_string());
    if ccx.state.core.config.assertions_revert {
        Err(msg.into_owned().into())
    } else {
        executor.console_log(&msg);
        ccx.evm.state_mut().storage_slot(&CHEATCODE_ADDRESS, GLOBAL_FAIL_SLOT)?.set(U256::ONE);
        Ok(Default::default())
    }
}

/// Implements [crate::Cheatcode] for pairs of cheatcodes.
///
/// Accepts a list of pairs of cheatcodes, where the first cheatcode is the one that doesn't contain
/// a custom error message, and the second one contains it at `err` field.
///
/// Passed `args` are the common arguments for both cheatcode structs (excluding `err` field).
///
/// Macro also accepts an optional closure that formats the error returned by the assertion.
macro_rules! impl_assertions {
    (|$($arg:ident),*| $body:expr, false, $(($no_error:ident, $with_error:ident)),* $(,)?) => {
        impl_assertions! { @args_tt |($($arg),*)| $body, None, $(($no_error, $with_error)),* }
    };
    (|$($arg:ident),*| $body:expr, $(($no_error:ident, $with_error:ident)),* $(,)?) => {
        impl_assertions! { @args_tt |($($arg),*)| $body, Some(&ToString::to_string), $(($no_error, $with_error)),* }
    };
    (|$($arg:ident),*| $body:expr, $error_formatter:expr, $(($no_error:ident, $with_error:ident)),* $(,)?) => {
        impl_assertions! { @args_tt |($($arg),*)| $body, Some(&$error_formatter), $(($no_error, $with_error)),* }
    };

    // We convert args to `tt` and later expand them back into tuple to allow usage of expanded args inside of
    // each assertion type context.
    (@args_tt |$args:tt| $body:expr, $error_formatter:expr, $(($no_error:ident, $with_error:ident)),* $(,)?) => {
        $(
            impl_assertions! { @impl $no_error, $with_error, $args, $body, $error_formatter }
        )*
    };

    (@impl $no_error:ident, $with_error:ident, ($($arg:ident),*), $body:expr, $error_formatter:expr) => {
        impl crate::Cheatcode for $no_error {
            fn apply_full<FEN: FoundryEvmNetwork>(
                &self,
                ccx: &mut CheatsCtxt<'_, '_, FEN>,
                executor: &mut dyn CheatcodesExecutor<FEN>,
            ) -> Result {
                let Self { $($arg),* } = self;
                match $body {
                    Ok(()) => Ok(Default::default()),
                    Err(err) => handle_assertion_result(
                        ccx,
                        executor,
                        Self::SELECTOR,
                        err,
                        $error_formatter,
                        None,
                    ),
                }
            }
        }

        impl crate::Cheatcode for $with_error {
            fn apply_full<FEN: FoundryEvmNetwork>(
                &self,
                ccx: &mut CheatsCtxt<'_, '_, FEN>,
                executor: &mut dyn CheatcodesExecutor<FEN>,
            ) -> Result {
                let Self { $($arg,)* err } = self;
                match $body {
                    Ok(()) => Ok(Default::default()),
                    Err(assertion_err) => handle_assertion_result(
                        ccx,
                        executor,
                        Self::SELECTOR,
                        assertion_err,
                        $error_formatter,
                        Some(err),
                    ),
                }
            }
        }
    };
}

impl_assertions! {
    |condition| assert_true(*condition),
    false,
    (assertTrue_0Call, assertTrue_1Call),
}

impl_assertions! {
    |condition| assert_false(*condition),
    false,
    (assertFalse_0Call, assertFalse_1Call),
}

impl_assertions! {
    |left, right| assert_eq(left, right),
    ComparisonAssertionError::format_for_values,
    (assertEq_0Call, assertEq_1Call),
    (assertEq_2Call, assertEq_3Call),
    (assertEq_4Call, assertEq_5Call),
    (assertEq_6Call, assertEq_7Call),
    (assertEq_8Call, assertEq_9Call),
    (assertEq_10Call, assertEq_11Call),
    (assertEq_12Call, assertEq_13Call),
}

impl_assertions! {
    |left, right| assert_eq(left, right),
    ComparisonAssertionError::format_for_arrays,
    (assertEq_14Call, assertEq_15Call),
    (assertEq_16Call, assertEq_17Call),
    (assertEq_18Call, assertEq_19Call),
    (assertEq_20Call, assertEq_21Call),
    (assertEq_22Call, assertEq_23Call),
    (assertEq_24Call, assertEq_25Call),
    (assertEq_26Call, assertEq_27Call),
}

impl_assertions! {
    |left, right, decimals| assert_eq(left, right),
    |e| e.format_with_decimals(decimals),
    (assertEqDecimal_0Call, assertEqDecimal_1Call),
    (assertEqDecimal_2Call, assertEqDecimal_3Call),
}

impl_assertions! {
    |left, right| assert_not_eq(left, right),
    ComparisonAssertionError::format_for_values,
    (assertNotEq_0Call, assertNotEq_1Call),
    (assertNotEq_2Call, assertNotEq_3Call),
    (assertNotEq_4Call, assertNotEq_5Call),
    (assertNotEq_6Call, assertNotEq_7Call),
    (assertNotEq_8Call, assertNotEq_9Call),
    (assertNotEq_10Call, assertNotEq_11Call),
    (assertNotEq_12Call, assertNotEq_13Call),
}

impl_assertions! {
    |left, right| assert_not_eq(left, right),
    ComparisonAssertionError::format_for_arrays,
    (assertNotEq_14Call, assertNotEq_15Call),
    (assertNotEq_16Call, assertNotEq_17Call),
    (assertNotEq_18Call, assertNotEq_19Call),
    (assertNotEq_20Call, assertNotEq_21Call),
    (assertNotEq_22Call, assertNotEq_23Call),
    (assertNotEq_24Call, assertNotEq_25Call),
    (assertNotEq_26Call, assertNotEq_27Call),
}

impl_assertions! {
    |left, right, decimals| assert_not_eq(left, right),
    |e| e.format_with_decimals(decimals),
    (assertNotEqDecimal_0Call, assertNotEqDecimal_1Call),
    (assertNotEqDecimal_2Call, assertNotEqDecimal_3Call),
}

impl_assertions! {
    |left, right| assert_gt(left, right),
    ComparisonAssertionError::format_for_values,
    (assertGt_0Call, assertGt_1Call),
    (assertGt_2Call, assertGt_3Call),
}

impl_assertions! {
    |left, right, decimals| assert_gt(left, right),
    |e| e.format_with_decimals(decimals),
    (assertGtDecimal_0Call, assertGtDecimal_1Call),
    (assertGtDecimal_2Call, assertGtDecimal_3Call),
}

impl_assertions! {
    |left, right| assert_ge(left, right),
    ComparisonAssertionError::format_for_values,
    (assertGe_0Call, assertGe_1Call),
    (assertGe_2Call, assertGe_3Call),
}

impl_assertions! {
    |left, right, decimals| assert_ge(left, right),
    |e| e.format_with_decimals(decimals),
    (assertGeDecimal_0Call, assertGeDecimal_1Call),
    (assertGeDecimal_2Call, assertGeDecimal_3Call),
}

impl_assertions! {
    |left, right| assert_lt(left, right),
    ComparisonAssertionError::format_for_values,
    (assertLt_0Call, assertLt_1Call),
    (assertLt_2Call, assertLt_3Call),
}

impl_assertions! {
    |left, right, decimals| assert_lt(left, right),
    |e| e.format_with_decimals(decimals),
    (assertLtDecimal_0Call, assertLtDecimal_1Call),
    (assertLtDecimal_2Call, assertLtDecimal_3Call),
}

impl_assertions! {
    |left, right| assert_le(left, right),
    ComparisonAssertionError::format_for_values,
    (assertLe_0Call, assertLe_1Call),
    (assertLe_2Call, assertLe_3Call),
}

impl_assertions! {
    |left, right, decimals| assert_le(left, right),
    |e| e.format_with_decimals(decimals),
    (assertLeDecimal_0Call, assertLeDecimal_1Call),
    (assertLeDecimal_2Call, assertLeDecimal_3Call),
}

impl_assertions! {
    |left, right, maxDelta| uint_assert_approx_eq_abs(*left, *right, *maxDelta),
    (assertApproxEqAbs_0Call, assertApproxEqAbs_1Call),
}

impl_assertions! {
    |left, right, maxDelta| int_assert_approx_eq_abs(*left, *right, *maxDelta),
    (assertApproxEqAbs_2Call, assertApproxEqAbs_3Call),
}

impl_assertions! {
    |left, right, decimals, maxDelta| uint_assert_approx_eq_abs(*left, *right, *maxDelta),
    |e| e.format_with_decimals(decimals),
    (assertApproxEqAbsDecimal_0Call, assertApproxEqAbsDecimal_1Call),
}

impl_assertions! {
    |left, right, decimals, maxDelta| int_assert_approx_eq_abs(*left, *right, *maxDelta),
    |e| e.format_with_decimals(decimals),
    (assertApproxEqAbsDecimal_2Call, assertApproxEqAbsDecimal_3Call),
}

impl_assertions! {
    |left, right, maxPercentDelta| uint_assert_approx_eq_rel(*left, *right, *maxPercentDelta),
    (assertApproxEqRel_0Call, assertApproxEqRel_1Call),
}

impl_assertions! {
    |left, right, maxPercentDelta| int_assert_approx_eq_rel(*left, *right, *maxPercentDelta),
    (assertApproxEqRel_2Call, assertApproxEqRel_3Call),
}

impl_assertions! {
    |left, right, decimals, maxPercentDelta| uint_assert_approx_eq_rel(*left, *right, *maxPercentDelta),
    |e| e.format_with_decimals(decimals),
    (assertApproxEqRelDecimal_0Call, assertApproxEqRelDecimal_1Call),
}

impl_assertions! {
    |left, right, decimals, maxPercentDelta| int_assert_approx_eq_rel(*left, *right, *maxPercentDelta),
    |e| e.format_with_decimals(decimals),
    (assertApproxEqRelDecimal_2Call, assertApproxEqRelDecimal_3Call),
}

const fn assert_true(condition: bool) -> Result<(), ()> {
    if condition { Ok(()) } else { Err(()) }
}

const fn assert_false(condition: bool) -> Result<(), ()> {
    assert_true(!condition)
}

fn assert_eq<'a, T: PartialEq>(left: &'a T, right: &'a T) -> ComparisonResult<'a, T> {
    if left == right {
        Ok(())
    } else {
        Err(ComparisonAssertionError { kind: AssertionKind::Eq, left, right })
    }
}

fn assert_not_eq<'a, T: PartialEq>(left: &'a T, right: &'a T) -> ComparisonResult<'a, T> {
    if left == right {
        Err(ComparisonAssertionError { kind: AssertionKind::Ne, left, right })
    } else {
        Ok(())
    }
}

fn assert_gt<'a, T: PartialOrd>(left: &'a T, right: &'a T) -> ComparisonResult<'a, T> {
    if left > right {
        Ok(())
    } else {
        Err(ComparisonAssertionError { kind: AssertionKind::Gt, left, right })
    }
}

fn assert_ge<'a, T: PartialOrd>(left: &'a T, right: &'a T) -> ComparisonResult<'a, T> {
    if left >= right {
        Ok(())
    } else {
        Err(ComparisonAssertionError { kind: AssertionKind::Ge, left, right })
    }
}

fn assert_lt<'a, T: PartialOrd>(left: &'a T, right: &'a T) -> ComparisonResult<'a, T> {
    if left < right {
        Ok(())
    } else {
        Err(ComparisonAssertionError { kind: AssertionKind::Lt, left, right })
    }
}

fn assert_le<'a, T: PartialOrd>(left: &'a T, right: &'a T) -> ComparisonResult<'a, T> {
    if left <= right {
        Ok(())
    } else {
        Err(ComparisonAssertionError { kind: AssertionKind::Le, left, right })
    }
}

fn get_delta_int(left: I256, right: I256) -> U256 {
    let (left_sign, left_abs) = left.into_sign_and_abs();
    let (right_sign, right_abs) = right.into_sign_and_abs();

    if left_sign == right_sign {
        if left_abs > right_abs { left_abs - right_abs } else { right_abs - left_abs }
    } else {
        left_abs.wrapping_add(right_abs)
    }
}

/// Calculates the relative delta for an absolute difference.
///
/// Avoids overflow in the multiplication by using [`U512`] to hold the intermediary result.
fn calc_delta_full<T>(abs_diff: U256, right: U256) -> Result<U256, EqRelAssertionError<T>> {
    let delta = U512::from(abs_diff) * U512::from(10).pow(U512::from(EQ_REL_DELTA_RESOLUTION))
        / U512::from(right);
    U256::checked_from_limbs_slice(delta.as_limbs()).ok_or(EqRelAssertionError::Overflow)
}

fn uint_assert_approx_eq_abs(
    left: U256,
    right: U256,
    max_delta: U256,
) -> Result<(), Box<EqAbsAssertionError<U256, U256>>> {
    let delta = left.abs_diff(right);

    if delta <= max_delta {
        Ok(())
    } else {
        Err(Box::new(EqAbsAssertionError { left, right, max_delta, real_delta: delta }))
    }
}

fn int_assert_approx_eq_abs(
    left: I256,
    right: I256,
    max_delta: U256,
) -> Result<(), Box<EqAbsAssertionError<I256, U256>>> {
    let delta = get_delta_int(left, right);

    if delta <= max_delta {
        Ok(())
    } else {
        Err(Box::new(EqAbsAssertionError { left, right, max_delta, real_delta: delta }))
    }
}

fn uint_assert_approx_eq_rel(
    left: U256,
    right: U256,
    max_delta: U256,
) -> Result<(), EqRelAssertionError<U256>> {
    if right.is_zero() {
        if left.is_zero() {
            return Ok(());
        }
        return Err(EqRelAssertionError::Failure(Box::new(EqRelAssertionFailure {
            left,
            right,
            max_delta,
            real_delta: EqRelDelta::Undefined,
        })));
    }

    let delta = calc_delta_full::<U256>(left.abs_diff(right), right)?;

    if delta <= max_delta {
        Ok(())
    } else {
        Err(EqRelAssertionError::Failure(Box::new(EqRelAssertionFailure {
            left,
            right,
            max_delta,
            real_delta: EqRelDelta::Defined(delta),
        })))
    }
}

fn int_assert_approx_eq_rel(
    left: I256,
    right: I256,
    max_delta: U256,
) -> Result<(), EqRelAssertionError<I256>> {
    if right.is_zero() {
        if left.is_zero() {
            return Ok(());
        }
        return Err(EqRelAssertionError::Failure(Box::new(EqRelAssertionFailure {
            left,
            right,
            max_delta,
            real_delta: EqRelDelta::Undefined,
        })));
    }

    let delta = calc_delta_full::<I256>(get_delta_int(left, right), right.unsigned_abs())?;

    if delta <= max_delta {
        Ok(())
    } else {
        Err(EqRelAssertionError::Failure(Box::new(EqRelAssertionFailure {
            left,
            right,
            max_delta,
            real_delta: EqRelDelta::Defined(delta),
        })))
    }
}

/// Records the failed assertions and frames of a call by value-independent call sites, so a
/// minimizer can tell whether a smaller input fails the same way.
#[derive(Clone, Debug, Default)]
pub struct FailureTrace {
    /// Site of the latest call made by an executing frame.
    pending: B256,
    /// Tracking of each executing frame, indexed by call depth.
    frames: Vec<SiteFrame>,
    /// Failed assertions and failed frames, in execution order.
    events: Vec<FailureEvent>,
    /// Whether a snapshot was restored, which can make a different path look like this one.
    pub(crate) snapshot_restored: bool,
}

/// Call site tracking of one executing frame.
#[derive(Clone, Debug, Default)]
struct SiteFrame {
    /// Site of the call that entered this frame.
    entry: B256,
    /// For each stack slot, the pc of the `PUSH` its value was copied from, if any.
    pushed_at: Vec<Option<usize>>,
    /// Address and code of this frame, to identify the instruction that halted it.
    destination: Address,
    /// See [`Self::destination`].
    code: Bytes,
    /// Whether this frame runs initcode.
    is_create: bool,
    /// Highest pc executed so far, used to hash the executed part of initcode.
    max_pc: usize,
    /// Pc of the latest instruction, which halted the frame if it halted.
    last_pc: usize,
    /// Pc of the latest `JUMPI`, taken or not.
    last_jumpi: usize,
    /// Number of failure events recorded before this frame started.
    events_before: usize,
    /// Site of the `REVERT` or `INVALID` instruction that failed this frame, if any.
    failed_at: Option<B256>,
}

impl SiteFrame {
    /// Returns the site of the instruction about to execute: the frame's entry, the executing
    /// code and pc, and the return labels of the internal functions that led to it. Values that
    /// are not pushed constants, such as inputs, are ignored.
    fn site<FEN: FoundryEvmNetwork>(&self, interpreter: &InterpreterFor<'_, '_, FEN>) -> B256 {
        let code = interpreter.original_bytecode_slice();
        let mut preimage = self.entry.to_vec();
        // Initcode embeds constructor arguments, so constructors only hash executed code, and
        // their address can derive from those arguments, so the entry identifies them instead.
        let code_hash = if self.is_create {
            keccak256(code.get(..=self.max_pc).unwrap_or(code))
        } else {
            preimage.extend_from_slice(interpreter.message().destination.as_slice());
            interpreter.original_bytecode_hash()
        };
        preimage.extend_from_slice(code_hash.as_slice());
        preimage.extend_from_slice(&interpreter.pc().to_be_bytes());
        for (value, pushed_at) in interpreter.stack().as_slice().iter().zip(&self.pushed_at) {
            if let Some(pushed_at) = pushed_at
                && is_return_label(code, value)
            {
                preimage.extend_from_slice(&pushed_at.to_be_bytes());
            }
        }
        keccak256(preimage)
    }

    /// Returns the site of the latest instruction, which halted the frame, without the return
    /// labels on its stack.
    fn halt_site(&self) -> B256 {
        let mut preimage = self.entry.to_vec();
        let code_hash = if self.is_create {
            keccak256(self.code.get(..=self.max_pc).unwrap_or(&self.code))
        } else {
            preimage.extend_from_slice(self.destination.as_slice());
            keccak256(&self.code)
        };
        preimage.extend_from_slice(code_hash.as_slice());
        preimage.extend_from_slice(&self.last_pc.to_be_bytes());
        preimage.extend_from_slice(&self.last_jumpi.to_be_bytes());
        keccak256(preimage)
    }
}

/// A failure recorded by [`FailureTrace`].
#[derive(Clone, Debug)]
enum FailureEvent {
    /// A failed assertion, identified by its call site, cheatcode and unformatted message, with
    /// the hash of its revert data when assertions revert.
    Assertion { identity: B256, revert: Option<B256> },
    /// A call or create that reverted or halted, identified by the site where it failed, or of
    /// the call when it ran no instructions, with the number of failure events recorded within it.
    Frame { site: B256, stop: InstrStop, events: usize },
}

impl FailureTrace {
    /// Starts tracking the frame of `interpreter` at `depth`.
    pub(crate) fn enter_frame<FEN: FoundryEvmNetwork>(
        &mut self,
        interpreter: &InterpreterFor<'_, '_, FEN>,
        depth: usize,
    ) {
        let frame = SiteFrame {
            entry: self.pending,
            events_before: self.events.len(),
            ..Default::default()
        };
        // Frames entered without an interpreter, such as cheatcode calls, were entered at
        // `pending` too.
        self.frames.resize(depth, frame.clone());
        self.frames.push(SiteFrame {
            destination: interpreter.message().destination,
            code: interpreter.original_bytecode(),
            is_create: interpreter.message().kind.is_create(),
            ..frame
        });
    }

    /// Records the site of external calls and tracks which stack values are pushed constants.
    pub(crate) fn step<FEN: FoundryEvmNetwork>(
        &mut self,
        interpreter: &mut InterpreterFor<'_, '_, FEN>,
        depth: usize,
    ) {
        let Some(frame) = self.frames.get_mut(depth) else { return };
        let height = interpreter.stack().len();
        frame.pushed_at.resize(height, None);
        let pc = interpreter.pc();
        frame.max_pc = frame.max_pc.max(pc);
        frame.last_pc = pc;
        let opcode = interpreter.opcode();
        match opcode {
            op::PUSH0..=op::PUSH32 => return frame.pushed_at.push(Some(pc)),
            op::DUP1..=op::DUP16 => {
                let n = (opcode - op::DUP1 + 1) as usize;
                if let Some(slot) = height.checked_sub(n) {
                    frame.pushed_at.push(frame.pushed_at[slot]);
                }
                return;
            }
            op::SWAP1..=op::SWAP16 => {
                let n = (opcode - op::SWAP1 + 1) as usize;
                if let Some(slot) = height.checked_sub(n + 1) {
                    frame.pushed_at.swap(slot, height - 1);
                }
                return;
            }
            // Their stack effect depends on an immediate, so stop trusting the tracked slots.
            op::SWAPN | op::EXCHANGE => frame.pushed_at.fill(None),
            op::CALL
            | op::CALLCODE
            | op::DELEGATECALL
            | op::STATICCALL
            | op::CREATE
            | op::CREATE2 => self.pending = frame.site::<FEN>(interpreter),
            op::JUMPI => frame.last_jumpi = pc,
            op::REVERT | op::INVALID => {
                // The optimizer shares revert blocks between checks, which the latest branch tells
                // apart.
                let site = frame.site::<FEN>(interpreter);
                let preimage = [site.as_slice(), &frame.last_jumpi.to_be_bytes()].concat();
                frame.failed_at = Some(keccak256(preimage));
            }
            _ => {}
        }
        if let Some(info) = OpCode::new(opcode) {
            frame.pushed_at.truncate(height.saturating_sub(info.inputs() as usize));
            frame.pushed_at.extend(std::iter::repeat_n(None, info.outputs() as usize));
        }
    }

    /// Records a failed assertion, or a `fail()`, made by the latest call.
    pub(crate) fn record_assertion(
        &mut self,
        selector: [u8; 4],
        message: &str,
        revert: Option<B256>,
    ) {
        let identity = keccak256([self.pending.as_slice(), &selector, message.as_bytes()].concat());
        self.events.push(FailureEvent::Assertion { identity, revert });
    }

    /// Records the outcome of the frame at `depth` once all inspectors have finalized it.
    pub fn finish_frame<FEN: FoundryEvmNetwork>(
        &mut self,
        depth: usize,
        result: &MessageResultFor<FEN>,
    ) {
        if !result.stop.is_success() {
            // Frames without an interpreter, such as cheatcode calls, were entered at `pending`.
            let (site, events_before) =
                self.frames.get(depth + 1).map_or((self.pending, self.events.len()), |frame| {
                    (frame.failed_at.unwrap_or_else(|| frame.halt_site()), frame.events_before)
                });
            let events = self.events.len() - events_before;
            self.events.push(FailureEvent::Frame { site, stop: result.stop, events });
        }
        self.frames.truncate(depth + 1);
    }

    /// Returns the value-independent identity of the finished call that stopped with `exit` and
    /// returned `output`, and starts recording the next call.
    ///
    /// The identity is unknown when a snapshot restore discarded part of the history, when the
    /// call succeeded without a failed assertion or `fail()`, such as after a legacy DSTest failure
    /// that only wrote its own flag, or when several failed assertions could have produced the
    /// revert data.
    pub fn take_identity(&mut self, exit: InstrStop, output: &[u8]) -> Option<B256> {
        let Self { events, snapshot_restored, .. } = std::mem::take(self);
        if snapshot_restored {
            return None;
        }
        let mut preimage = vec![exit as u8];
        for event in &events {
            match event {
                FailureEvent::Assertion { identity, .. } => {
                    preimage.push(0);
                    preimage.extend_from_slice(identity.as_slice());
                }
                FailureEvent::Frame { site, stop, events } => {
                    preimage.push(1);
                    preimage.extend_from_slice(site.as_slice());
                    preimage.push(*stop as u8);
                    preimage.extend_from_slice(&events.to_be_bytes());
                }
            }
        }
        if exit.is_success() {
            if !events.iter().any(|event| matches!(event, FailureEvent::Assertion { .. })) {
                return None;
            }
        } else {
            // Revert data produced by exactly one failed assertion is identified by that
            // assertion, without the values in its message. Other revert data is compared in full.
            let output_hash = keccak256(output);
            let mut sources = events.iter().enumerate().filter(|(_, event)| {
                matches!(event, FailureEvent::Assertion { revert, .. } if *revert == Some(output_hash))
            });
            match (sources.next(), sources.next()) {
                (Some((index, _)), None) => {
                    preimage.push(2);
                    preimage.extend_from_slice(&index.to_be_bytes());
                }
                (None, _) => {
                    preimage.push(3);
                    preimage.extend_from_slice(output_hash.as_slice());
                }
                (Some(_), Some(_)) => return None,
            }
        }
        Some(keccak256(preimage))
    }
}

/// Records where a call failed, cheaply enough to run during campaigns, so a replay of the same
/// input can be checked to fail the same way, unlike its revert data, exit reason and gas, which
/// other failure paths can reproduce.
///
/// Unlike [`FailureTrace`], sites include values such as return labels, so only runs of the same
/// input are comparable.
#[derive(Clone, Debug, Default)]
pub struct FailureProvenance {
    /// Frame index and site of the `REVERT` or `INVALID` instruction about to end a frame.
    pending_revert: Option<(usize, B256)>,
    /// Identity of the failed assertion made by the cheatcode call in progress.
    pub(crate) pending_assertion: Option<B256>,
    /// Hash of the failed assertions and failed frames so far, in execution order.
    hash: B256,
    /// Whether a frame halted at an instruction that is not recorded, so the call has no
    /// provenance.
    unknown: bool,
    /// Sites of the calls that entered the frames in progress, by depth.
    callers: Vec<B256>,
    /// Pc of the latest `JUMPI` of each frame in progress, taken or not, by the depth
    /// [`Self::record_revert`] receives.
    jumpis: Vec<usize>,
}

impl FailureProvenance {
    /// Returns the site of the instruction about to execute: the executing address, code and pc,
    /// and the stack values that look like return labels of internal functions.
    fn site<FEN: FoundryEvmNetwork>(interpreter: &InterpreterFor<'_, '_, FEN>) -> B256 {
        let code = interpreter.original_bytecode_slice();
        let mut preimage = interpreter.message().destination.to_vec();
        preimage.extend_from_slice(interpreter.original_bytecode_hash().as_slice());
        preimage.extend_from_slice(&interpreter.pc().to_be_bytes());
        for value in interpreter.stack().as_slice() {
            if is_return_label(code, value) {
                preimage.extend_from_slice(&value.to_be_bytes::<32>());
            }
        }
        keccak256(preimage)
    }

    /// Records the site of the `REVERT` or `INVALID` instruction about to execute in the frame at
    /// `depth`.
    pub(crate) fn record_revert<FEN: FoundryEvmNetwork>(
        &mut self,
        interpreter: &InterpreterFor<'_, '_, FEN>,
        depth: usize,
    ) {
        // The optimizer shares revert blocks between checks, which the frame's latest branch
        // tells apart.
        let jumpi = self.jumpis.get(depth).copied().unwrap_or_default();
        let site = Self::site::<FEN>(interpreter);
        self.pending_revert =
            Some((depth, keccak256([site.as_slice(), &jumpi.to_be_bytes()].concat())));
    }

    /// Records the `JUMPI` about to execute at `pc` in the frame at `depth`.
    pub(crate) fn record_jumpi(&mut self, depth: usize, pc: usize) {
        if self.jumpis.len() <= depth {
            self.jumpis.resize(depth + 1, 0);
        }
        self.jumpis[depth] = pc;
    }

    /// Records the site of `interpreter`'s call that enters the frame at `depth`.
    pub fn enter_frame<FEN: FoundryEvmNetwork>(
        &mut self,
        depth: usize,
        interpreter: &InterpreterFor<'_, '_, FEN>,
    ) {
        self.callers.truncate(depth);
        self.callers.push(Self::site::<FEN>(interpreter));
        // The entered frame, stepped at `depth + 1`, has not branched yet.
        self.jumpis.truncate(depth + 1);
    }

    /// Records the failed assertion of the finished cheatcode call, if any, by the sites of the
    /// calls that led to it, so the same assertion reached through different callers differs.
    pub(crate) fn finish_assertion(&mut self) {
        if let Some(assertion) = self.pending_assertion.take() {
            let site = keccak256(self.callers.concat());
            self.fold(keccak256([site.as_slice(), assertion.as_slice()].concat()));
        }
    }

    /// Records the outcome of the frame at `depth` once all inspectors have finalized it, by the
    /// sites of the calls that led to it, given the interpreter of its caller, or of the frame
    /// itself for the first frame.
    pub fn finish_frame<FEN: FoundryEvmNetwork>(
        &mut self,
        depth: usize,
        interpreter: &InterpreterFor<'_, '_, FEN>,
        result: &MessageResultFor<FEN>,
    ) {
        let pending_revert = self.pending_revert.take();
        // The same callee fails the same way when entered through different calls.
        let callers = (!result.stop.is_success()).then(|| keccak256(self.callers.concat()));
        self.callers.truncate(depth);
        let Some(callers) = callers else { return };
        let stop = result.stop as u8;
        let site = match pending_revert {
            Some((frame, site)) if frame == depth + 1 => site,
            // Halts such as invalid jumps or running out of gas can happen at any instruction,
            // which is not recorded.
            _ if result.stop != InstrStop::Revert => {
                self.unknown = true;
                return;
            }
            // Frames without instructions, such as cheatcode calls, are identified by the call
            // that entered them.
            _ => {
                let entry = if usize::from(interpreter.message().depth) < depth {
                    Self::site::<FEN>(interpreter)
                } else {
                    keccak256(interpreter.message().destination)
                };
                keccak256([entry.as_slice(), &[stop]].concat())
            }
        };
        let output = keccak256(&result.output);
        self.fold(keccak256(
            [callers.as_slice(), site.as_slice(), &[stop], output.as_slice()].concat(),
        ));
    }

    /// Returns the provenance of the finished call, if known, and starts recording the next call.
    pub fn take(&mut self) -> Option<B256> {
        let this = std::mem::take(self);
        (!this.unknown).then_some(this.hash)
    }

    /// Appends a failure event to the hash.
    fn fold(&mut self, event: B256) {
        self.hash = keccak256([self.hash.as_slice(), event.as_slice()].concat());
    }
}

/// Returns whether `value` is the pc of a `JUMPDEST` right after a `JUMP` in `code`, as the
/// return label of an internal function call is.
fn is_return_label(code: &[u8], value: &U256) -> bool {
    usize::try_from(*value).is_ok_and(|label| {
        label > 0 && code.get(label) == Some(&op::JUMPDEST) && code[label - 1] == op::JUMP
    })
}
