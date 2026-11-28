//! Summary statistics over per-seed samples.
//!
//! All intervals are two-sided 95% Student t-intervals. Seeds are the unit of replication, so a
//! sample always has one value per seed.

use serde::{Deserialize, Serialize};

/// Two-sided 95% Student t critical values for 1..=30 degrees of freedom.
const T_CRITICAL_95: [f64; 30] = [
    12.706, 4.303, 3.182, 2.776, 2.571, 2.447, 2.365, 2.306, 2.262, 2.228, 2.201, 2.179, 2.160,
    2.145, 2.131, 2.120, 2.110, 2.101, 2.093, 2.086, 2.080, 2.074, 2.069, 2.064, 2.060, 2.056,
    2.052, 2.048, 2.045, 2.042,
];

/// Mean, sample standard deviation, and 95% confidence interval of a sample.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Estimate {
    /// Number of samples.
    pub n: usize,
    /// Sample mean.
    pub mean: f64,
    /// Sample standard deviation (`n - 1` denominator); zero for a single sample.
    pub std_dev: f64,
    /// Lower bound of the 95% t-interval, when `n >= 2`.
    pub ci_low: Option<f64>,
    /// Upper bound of the 95% t-interval, when `n >= 2`.
    pub ci_high: Option<f64>,
}

impl Estimate {
    /// Estimates the mean of `samples`. Returns `None` for an empty sample.
    pub fn from_samples(samples: &[f64]) -> Option<Self> {
        let n = samples.len();
        if n == 0 {
            return None;
        }
        let mean = samples.iter().sum::<f64>() / n as f64;
        if n == 1 {
            return Some(Self { n, mean, std_dev: 0.0, ci_low: None, ci_high: None });
        }
        let variance = samples.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1) as f64;
        let std_dev = variance.sqrt();
        let half_width = t_critical_95(n - 1) * std_dev / (n as f64).sqrt();
        Some(Self {
            n,
            mean,
            std_dev,
            ci_low: Some(mean - half_width),
            ci_high: Some(mean + half_width),
        })
    }

    /// Returns whether both intervals exist and overlap. Single-sample estimates have no
    /// interval and are treated as overlapping, since they cannot show a difference.
    pub fn overlaps(&self, other: &Self) -> bool {
        match (self.ci_low, self.ci_high, other.ci_low, other.ci_high) {
            (Some(low), Some(high), Some(other_low), Some(other_high)) => {
                low <= other_high && other_low <= high
            }
            _ => true,
        }
    }
}

/// Estimated difference `current - baseline` between two samples.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Difference {
    /// Difference of the means.
    pub delta: f64,
    /// Lower bound of the 95% interval of the difference.
    pub ci_low: Option<f64>,
    /// Upper bound of the 95% interval of the difference.
    pub ci_high: Option<f64>,
    /// Whether the interval was computed from per-seed paired differences.
    pub paired: bool,
}

impl Difference {
    /// Estimates `current - baseline`.
    ///
    /// When both samples cover the same seeds (`paired`), this uses a paired t-interval over
    /// per-seed differences. Otherwise it uses Welch's t-interval with Welch-Satterthwaite degrees
    /// of freedom.
    pub fn between(baseline: &[f64], current: &[f64], paired: bool) -> Option<Self> {
        if paired && baseline.len() == current.len() {
            let diffs = baseline.iter().zip(current).map(|(b, c)| c - b).collect::<Vec<_>>();
            let estimate = Estimate::from_samples(&diffs)?;
            return Some(Self {
                delta: estimate.mean,
                ci_low: estimate.ci_low,
                ci_high: estimate.ci_high,
                paired: true,
            });
        }
        let base = Estimate::from_samples(baseline)?;
        let cur = Estimate::from_samples(current)?;
        let delta = cur.mean - base.mean;
        if base.n < 2 || cur.n < 2 {
            return Some(Self { delta, ci_low: None, ci_high: None, paired: false });
        }
        let base_var = base.std_dev.powi(2) / base.n as f64;
        let cur_var = cur.std_dev.powi(2) / cur.n as f64;
        let se = (base_var + cur_var).sqrt();
        if se == 0.0 {
            return Some(Self { delta, ci_low: Some(delta), ci_high: Some(delta), paired: false });
        }
        let df = (base_var + cur_var).powi(2)
            / (base_var.powi(2) / (base.n - 1) as f64 + cur_var.powi(2) / (cur.n - 1) as f64);
        let half_width = t_critical_95(df.floor().max(1.0) as usize) * se;
        Some(Self {
            delta,
            ci_low: Some(delta - half_width),
            ci_high: Some(delta + half_width),
            paired: false,
        })
    }

    /// Returns whether the whole interval lies above zero.
    pub fn is_increase(&self) -> bool {
        self.ci_low.is_some_and(|low| low > 0.0)
    }

    /// Returns whether the whole interval lies below zero.
    pub fn is_decrease(&self) -> bool {
        self.ci_high.is_some_and(|high| high < 0.0)
    }
}

/// Two-sided 95% Student t critical value for `df` degrees of freedom.
pub const fn t_critical_95(df: usize) -> f64 {
    match df {
        0 => f64::NAN,
        1..=30 => T_CRITICAL_95[df - 1],
        31..=40 => 2.021,
        41..=60 => 2.000,
        61..=120 => 1.980,
        _ => 1.960,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_close(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < 1e-3, "expected {expected}, got {actual}");
    }

    #[test]
    fn empty_sample_has_no_estimate() {
        assert_eq!(Estimate::from_samples(&[]), None);
    }

    #[test]
    fn single_sample_has_no_interval() {
        let estimate = Estimate::from_samples(&[3.0]).unwrap();
        assert_eq!(
            estimate,
            Estimate { n: 1, mean: 3.0, std_dev: 0.0, ci_low: None, ci_high: None }
        );
    }

    #[test]
    fn t_interval_matches_reference_values() {
        // mean 3, sd 1.5811, t(4) = 2.776 -> half width 1.9630
        let estimate = Estimate::from_samples(&[1.0, 2.0, 3.0, 4.0, 5.0]).unwrap();
        assert_eq!(estimate.n, 5);
        assert_close(estimate.mean, 3.0);
        assert_close(estimate.std_dev, 1.5811);
        assert_close(estimate.ci_low.unwrap(), 1.0370);
        assert_close(estimate.ci_high.unwrap(), 4.9630);
    }

    #[test]
    fn constant_sample_has_zero_width_interval() {
        let estimate = Estimate::from_samples(&[2.0, 2.0, 2.0]).unwrap();
        assert_eq!(estimate.ci_low, Some(2.0));
        assert_eq!(estimate.ci_high, Some(2.0));
    }

    #[test]
    fn overlap_detection() {
        let low = Estimate::from_samples(&[1.0, 1.1, 0.9]).unwrap();
        let high = Estimate::from_samples(&[5.0, 5.1, 4.9]).unwrap();
        let near = Estimate::from_samples(&[1.0, 1.2, 0.95]).unwrap();
        assert!(!low.overlaps(&high));
        assert!(low.overlaps(&near));
    }

    #[test]
    fn paired_difference_uses_per_seed_deltas() {
        // Diffs are 1, 1, 2 -> mean 1.3333, sd 0.5774, t(2) = 4.303 -> half width 1.4343.
        let diff = Difference::between(&[1.0, 2.0, 3.0], &[2.0, 3.0, 5.0], true).unwrap();
        assert!(diff.paired);
        assert_close(diff.delta, 1.3333);
        assert_close(diff.ci_low.unwrap(), -0.1010);
        assert_close(diff.ci_high.unwrap(), 2.7676);
        assert!(!diff.is_increase());
    }

    #[test]
    fn consistent_paired_improvement_is_an_increase() {
        let diff = Difference::between(&[1.0, 2.0, 3.0], &[2.0, 3.0, 4.0], true).unwrap();
        assert_eq!(diff.ci_low, Some(1.0));
        assert!(diff.is_increase());
        assert!(!diff.is_decrease());
    }

    #[test]
    fn welch_difference_for_unpaired_samples() {
        // Both variances 1, n = 3: se = 0.8165, df = 4, t = 2.776 -> half width 2.2666.
        let diff = Difference::between(&[1.0, 2.0, 3.0], &[5.0, 6.0, 7.0], false).unwrap();
        assert!(!diff.paired);
        assert_close(diff.delta, 4.0);
        assert_close(diff.ci_low.unwrap(), 1.7334);
        assert_close(diff.ci_high.unwrap(), 6.2666);
        assert!(diff.is_increase());
    }
}
