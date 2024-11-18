//! Commonly used calculations.

/// Returns the mean of the slice.
#[inline]
pub fn mean(values: &[u64]) -> u64 {
    if values.is_empty() {
        return 0;
    }

    (values.iter().map(|x| *x as u128).sum::<u128>() / values.len() as u128) as u64
}

/// Returns the median of a _sorted_ slice.
#[inline]
pub fn median_sorted(values: &[u64]) -> u64 {
    if values.is_empty() {
        return 0;
    }

    let len = values.len();
    let mid = len / 2;
    if len % 2 == 0 {
        (values[mid - 1] + values[mid]) / 2
    } else {
        values[mid]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calc_mean_empty() {
        let m = mean(&[]);
        assert_eq!(m, 0);
    }

    #[test]
    fn calc_mean() {
        let m = mean(&[0, 1, 2, 3, 4, 5, 6]);
        assert_eq!(m, 3);
    }

    #[test]
    fn calc_mean_overflow() {
        let m = mean(&[0, 1, 2, u32::MAX as u64, 3, u16::MAX as u64, u64::MAX, 6]);
        assert_eq!(m, 2305843009750573057);
    }

    #[test]
    fn calc_median_empty() {
        let m = median_sorted(&[]);
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
