use alloy_primitives::{Sign, I256, U256};
use yansi::Paint;

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

/// Formats a U256 number to string, adding an exponential notation _hint_ if it
/// is larger than `10_000`, with a precision of `4` figures, and trimming the
/// trailing zeros.
///
/// # Examples
///
/// ```
/// use alloy_primitives::U256;
/// use foundry_common_fmt::format_uint_exp as f;
///
/// # yansi::disable();
/// assert_eq!(f(U256::from(0)), "0");
/// assert_eq!(f(U256::from(1234)), "1234");
/// assert_eq!(f(U256::from(1234567890)), "1234567890 [1.234e9]");
/// assert_eq!(f(U256::from(1000000000000000000_u128)), "1000000000000000000 [1e18]");
/// assert_eq!(f(U256::from(10000000000000000000000_u128)), "10000000000000000000000 [1e22]");
/// ```
pub fn format_uint_exp(num: U256) -> String {
    if num < U256::from(10_000) {
        return num.to_string()
    }

    let exp = to_exp_notation(num, 4, true, Sign::Positive);
    format!("{num} {}", format!("[{exp}]").dim())
}

/// Formats a U256 number to string, adding an exponential notation _hint_.
///
/// Same as [`format_uint_exp`].
///
/// # Examples
///
/// ```
/// use alloy_primitives::I256;
/// use foundry_common_fmt::format_int_exp as f;
///
/// # yansi::disable();
/// assert_eq!(f(I256::try_from(0).unwrap()), "0");
/// assert_eq!(f(I256::try_from(-1).unwrap()), "-1");
/// assert_eq!(f(I256::try_from(1234).unwrap()), "1234");
/// assert_eq!(f(I256::try_from(1234567890).unwrap()), "1234567890 [1.234e9]");
/// assert_eq!(f(I256::try_from(-1234567890).unwrap()), "-1234567890 [-1.234e9]");
/// assert_eq!(f(I256::try_from(1000000000000000000_u128).unwrap()), "1000000000000000000 [1e18]");
/// assert_eq!(
///     f(I256::try_from(10000000000000000000000_u128).unwrap()),
///     "10000000000000000000000 [1e22]"
/// );
/// assert_eq!(
///     f(I256::try_from(-10000000000000000000000_i128).unwrap()),
///     "-10000000000000000000000 [-1e22]"
/// );
/// ```
pub fn format_int_exp(num: I256) -> String {
    let (sign, abs) = num.into_sign_and_abs();
    if abs < U256::from(10_000) {
        return format!("{sign}{abs}");
    }

    let exp = to_exp_notation(abs, 4, true, sign);
    format!("{sign}{abs} {}", format!("[{exp}]").dim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_to_exponential_notation() {
        let value = 1234124124u64;

        let formatted = to_exp_notation(U256::from(value), 4, false, Sign::Positive);
        assert_eq!(formatted, "1.234e9");

        let formatted = to_exp_notation(U256::from(value), 3, true, Sign::Positive);
        assert_eq!(formatted, "1.23e9");

        let value = 10000000u64;

        let formatted = to_exp_notation(U256::from(value), 4, false, Sign::Positive);
        assert_eq!(formatted, "1.000e7");

        let formatted = to_exp_notation(U256::from(value), 3, true, Sign::Positive);
        assert_eq!(formatted, "1e7");
    }
}
