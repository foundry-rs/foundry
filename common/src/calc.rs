//! commonly used calculations

use ethers_core::types::U256;
use std::{
    fmt::Display,
    ops::{Add, Div},
};

/// Returns the mean of the slice
#[inline]
pub fn mean<T>(values: &[T]) -> U256
where
    T: Into<U256> + Copy,
{
    if values.is_empty() {
        return U256::zero()
    }

    values.iter().copied().fold(U256::zero(), |sum, val| sum + val.into()) / values.len()
}

/// Returns the median of a _sorted_ slice
#[inline]
pub fn median_sorted<T>(values: &[T]) -> T
where
    T: Add<Output = T> + Div<u64, Output = T> + From<u64> + Copy,
{
    if values.is_empty() {
        return 0u64.into()
    }

    let len = values.len();
    let mid = len / 2;
    if len % 2 == 0 {
        (values[mid - 1] + values[mid]) / 2u64
    } else {
        values[mid]
    }
}

/// Returns the number expressed as a string in exponential notation
/// with the given precision (number of significant figures),
/// optionally removing trailing zeros from the mantissa.
///
/// Examples:
///
/// ```text
/// precision = 4, trim_end_zeroes = false
///     1234124124 -> 1.234e9
///     10000000 -> 1.000e7
/// precision = 3, trim_end_zeroes = true
///     1234124124 -> 1.23e9
///     10000000 -> 1e7
/// ```
#[inline]
pub fn to_exponential_notation<T>(value: T, precision: usize, trim_end_zeros: bool) -> String
where
    T: Into<U256> + Display,
{
    let stringified = value.to_string();
    let exponent = stringified.len() - 1;
    let mut mantissa = stringified.chars().take(precision).collect::<String>();

    // optionally remove trailing zeros
    if trim_end_zeros {
        mantissa = mantissa.trim_end_matches('0').to_string();
    }

    // Place a decimal point only if needed
    // e.g. 1234 -> 1.234e3 (needed)
    //      5 -> 5 (not needed)
    if mantissa.len() > 1 {
        mantissa.insert(1, '.');
    }

    format!("{}e{}", mantissa, exponent)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calc_mean_empty() {
        let values: [u64; 0] = [];
        let m = mean(&values);
        assert_eq!(m, U256::zero());
    }

    #[test]
    fn calc_mean() {
        let values = [0u64, 1u64, 2u64, 3u64, 4u64, 5u64, 6u64];
        let m = mean(&values);
        assert_eq!(m, 3u64.into());
    }

    #[test]
    fn calc_median_empty() {
        let values: Vec<u64> = vec![];
        let m = median_sorted(&values);
        assert_eq!(m, 0);
    }

    #[test]
    fn calc_median() {
        let mut values = vec![29, 30, 31, 40, 59, 61, 71];
        values.sort();
        let m = median_sorted(&values);
        assert_eq!(m, 40);
    }

    #[test]
    fn calc_median_even() {
        let mut values = vec![80, 90, 30, 40, 50, 60, 10, 20];
        values.sort();
        let m = median_sorted(&values);
        assert_eq!(m, 45);
    }

    #[test]
    fn test_format_to_exponential_notation() {
        let value = 1234124124u64;

        let formatted = to_exponential_notation(value, 4, false);
        assert_eq!(formatted, "1.234e9");

        let formatted = to_exponential_notation(value, 3, true);
        assert_eq!(formatted, "1.23e9");

        let value = 10000000u64;

        let formatted = to_exponential_notation(value, 4, false);
        assert_eq!(formatted, "1.000e7");

        let formatted = to_exponential_notation(value, 3, true);
        assert_eq!(formatted, "1e7");
    }
}
