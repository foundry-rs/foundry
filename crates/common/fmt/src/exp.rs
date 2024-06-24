use alloy_primitives::{Sign, U256};

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
pub fn to_exp_notation(value: U256, precision: usize, trim_end_zeros: bool, sign: Sign) -> String {
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

    format!("{sign}{mantissa}e{exponent}")
}
