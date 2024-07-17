use crate::{CheatcodesExecutor, CheatsCtxt, Result, Vm::*};
use alloy_primitives::{hex, I256, U256};
use foundry_evm_core::{
    abi::{format_units_int, format_units_uint},
    backend::{DatabaseExt, GLOBAL_FAIL_SLOT},
    constants::CHEATCODE_ADDRESS,
};
use itertools::Itertools;
use std::fmt::{Debug, Display};

const EQ_REL_DELTA_RESOLUTION: U256 = U256::from_limbs([18, 0, 0, 0]);

#[derive(Debug, thiserror::Error)]
#[error("assertion failed")]
struct SimpleAssertionError;

#[derive(thiserror::Error, Debug)]
enum ComparisonAssertionError<'a, T> {
    Ne { left: &'a T, right: &'a T },
    Eq { left: &'a T, right: &'a T },
    Ge { left: &'a T, right: &'a T },
    Gt { left: &'a T, right: &'a T },
    Le { left: &'a T, right: &'a T },
    Lt { left: &'a T, right: &'a T },
}

macro_rules! format_values {
    ($self:expr, $format_fn:expr) => {
        match $self {
            Self::Ne { left, right } => format!("{} == {}", $format_fn(left), $format_fn(right)),
            Self::Eq { left, right } => format!("{} != {}", $format_fn(left), $format_fn(right)),
            Self::Ge { left, right } => format!("{} < {}", $format_fn(left), $format_fn(right)),
            Self::Gt { left, right } => format!("{} <= {}", $format_fn(left), $format_fn(right)),
            Self::Le { left, right } => format!("{} > {}", $format_fn(left), $format_fn(right)),
            Self::Lt { left, right } => format!("{} >= {}", $format_fn(left), $format_fn(right)),
        }
    };
}

impl<'a, T: Display> ComparisonAssertionError<'a, T> {
    fn format_for_values(&self) -> String {
        format_values!(self, T::to_string)
    }
}

impl<'a, T: Display> ComparisonAssertionError<'a, Vec<T>> {
    fn format_for_arrays(&self) -> String {
        let formatter = |v: &Vec<T>| format!("[{}]", v.iter().format(", "));
        format_values!(self, formatter)
    }
}

impl<'a> ComparisonAssertionError<'a, U256> {
    fn format_with_decimals(&self, decimals: &U256) -> String {
        let formatter = |v: &U256| format_units_uint(v, decimals);
        format_values!(self, formatter)
    }
}

impl<'a> ComparisonAssertionError<'a, I256> {
    fn format_with_decimals(&self, decimals: &U256) -> String {
        let formatter = |v: &I256| format_units_int(v, decimals);
        format_values!(self, formatter)
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

impl Display for EqRelDelta {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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

type ComparisonResult<'a, T> = Result<Vec<u8>, ComparisonAssertionError<'a, T>>;

fn handle_assertion_result<DB: DatabaseExt, E: CheatcodesExecutor, ERR>(
    result: core::result::Result<Vec<u8>, ERR>,
    ccx: &mut CheatsCtxt<DB>,
    executor: &mut E,
    error_formatter: impl Fn(&ERR) -> String,
    error_msg: Option<&str>,
    format_error: bool,
) -> Result {
    match result {
        Ok(_) => Ok(Default::default()),
        Err(err) => {
            let error_msg = error_msg.unwrap_or("assertion failed").to_string();
            let msg = if format_error {
                format!("{error_msg}: {}", error_formatter(&err))
            } else {
                error_msg
            };
            if ccx.state.config.assertions_revert {
                Err(msg.into())
            } else {
                executor.console_log(ccx, msg);
                ccx.ecx.sstore(CHEATCODE_ADDRESS, GLOBAL_FAIL_SLOT, U256::from(1))?;
                Ok(Default::default())
            }
        }
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
    (|$($arg:ident),*| $body:expr, $format_error:literal, $(($no_error:ident, $with_error:ident)),* $(,)?) => {
        impl_assertions!(@args_tt |($($arg),*)| $body, |e| e.to_string(), $format_error, $(($no_error, $with_error),)*);
    };
    (|$($arg:ident),*| $body:expr, $(($no_error:ident, $with_error:ident)),* $(,)?) => {
        impl_assertions!(@args_tt |($($arg),*)| $body, |e| e.to_string(), true, $(($no_error, $with_error),)*);
    };
    (|$($arg:ident),*| $body:expr, $error_formatter:expr, $(($no_error:ident, $with_error:ident)),* $(,)?) => {
        impl_assertions!(@args_tt |($($arg),*)| $body, $error_formatter, true, $(($no_error, $with_error)),*);
    };
    // We convert args to `tt` and later expand them back into tuple to allow usage of expanded args inside of
    // each assertion type context.
    (@args_tt |$args:tt| $body:expr, $error_formatter:expr, $format_error:literal, $(($no_error:ident, $with_error:ident)),* $(,)?) => {
        $(
            impl_assertions!(@impl $no_error, $with_error, $args, $body, $error_formatter, $format_error);
        )*
    };
    (@impl $no_error:ident, $with_error:ident, ($($arg:ident),*), $body:expr, $error_formatter:expr, $format_error:literal) => {
        impl crate::Cheatcode for $no_error {
            fn apply_full<DB: DatabaseExt, E: crate::CheatcodesExecutor>(
                &self,
                ccx: &mut CheatsCtxt<DB>,
                executor: &mut E,
            ) -> Result {
                let Self { $($arg),* } = self;
                handle_assertion_result($body, ccx, executor, $error_formatter, None, $format_error)
            }
        }

        impl crate::Cheatcode for $with_error {
            fn apply_full<DB: DatabaseExt, E: crate::CheatcodesExecutor>(
                &self,
                ccx: &mut CheatsCtxt<DB>,
                executor: &mut E,
            ) -> Result {
                let Self { $($arg),*, error} = self;
                handle_assertion_result($body, ccx, executor, $error_formatter, Some(error), $format_error)
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
    |e| e.format_for_values(),
    (assertEq_0Call, assertEq_1Call),
    (assertEq_2Call, assertEq_3Call),
    (assertEq_4Call, assertEq_5Call),
    (assertEq_6Call, assertEq_7Call),
    (assertEq_8Call, assertEq_9Call),
    (assertEq_10Call, assertEq_11Call),
}

impl_assertions! {
    |left, right| assert_eq(&hex::encode_prefixed(left), &hex::encode_prefixed(right)),
    |e| e.format_for_values(),
    (assertEq_12Call, assertEq_13Call),
}

impl_assertions! {
    |left, right| assert_eq(left, right),
    |e| e.format_for_arrays(),
    (assertEq_14Call, assertEq_15Call),
    (assertEq_16Call, assertEq_17Call),
    (assertEq_18Call, assertEq_19Call),
    (assertEq_20Call, assertEq_21Call),
    (assertEq_22Call, assertEq_23Call),
    (assertEq_24Call, assertEq_25Call),
}

impl_assertions! {
    |left, right| assert_eq(
        &left.iter().map(hex::encode_prefixed).collect::<Vec<_>>(),
        &right.iter().map(hex::encode_prefixed).collect::<Vec<_>>(),
    ),
    |e| e.format_for_arrays(),
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
    |e| e.format_for_values(),
    (assertNotEq_0Call, assertNotEq_1Call),
    (assertNotEq_2Call, assertNotEq_3Call),
    (assertNotEq_4Call, assertNotEq_5Call),
    (assertNotEq_6Call, assertNotEq_7Call),
    (assertNotEq_8Call, assertNotEq_9Call),
    (assertNotEq_10Call, assertNotEq_11Call),
}

impl_assertions! {
    |left, right| assert_not_eq(&hex::encode_prefixed(left), &hex::encode_prefixed(right)),
    |e| e.format_for_values(),
    (assertNotEq_12Call, assertNotEq_13Call),
}

impl_assertions! {
    |left, right| assert_not_eq(left, right),
    |e| e.format_for_arrays(),
    (assertNotEq_14Call, assertNotEq_15Call),
    (assertNotEq_16Call, assertNotEq_17Call),
    (assertNotEq_18Call, assertNotEq_19Call),
    (assertNotEq_20Call, assertNotEq_21Call),
    (assertNotEq_22Call, assertNotEq_23Call),
    (assertNotEq_24Call, assertNotEq_25Call),
}

impl_assertions! {
    |left, right| assert_not_eq(
        &left.iter().map(hex::encode_prefixed).collect::<Vec<_>>(),
        &right.iter().map(hex::encode_prefixed).collect::<Vec<_>>(),
    ),
    |e| e.format_for_arrays(),
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
    |e| e.format_for_values(),
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
    |e| e.format_for_values(),
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
    |e| e.format_for_values(),
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
    |e| e.format_for_values(),
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

fn assert_true(condition: bool) -> Result<Vec<u8>, SimpleAssertionError> {
    if condition {
        Ok(Default::default())
    } else {
        Err(SimpleAssertionError)
    }
}

fn assert_false(condition: bool) -> Result<Vec<u8>, SimpleAssertionError> {
    if !condition {
        Ok(Default::default())
    } else {
        Err(SimpleAssertionError)
    }
}

fn assert_eq<'a, T: PartialEq>(left: &'a T, right: &'a T) -> ComparisonResult<'a, T> {
    if left == right {
        Ok(Default::default())
    } else {
        Err(ComparisonAssertionError::Eq { left, right })
    }
}

fn assert_not_eq<'a, T: PartialEq>(left: &'a T, right: &'a T) -> ComparisonResult<'a, T> {
    if left != right {
        Ok(Default::default())
    } else {
        Err(ComparisonAssertionError::Ne { left, right })
    }
}

fn get_delta_uint(left: U256, right: U256) -> U256 {
    if left > right {
        left - right
    } else {
        right - left
    }
}

fn get_delta_int(left: I256, right: I256) -> U256 {
    let (left_sign, left_abs) = left.into_sign_and_abs();
    let (right_sign, right_abs) = right.into_sign_and_abs();

    if left_sign == right_sign {
        if left_abs > right_abs {
            left_abs - right_abs
        } else {
            right_abs - left_abs
        }
    } else {
        left_abs + right_abs
    }
}

fn uint_assert_approx_eq_abs(
    left: U256,
    right: U256,
    max_delta: U256,
) -> Result<Vec<u8>, Box<EqAbsAssertionError<U256, U256>>> {
    let delta = get_delta_uint(left, right);

    if delta <= max_delta {
        Ok(Default::default())
    } else {
        Err(Box::new(EqAbsAssertionError { left, right, max_delta, real_delta: delta }))
    }
}

fn int_assert_approx_eq_abs(
    left: I256,
    right: I256,
    max_delta: U256,
) -> Result<Vec<u8>, Box<EqAbsAssertionError<I256, U256>>> {
    let delta = get_delta_int(left, right);

    if delta <= max_delta {
        Ok(Default::default())
    } else {
        Err(Box::new(EqAbsAssertionError { left, right, max_delta, real_delta: delta }))
    }
}

fn uint_assert_approx_eq_rel(
    left: U256,
    right: U256,
    max_delta: U256,
) -> Result<Vec<u8>, EqRelAssertionError<U256>> {
    if right.is_zero() {
        if left.is_zero() {
            return Ok(Default::default())
        } else {
            return Err(EqRelAssertionError::Failure(Box::new(EqRelAssertionFailure {
                left,
                right,
                max_delta,
                real_delta: EqRelDelta::Undefined,
            })))
        };
    }

    let delta = get_delta_uint(left, right)
        .checked_mul(U256::pow(U256::from(10), EQ_REL_DELTA_RESOLUTION))
        .ok_or(EqRelAssertionError::Overflow)? /
        right;

    if delta <= max_delta {
        Ok(Default::default())
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
) -> Result<Vec<u8>, EqRelAssertionError<I256>> {
    if right.is_zero() {
        if left.is_zero() {
            return Ok(Default::default())
        } else {
            return Err(EqRelAssertionError::Failure(Box::new(EqRelAssertionFailure {
                left,
                right,
                max_delta,
                real_delta: EqRelDelta::Undefined,
            })))
        }
    }

    let (_, abs_right) = right.into_sign_and_abs();
    let delta = get_delta_int(left, right)
        .checked_mul(U256::pow(U256::from(10), EQ_REL_DELTA_RESOLUTION))
        .ok_or(EqRelAssertionError::Overflow)? /
        abs_right;

    if delta <= max_delta {
        Ok(Default::default())
    } else {
        Err(EqRelAssertionError::Failure(Box::new(EqRelAssertionFailure {
            left,
            right,
            max_delta,
            real_delta: EqRelDelta::Defined(delta),
        })))
    }
}

fn assert_gt<'a, T: PartialOrd>(left: &'a T, right: &'a T) -> ComparisonResult<'a, T> {
    if left > right {
        Ok(Default::default())
    } else {
        Err(ComparisonAssertionError::Gt { left, right })
    }
}

fn assert_ge<'a, T: PartialOrd>(left: &'a T, right: &'a T) -> ComparisonResult<'a, T> {
    if left >= right {
        Ok(Default::default())
    } else {
        Err(ComparisonAssertionError::Ge { left, right })
    }
}

fn assert_lt<'a, T: PartialOrd>(left: &'a T, right: &'a T) -> ComparisonResult<'a, T> {
    if left < right {
        Ok(Default::default())
    } else {
        Err(ComparisonAssertionError::Lt { left, right })
    }
}

fn assert_le<'a, T: PartialOrd>(left: &'a T, right: &'a T) -> ComparisonResult<'a, T> {
    if left <= right {
        Ok(Default::default())
    } else {
        Err(ComparisonAssertionError::Le { left, right })
    }
}
