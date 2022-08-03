//! commonly used calculations

use ethers_core::types::U256;
use std::ops::{Add, Div};

/// Returns the mean of the slice
#[inline]
pub fn mean<T>(values: &[T]) -> U256
where
    T: Into<U256> + Copy,
{
    values.iter().copied().fold(U256::zero(), |sum, val| sum + val.into()) / values.len()
}

/// Returns the median of a _sorted_ slice
#[inline]
pub fn median_sorted<T>(values: &[T]) -> T
where
    T: Add<Output = T> + Div<u64, Output = T> + From<u64> + Copy,
{
    let len = values.len();
    if len > 0 {
        if len % 2 == 0 {
            (values[len / 2 - 1] + values[len / 2]) / 2u64
        } else {
            values[len / 2]
        }
    } else {
        0u64.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calc_mean() {
        let values = [0u64, 1u64, 2u64, 3u64, 4u64, 5u64, 6u64];
        let m = mean(&values);
        assert_eq!(m, 3u64.into());
    }
}
