//! Commonly used calculations.

use alloy_primitives::U256;
use std::ops::Div;

/// Returns the mean of the slice
#[inline]
pub fn mean(values: &[U256]) -> U256 {
    if values.is_empty() {
        return U256::ZERO
    }

    values.iter().copied().fold(U256::ZERO, |sum, val| sum + val).div(U256::from(values.len()))
}

/// Returns the median of a _sorted_ slice
#[inline]
pub fn median_sorted(values: &[U256]) -> U256 {
    if values.is_empty() {
        return U256::ZERO
    }

    let len = values.len();
    let mid = len / 2;
    if len % 2 == 0 {
        (values[mid - 1] + values[mid]) / U256::from(2u64)
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
pub fn to_exp_notation(value: U256, precision: usize, trim_end_zeros: bool) -> String {
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

    format!("{mantissa}e{exponent}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calc_mean_empty() {
        let values: [U256; 0] = [];
        let m = mean(&values);
        assert_eq!(m, U256::ZERO);
    }

    #[test]
    fn calc_mean() {
        let values = [
            U256::ZERO,
            U256::from(1),
            U256::from(2u64),
            U256::from(3u64),
            U256::from(4u64),
            U256::from(5u64),
            U256::from(6u64),
        ];
        let m = mean(&values);
        assert_eq!(m, U256::from(3u64));
    }

    #[test]
    fn calc_median_empty() {
        let values: Vec<U256> = vec![];
        let m = median_sorted(&values);
        assert_eq!(m, U256::from(0));
    }

    #[test]
    fn calc_median() {
        let mut values =
            vec![29, 30, 31, 40, 59, 61, 71].into_iter().map(U256::from).collect::<Vec<_>>();
        values.sort();
        let m = median_sorted(&values);
        assert_eq!(m, U256::from(40));
    }

    #[test]
    fn calc_median_even() {
        let mut values =
            vec![80, 90, 30, 40, 50, 60, 10, 20].into_iter().map(U256::from).collect::<Vec<_>>();
        values.sort();
        let m = median_sorted(&values);
        assert_eq!(m, U256::from(45));
    }

    #[test]
    fn test_format_to_exponential_notation() {
        let value = 1234124124u64;

        let formatted = to_exp_notation(U256::from(value), 4, false);
        assert_eq!(formatted, "1.234e9");

        let formatted = to_exp_notation(U256::from(value), 3, true);
        assert_eq!(formatted, "1.23e9");

        let value = 10000000u64;

        let formatted = to_exp_notation(U256::from(value), 4, false);
        assert_eq!(formatted, "1.000e7");

        let formatted = to_exp_notation(U256::from(value), 3, true);
        assert_eq!(formatted, "1e7");
    }
}
