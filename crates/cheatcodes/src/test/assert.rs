use std::fmt::{Debug, Display};

use alloy_primitives::{I256, U256};
use foundry_evm_core::abi::{format_units_int, format_units_uint};
use itertools::Itertools;

use crate::{Cheatcode, Cheatcodes, Result, Vm::*};

const EQ_REL_DELTA_RESOLUTION: U256 = U256::from_limbs([18, 0, 0, 0]);

#[derive(Debug, thiserror::Error)]
#[error("Assertion failed")]
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
        let formatter = |v: &Vec<T>| format!("[{}]", v.iter().format(", ").to_string());
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

#[derive(thiserror::Error, Debug)]
#[error(
    "{left} !~= {right} (max delta: {}, real delta: {})",
    format_delta_percent(max_delta),
    format_delta_percent(real_delta)
)]
struct EqRelAssertionFailure<T> {
    left: T,
    right: T,
    max_delta: U256,
    real_delta: U256,
}

#[derive(thiserror::Error, Debug)]
enum EqRelAssertionError<T> {
    #[error(transparent)]
    Failure(Box<EqRelAssertionFailure<T>>),
    #[error("Overflow in delta calculation")]
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
                format_delta_percent(&f.real_delta),
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
                format_delta_percent(&f.real_delta),
            ),
            Self::Overflow => self.to_string(),
        }
    }
}

type ComparisonResult<'a, T> = Result<Vec<u8>, ComparisonAssertionError<'a, T>>;

impl Cheatcode for assertTrue_0Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok(assert_true(self.condition).map_err(|_| "Assertion failed")?)
    }
}

impl Cheatcode for assertTrue_1Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok(assert_true(self.condition).map_err(|_| self.error.to_string())?)
    }
}

impl Cheatcode for assertFalse_0Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok(assert_false(self.condition).map_err(|_| "Assertion failed")?)
    }
}

impl Cheatcode for assertFalse_1Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok(assert_false(self.condition).map_err(|_| self.error.to_string())?)
    }
}

impl Cheatcode for assertEq_0Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right } = self;
        Ok(assert_eq(left, right)
            .map_err(|e| format!("Assertion failed: {}", e.format_for_values()))?)
    }
}

impl Cheatcode for assertEq_1Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right, error } = self;
        Ok(assert_eq(left, right).map_err(|e| format!("{}: {}", error, e.format_for_values()))?)
    }
}

impl Cheatcode for assertEq_2Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right } = self;
        Ok(assert_eq(left, right)
            .map_err(|e| format!("Assertion failed: {}", e.format_for_values()))?)
    }
}

impl Cheatcode for assertEq_3Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right, error } = self;
        Ok(assert_eq(left, right).map_err(|e| format!("{}: {}", error, e.format_for_values()))?)
    }
}

impl Cheatcode for assertEq_4Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right } = self;
        Ok(assert_eq(left, right)
            .map_err(|e| format!("Assertion failed: {}", e.format_for_values()))?)
    }
}

impl Cheatcode for assertEq_5Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right, error } = self;
        Ok(assert_eq(left, right).map_err(|e| format!("{}: {}", error, e.format_for_values()))?)
    }
}

impl Cheatcode for assertEq_6Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right } = self;
        Ok(assert_eq(left, right)
            .map_err(|e| format!("Assertion failed: {}", e.format_for_values()))?)
    }
}

impl Cheatcode for assertEq_7Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right, error } = self;
        Ok(assert_eq(left, right).map_err(|e| format!("{}: {}", error, e.format_for_values()))?)
    }
}

impl Cheatcode for assertEq_8Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right } = self;
        Ok(assert_eq(left, right)
            .map_err(|e| format!("Assertion failed: {}", e.format_for_values()))?)
    }
}

impl Cheatcode for assertEq_9Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right, error } = self;
        Ok(assert_eq(left, right).map_err(|e| format!("{}: {}", error, e.format_for_values()))?)
    }
}

impl Cheatcode for assertEq_10Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right } = self;
        Ok(assert_eq(left, right)
            .map_err(|e| format!("Assertion failed: {}", e.format_for_values()))?)
    }
}

impl Cheatcode for assertEq_11Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right, error } = self;
        Ok(assert_eq(left, right).map_err(|e| format!("{}: {}", error, e.format_for_values()))?)
    }
}

impl Cheatcode for assertEq_12Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right } = self;
        Ok(assert_eq(&hex::encode_prefixed(left), &hex::encode_prefixed(right))
            .map_err(|e| format!("Assertion failed: {}", e.format_for_values()))?)
    }
}

impl Cheatcode for assertEq_13Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right, error } = self;
        Ok(assert_eq(&hex::encode_prefixed(left), &hex::encode_prefixed(right))
            .map_err(|e| format!("{}: {}", error, e.format_for_values()))?)
    }
}

impl Cheatcode for assertEq_14Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right } = self;
        Ok(assert_eq(left, right)
            .map_err(|e| format!("Assertion failed: {}", e.format_for_arrays()))?)
    }
}

impl Cheatcode for assertEq_15Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right, error } = self;
        Ok(assert_eq(left, right).map_err(|e| format!("{}: {}", error, e.format_for_arrays()))?)
    }
}

impl Cheatcode for assertEq_16Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right } = self;
        Ok(assert_eq(left, right)
            .map_err(|e| format!("Assertion failed: {}", e.format_for_arrays()))?)
    }
}

impl Cheatcode for assertEq_17Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right, error } = self;
        Ok(assert_eq(left, right).map_err(|e| format!("{}: {}", error, e.format_for_arrays()))?)
    }
}

impl Cheatcode for assertEq_18Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right } = self;
        Ok(assert_eq(left, right)
            .map_err(|e| format!("Assertion failed: {}", e.format_for_arrays()))?)
    }
}

impl Cheatcode for assertEq_19Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right, error } = self;
        Ok(assert_eq(left, right).map_err(|e| format!("{}: {}", error, e.format_for_arrays()))?)
    }
}

impl Cheatcode for assertEq_20Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right } = self;
        Ok(assert_eq(left, right)
            .map_err(|e| format!("Assertion failed: {}", e.format_for_arrays()))?)
    }
}

impl Cheatcode for assertEq_21Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right, error } = self;
        Ok(assert_eq(left, right).map_err(|e| format!("{}: {}", error, e.format_for_arrays()))?)
    }
}

impl Cheatcode for assertEq_22Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right } = self;
        Ok(assert_eq(left, right)
            .map_err(|e| format!("Assertion failed: {}", e.format_for_arrays()))?)
    }
}

impl Cheatcode for assertEq_23Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right, error } = self;
        Ok(assert_eq(left, right).map_err(|e| format!("{}: {}", error, e.format_for_arrays()))?)
    }
}

impl Cheatcode for assertEq_24Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right } = self;
        Ok(assert_eq(left, right)
            .map_err(|e| format!("Assertion failed: {}", e.format_for_arrays()))?)
    }
}

impl Cheatcode for assertEq_25Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right, error } = self;
        Ok(assert_eq(left, right).map_err(|e| format!("{}: {}", error, e.format_for_arrays()))?)
    }
}

impl Cheatcode for assertEq_26Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right } = self;
        let left = left.iter().map(hex::encode_prefixed).collect::<Vec<_>>();
        let right = right.iter().map(hex::encode_prefixed).collect::<Vec<_>>();
        Ok(assert_eq(&left, &right)
            .map_err(|e| format!("Assertion failed: {}", e.format_for_arrays()))?)
    }
}

impl Cheatcode for assertEq_27Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right, error } = self;
        let left = left.iter().map(hex::encode_prefixed).collect::<Vec<_>>();
        let right = right.iter().map(hex::encode_prefixed).collect::<Vec<_>>();
        Ok(assert_eq(&left, &right).map_err(|e| format!("{}: {}", error, e.format_for_arrays()))?)
    }
}

impl Cheatcode for assertEqDecimal_0Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok(assert_eq(&self.left, &self.right)
            .map_err(|e| format!("Assertion failed: {}", e.format_with_decimals(&self.decimals)))?)
    }
}

impl Cheatcode for assertEqDecimal_1Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok(assert_eq(&self.left, &self.right)
            .map_err(|e| format!("{}: {}", self.error, e.format_with_decimals(&self.decimals)))?)
    }
}

impl Cheatcode for assertEqDecimal_2Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok(assert_eq(&self.left, &self.right)
            .map_err(|e| format!("Assertion failed: {}", e.format_with_decimals(&self.decimals)))?)
    }
}

impl Cheatcode for assertEqDecimal_3Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok(assert_eq(&self.left, &self.right)
            .map_err(|e| format!("{}: {}", self.error, e.format_with_decimals(&self.decimals)))?)
    }
}

impl Cheatcode for assertNotEq_0Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right } = self;
        Ok(assert_not_eq(left, right)
            .map_err(|e| format!("Assertion failed: {}", e.format_for_values()))?)
    }
}

impl Cheatcode for assertNotEq_1Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right, error } = self;
        Ok(assert_not_eq(left, right)
            .map_err(|e| format!("{}: {}", error, e.format_for_values()))?)
    }
}

impl Cheatcode for assertNotEq_2Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right } = self;
        Ok(assert_not_eq(left, right)
            .map_err(|e| format!("Assertion failed: {}", e.format_for_values()))?)
    }
}

impl Cheatcode for assertNotEq_3Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right, error } = self;
        Ok(assert_not_eq(left, right)
            .map_err(|e| format!("{}: {}", error, e.format_for_values()))?)
    }
}

impl Cheatcode for assertNotEq_4Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right } = self;
        Ok(assert_not_eq(left, right)
            .map_err(|e| format!("Assertion failed: {}", e.format_for_values()))?)
    }
}

impl Cheatcode for assertNotEq_5Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right, error } = self;
        Ok(assert_not_eq(left, right)
            .map_err(|e| format!("{}: {}", error, e.format_for_values()))?)
    }
}

impl Cheatcode for assertNotEq_6Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right } = self;
        Ok(assert_not_eq(left, right)
            .map_err(|e| format!("Assertion failed: {}", e.format_for_values()))?)
    }
}

impl Cheatcode for assertNotEq_7Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right, error } = self;
        Ok(assert_not_eq(left, right)
            .map_err(|e| format!("{}: {}", error, e.format_for_values()))?)
    }
}

impl Cheatcode for assertNotEq_8Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right } = self;
        Ok(assert_not_eq(left, right)
            .map_err(|e| format!("Assertion failed: {}", e.format_for_values()))?)
    }
}

impl Cheatcode for assertNotEq_9Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right, error } = self;
        Ok(assert_not_eq(left, right)
            .map_err(|e| format!("{}: {}", error, e.format_for_values()))?)
    }
}

impl Cheatcode for assertNotEq_10Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right } = self;
        Ok(assert_not_eq(left, right)
            .map_err(|e| format!("Assertion failed: {}", e.format_for_values()))?)
    }
}

impl Cheatcode for assertNotEq_11Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right, error } = self;
        Ok(assert_not_eq(left, right)
            .map_err(|e| format!("{}: {}", error, e.format_for_values()))?)
    }
}

impl Cheatcode for assertNotEq_12Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right } = self;
        Ok(assert_not_eq(&hex::encode_prefixed(left), &hex::encode_prefixed(right))
            .map_err(|e| format!("Assertion failed: {}", e.format_for_values()))?)
    }
}

impl Cheatcode for assertNotEq_13Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right, error } = self;
        Ok(assert_not_eq(&hex::encode_prefixed(left), &hex::encode_prefixed(right))
            .map_err(|e| format!("{}: {}", error, e.format_for_values()))?)
    }
}

impl Cheatcode for assertNotEq_14Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right } = self;
        Ok(assert_not_eq(left, right)
            .map_err(|e| format!("Assertion failed: {}", e.format_for_arrays()))?)
    }
}

impl Cheatcode for assertNotEq_15Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right, error } = self;
        Ok(assert_not_eq(left, right)
            .map_err(|e| format!("{}: {}", error, e.format_for_arrays()))?)
    }
}

impl Cheatcode for assertNotEq_16Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right } = self;
        Ok(assert_not_eq(left, right)
            .map_err(|e| format!("Assertion failed: {}", e.format_for_arrays()))?)
    }
}

impl Cheatcode for assertNotEq_17Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right, error } = self;
        Ok(assert_not_eq(left, right)
            .map_err(|e| format!("{}: {}", error, e.format_for_arrays()))?)
    }
}

impl Cheatcode for assertNotEq_18Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right } = self;
        Ok(assert_not_eq(left, right)
            .map_err(|e| format!("Assertion failed: {}", e.format_for_arrays()))?)
    }
}

impl Cheatcode for assertNotEq_19Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right, error } = self;
        Ok(assert_not_eq(left, right)
            .map_err(|e| format!("{}: {}", error, e.format_for_arrays()))?)
    }
}

impl Cheatcode for assertNotEq_20Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right } = self;
        Ok(assert_not_eq(left, right)
            .map_err(|e| format!("Assertion failed: {}", e.format_for_arrays()))?)
    }
}

impl Cheatcode for assertNotEq_21Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right, error } = self;
        Ok(assert_not_eq(left, right)
            .map_err(|e| format!("{}: {}", error, e.format_for_arrays()))?)
    }
}

impl Cheatcode for assertNotEq_22Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right } = self;
        Ok(assert_not_eq(left, right)
            .map_err(|e| format!("Assertion failed: {}", e.format_for_arrays()))?)
    }
}

impl Cheatcode for assertNotEq_23Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right, error } = self;
        Ok(assert_not_eq(left, right)
            .map_err(|e| format!("{}: {}", error, e.format_for_arrays()))?)
    }
}

impl Cheatcode for assertNotEq_24Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right } = self;
        Ok(assert_not_eq(left, right)
            .map_err(|e| format!("Assertion failed: {}", e.format_for_arrays()))?)
    }
}

impl Cheatcode for assertNotEq_25Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right, error } = self;
        Ok(assert_not_eq(left, right)
            .map_err(|e| format!("{}: {}", error, e.format_for_arrays()))?)
    }
}

impl Cheatcode for assertNotEq_26Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right } = self;
        let left = left.iter().map(hex::encode_prefixed).collect::<Vec<_>>();
        let right = right.iter().map(hex::encode_prefixed).collect::<Vec<_>>();
        Ok(assert_not_eq(&left, &right)
            .map_err(|e| format!("Assertion failed: {}", e.format_for_arrays()))?)
    }
}

impl Cheatcode for assertNotEq_27Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right, error } = self;
        let left = left.iter().map(hex::encode_prefixed).collect::<Vec<_>>();
        let right = right.iter().map(hex::encode_prefixed).collect::<Vec<_>>();
        Ok(assert_not_eq(&left, &right)
            .map_err(|e| format!("{}: {}", error, e.format_for_arrays()))?)
    }
}

impl Cheatcode for assertNotEqDecimal_0Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok(assert_not_eq(&self.left, &self.right)
            .map_err(|e| format!("Assertion failed: {}", e.format_with_decimals(&self.decimals)))?)
    }
}

impl Cheatcode for assertNotEqDecimal_1Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok(assert_not_eq(&self.left, &self.right)
            .map_err(|e| format!("{}: {}", self.error, e.format_with_decimals(&self.decimals)))?)
    }
}

impl Cheatcode for assertNotEqDecimal_2Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok(assert_not_eq(&self.left, &self.right)
            .map_err(|e| format!("Assertion failed: {}", e.format_with_decimals(&self.decimals)))?)
    }
}

impl Cheatcode for assertNotEqDecimal_3Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok(assert_not_eq(&self.left, &self.right)
            .map_err(|e| format!("{}: {}", self.error, e.format_with_decimals(&self.decimals)))?)
    }
}

impl Cheatcode for assertGt_0Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right } = self;
        Ok(assert_gt(left, right)
            .map_err(|e| format!("Assertion failed: {}", e.format_for_values()))?)
    }
}

impl Cheatcode for assertGt_1Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right, error } = self;
        Ok(assert_gt(left, right).map_err(|e| format!("{}: {}", error, e.format_for_values()))?)
    }
}

impl Cheatcode for assertGt_2Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right } = self;
        Ok(assert_gt(left, right)
            .map_err(|e| format!("Assertion failed: {}", e.format_for_values()))?)
    }
}

impl Cheatcode for assertGt_3Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right, error } = self;
        Ok(assert_gt(left, right).map_err(|e| format!("{}: {}", error, e.format_for_values()))?)
    }
}

impl Cheatcode for assertGtDecimal_0Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok(assert_gt(&self.left, &self.right)
            .map_err(|e| format!("Assertion failed: {}", e.format_with_decimals(&self.decimals)))?)
    }
}

impl Cheatcode for assertGtDecimal_1Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok(assert_gt(&self.left, &self.right)
            .map_err(|e| format!("{}: {}", self.error, e.format_with_decimals(&self.decimals)))?)
    }
}

impl Cheatcode for assertGtDecimal_2Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok(assert_gt(&self.left, &self.right)
            .map_err(|e| format!("Assertion failed: {}", e.format_with_decimals(&self.decimals)))?)
    }
}

impl Cheatcode for assertGtDecimal_3Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok(assert_gt(&self.left, &self.right)
            .map_err(|e| format!("{}: {}", self.error, e.format_with_decimals(&self.decimals)))?)
    }
}

impl Cheatcode for assertGe_0Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right } = self;
        Ok(assert_ge(left, right)
            .map_err(|e| format!("Assertion failed: {}", e.format_for_values()))?)
    }
}

impl Cheatcode for assertGe_1Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right, error } = self;
        Ok(assert_ge(left, right).map_err(|e| format!("{}: {}", error, e.format_for_values()))?)
    }
}

impl Cheatcode for assertGe_2Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right } = self;
        Ok(assert_ge(left, right)
            .map_err(|e| format!("Assertion failed: {}", e.format_for_values()))?)
    }
}

impl Cheatcode for assertGe_3Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right, error } = self;
        Ok(assert_ge(left, right).map_err(|e| format!("{}: {}", error, e.format_for_values()))?)
    }
}

impl Cheatcode for assertGeDecimal_0Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok(assert_ge(&self.left, &self.right)
            .map_err(|e| format!("Assertion failed: {}", e.format_with_decimals(&self.decimals)))?)
    }
}

impl Cheatcode for assertGeDecimal_1Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok(assert_ge(&self.left, &self.right)
            .map_err(|e| format!("{}: {}", self.error, e.format_with_decimals(&self.decimals)))?)
    }
}

impl Cheatcode for assertGeDecimal_2Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok(assert_ge(&self.left, &self.right)
            .map_err(|e| format!("Assertion failed: {}", e.format_with_decimals(&self.decimals)))?)
    }
}

impl Cheatcode for assertGeDecimal_3Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok(assert_ge(&self.left, &self.right)
            .map_err(|e| format!("{}: {}", self.error, e.format_with_decimals(&self.decimals)))?)
    }
}

impl Cheatcode for assertLt_0Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right } = self;
        Ok(assert_lt(left, right)
            .map_err(|e| format!("Assertion failed: {}", e.format_for_values()))?)
    }
}

impl Cheatcode for assertLt_1Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right, error } = self;
        Ok(assert_lt(left, right).map_err(|e| format!("{}: {}", error, e.format_for_values()))?)
    }
}

impl Cheatcode for assertLt_2Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right } = self;
        Ok(assert_lt(left, right)
            .map_err(|e| format!("Assertion failed: {}", e.format_for_values()))?)
    }
}

impl Cheatcode for assertLt_3Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right, error } = self;
        Ok(assert_lt(left, right).map_err(|e| format!("{}: {}", error, e.format_for_values()))?)
    }
}

impl Cheatcode for assertLtDecimal_0Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok(assert_lt(&self.left, &self.right)
            .map_err(|e| format!("Assertion failed: {}", e.format_with_decimals(&self.decimals)))?)
    }
}

impl Cheatcode for assertLtDecimal_1Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok(assert_lt(&self.left, &self.right)
            .map_err(|e| format!("{}: {}", self.error, e.format_with_decimals(&self.decimals)))?)
    }
}

impl Cheatcode for assertLtDecimal_2Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok(assert_lt(&self.left, &self.right)
            .map_err(|e| format!("Assertion failed: {}", e.format_with_decimals(&self.decimals)))?)
    }
}

impl Cheatcode for assertLtDecimal_3Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok(assert_lt(&self.left, &self.right)
            .map_err(|e| format!("{}: {}", self.error, e.format_with_decimals(&self.decimals)))?)
    }
}

impl Cheatcode for assertLe_0Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right } = self;
        Ok(assert_le(left, right)
            .map_err(|e| format!("Assertion failed: {}", e.format_for_values()))?)
    }
}

impl Cheatcode for assertLe_1Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right, error } = self;
        Ok(assert_le(left, right).map_err(|e| format!("{}: {}", error, e.format_for_values()))?)
    }
}

impl Cheatcode for assertLe_2Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right } = self;
        Ok(assert_le(left, right)
            .map_err(|e| format!("Assertion failed: {}", e.format_for_values()))?)
    }
}

impl Cheatcode for assertLe_3Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        let Self { left, right, error } = self;
        Ok(assert_le(left, right).map_err(|e| format!("{}: {}", error, e.format_for_values()))?)
    }
}

impl Cheatcode for assertLeDecimal_0Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok(assert_le(&self.left, &self.right)
            .map_err(|e| format!("Assertion failed: {}", e.format_with_decimals(&self.decimals)))?)
    }
}

impl Cheatcode for assertLeDecimal_1Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok(assert_le(&self.left, &self.right)
            .map_err(|e| format!("{}: {}", self.error, e.format_with_decimals(&self.decimals)))?)
    }
}

impl Cheatcode for assertLeDecimal_2Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok(assert_le(&self.left, &self.right)
            .map_err(|e| format!("Assertion failed: {}", e.format_with_decimals(&self.decimals)))?)
    }
}

impl Cheatcode for assertLeDecimal_3Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok(assert_le(&self.left, &self.right)
            .map_err(|e| format!("{}: {}", self.error, e.format_with_decimals(&self.decimals)))?)
    }
}

impl Cheatcode for assertApproxEqAbs_0Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok(uint_assert_approx_eq_abs(self.left, self.right, self.maxDelta)
            .map_err(|e| format!("Assertion failed: {}", e))?)
    }
}

impl Cheatcode for assertApproxEqAbs_1Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok(uint_assert_approx_eq_abs(self.left, self.right, self.maxDelta)
            .map_err(|e| format!("{}: {}", self.error, e))?)
    }
}

impl Cheatcode for assertApproxEqAbs_2Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok(int_assert_approx_eq_abs(self.left, self.right, self.maxDelta)
            .map_err(|e| format!("Assertion failed: {}", e))?)
    }
}

impl Cheatcode for assertApproxEqAbs_3Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok(int_assert_approx_eq_abs(self.left, self.right, self.maxDelta)
            .map_err(|e| format!("{}: {}", self.error, e))?)
    }
}

impl Cheatcode for assertApproxEqAbsDecimal_0Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok(uint_assert_approx_eq_abs(self.left, self.right, self.maxDelta)
            .map_err(|e| format!("Assertion failed: {}", e.format_with_decimals(&self.decimals)))?)
    }
}

impl Cheatcode for assertApproxEqAbsDecimal_1Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok(uint_assert_approx_eq_abs(self.left, self.right, self.maxDelta)
            .map_err(|e| format!("{}: {}", self.error, e.format_with_decimals(&self.decimals)))?)
    }
}

impl Cheatcode for assertApproxEqAbsDecimal_2Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok(int_assert_approx_eq_abs(self.left, self.right, self.maxDelta)
            .map_err(|e| format!("Assertion failed: {}", e.format_with_decimals(&self.decimals)))?)
    }
}

impl Cheatcode for assertApproxEqAbsDecimal_3Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok(int_assert_approx_eq_abs(self.left, self.right, self.maxDelta)
            .map_err(|e| format!("{}: {}", self.error, e.format_with_decimals(&self.decimals)))?)
    }
}

impl Cheatcode for assertApproxEqRel_0Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok(uint_assert_approx_eq_rel(self.left, self.right, self.maxPercentDelta)
            .map_err(|e| format!("Assertion failed: {}", e))?)
    }
}

impl Cheatcode for assertApproxEqRel_1Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok(uint_assert_approx_eq_rel(self.left, self.right, self.maxPercentDelta)
            .map_err(|e| format!("{}: {}", self.error, e))?)
    }
}

impl Cheatcode for assertApproxEqRel_2Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok(int_assert_approx_eq_rel(self.left, self.right, self.maxPercentDelta)
            .map_err(|e| format!("Assertion failed: {}", e))?)
    }
}

impl Cheatcode for assertApproxEqRel_3Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok(int_assert_approx_eq_rel(self.left, self.right, self.maxPercentDelta)
            .map_err(|e| format!("{}: {}", self.error, e))?)
    }
}

impl Cheatcode for assertApproxEqRelDecimal_0Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok(uint_assert_approx_eq_rel(self.left, self.right, self.maxPercentDelta)
            .map_err(|e| format!("Assertion failed: {}", e.format_with_decimals(&self.decimals)))?)
    }
}

impl Cheatcode for assertApproxEqRelDecimal_1Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok(uint_assert_approx_eq_rel(self.left, self.right, self.maxPercentDelta)
            .map_err(|e| format!("{}: {}", self.error, e.format_with_decimals(&self.decimals)))?)
    }
}

impl Cheatcode for assertApproxEqRelDecimal_2Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok(int_assert_approx_eq_rel(self.left, self.right, self.maxPercentDelta)
            .map_err(|e| format!("Assertion failed: {}", e.format_with_decimals(&self.decimals)))?)
    }
}

impl Cheatcode for assertApproxEqRelDecimal_3Call {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok(int_assert_approx_eq_rel(self.left, self.right, self.maxPercentDelta)
            .map_err(|e| format!("{}: {}", self.error, e.format_with_decimals(&self.decimals)))?)
    }
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
    let delta = get_delta_uint(left, right)
        .checked_mul(U256::pow(U256::from(10), EQ_REL_DELTA_RESOLUTION))
        .ok_or(EqRelAssertionError::Overflow)?
        .checked_div(right)
        .ok_or(EqRelAssertionError::Overflow)?;

    if delta <= max_delta {
        Ok(Default::default())
    } else {
        Err(EqRelAssertionError::Failure(Box::new(EqRelAssertionFailure {
            left,
            right,
            max_delta,
            real_delta: delta,
        })))
    }
}

fn int_assert_approx_eq_rel(
    left: I256,
    right: I256,
    max_delta: U256,
) -> Result<Vec<u8>, EqRelAssertionError<I256>> {
    let (_, abs_right) = right.into_sign_and_abs();
    let delta = get_delta_int(left, right)
        .checked_mul(U256::pow(U256::from(10), EQ_REL_DELTA_RESOLUTION))
        .ok_or(EqRelAssertionError::Overflow)?
        .checked_div(abs_right)
        .ok_or(EqRelAssertionError::Overflow)?;

    if delta <= max_delta {
        Ok(Default::default())
    } else {
        Err(EqRelAssertionError::Failure(Box::new(EqRelAssertionFailure {
            left,
            right,
            max_delta,
            real_delta: delta,
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
