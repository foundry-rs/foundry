//! commonly used calculations

use ethers_core::types::U256;
use std::ops::{Add, Div};

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
}
