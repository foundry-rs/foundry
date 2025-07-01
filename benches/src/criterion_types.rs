use serde::{Deserialize, Serialize};

/// Benchmark result from Criterion JSON output
///
/// This is a simplified version containing only the fields we actually use
#[derive(Debug, Deserialize, Serialize)]
pub struct CriterionResult {
    /// Unique identifier for the benchmark result (format: benchmark-name/version/repo)
    pub id: String,
    /// Mean performance estimate
    pub mean: Estimate,
    /// Unit of measurement (always "ns" for nanoseconds in our case)
    pub unit: String,
    /// Performance change data compared to baseline (if available)
    pub change: Option<Change>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Estimate {
    pub confidence_interval: ConfidenceInterval,
    pub point_estimate: f64,
    pub standard_error: f64,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ConfidenceInterval {
    pub confidence_level: f64,
    pub lower_bound: f64,
    pub upper_bound: f64,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Change {
    pub mean: Option<ChangeEstimate>,
    pub median: Option<ChangeEstimate>,
    pub change: Option<String>, // "NoChange", "Improved", or "Regressed"
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ChangeEstimate {
    pub estimate: f64,
    pub unit: String,
}
