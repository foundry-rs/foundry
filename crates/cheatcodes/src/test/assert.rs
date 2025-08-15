use crate::{CheatcodesExecutor, CheatsCtxt, Result, Vm::*};
use alloy_primitives::{I256, U256, U512};
use foundry_evm_core::{
    abi::console::{format_units_int, format_units_uint},
    backend::GLOBAL_FAIL_SLOT,
    constants::CHEATCODE_ADDRESS,
};
use itertools::Itertools;
use revm::context::JournalTr;
use std::{borrow::Cow, fmt};

const EQ_REL_DELTA_RESOLUTION: U256 = U256::from_limbs([18, 0, 0, 0]);

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
    fn inverse(self) -> Self {
        match self {
            Self::Eq => Self::Ne,
            Self::Ne => Self::Eq,
            Self::Gt => Self::Le,
            Self::Ge => Self::Lt,
            Self::Lt => Self::Ge,
            Self::Le => Self::Gt,
        }
    }

    fn to_str(self) -> &'static str {
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
                &f.real_delta,
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
                &f.real_delta,
            ),
            Self::Overflow => self.to_string(),
        }
    }
}

type ComparisonResult<'a, T> = Result<(), ComparisonAssertionError<'a, T>>;

#[cold]
fn handle_assertion_result<E>(
    ccx: &mut CheatsCtxt,
    executor: &mut dyn CheatcodesExecutor,
    err: E,
    error_formatter: Option<&dyn Fn(&E) -> String>,
    error_msg: Option<&str>,
) -> Result {
    let error_msg = error_msg.unwrap_or("assertion failed");
    let msg = if let Some(error_formatter) = error_formatter {
        Cow::Owned(format!("{error_msg}: {}", error_formatter(&err)))
    } else {
        Cow::Borrowed(error_msg)
    };
    handle_assertion_result_mono(ccx, executor, msg)
}

fn handle_assertion_result_mono(
    ccx: &mut CheatsCtxt,
    executor: &mut dyn CheatcodesExecutor,
    msg: Cow<'_, str>,
) -> Result {
    if ccx.state.config.assertions_revert {
        Err(msg.into_owned().into())
    } else {
        executor.console_log(ccx, &msg);
        ccx.ecx.journaled_state.sstore(CHEATCODE_ADDRESS, GLOBAL_FAIL_SLOT, U256::from(1))?;
        Ok(Default::default())
    }
}

/// Implements [crate::Cheatcode] for pairs of cheatcodes.
///
/// Accepts a list of pairs of cheatcodes, where the first cheatcode is the one that doesn't contain
/// a custom error message, and the second one contains it at `error` field.
///
/// Passed `args` are the common arguments for both cheatcode structs (excluding `error` field).
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
            fn apply_full(
                &self,
                ccx: &mut CheatsCtxt,
                executor: &mut dyn CheatcodesExecutor,
            ) -> Result {
                let Self { $($arg),* } = self;
                match $body {
                    Ok(()) => Ok(Default::default()),
                    Err(err) => handle_assertion_result(ccx, executor, err, $error_formatter, None)
                }
            }
        }

        impl crate::Cheatcode for $with_error {
            fn apply_full(
                &self,
                ccx: &mut CheatsCtxt,
                executor: &mut dyn CheatcodesExecutor,
            ) -> Result {
                let Self { $($arg,)* error } = self;
                match $body {
                    Ok(()) => Ok(Default::default()),
                    Err(err) => handle_assertion_result(ccx, executor, err, $error_formatter, Some(error))
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

fn assert_true(condition: bool) -> Result<(), ()> {
    if condition { Ok(()) } else { Err(()) }
}

fn assert_false(condition: bool) -> Result<(), ()> {
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
    if left != right {
        Ok(())
    } else {
        Err(ComparisonAssertionError { kind: AssertionKind::Ne, left, right })
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
        } else {
            return Err(EqRelAssertionError::Failure(Box::new(EqRelAssertionFailure {
                left,
                right,
                max_delta,
                real_delta: EqRelDelta::Undefined,
            })));
        };
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
        } else {
            return Err(EqRelAssertionError::Failure(Box::new(EqRelAssertionFailure {
                left,
                right,
                max_delta,
                real_delta: EqRelDelta::Undefined,
            })));
        }
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
