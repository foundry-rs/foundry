use serde::{Deserialize, Serialize};

/// Benchmark result from Criterion JSON output
#[derive(Debug, Deserialize, Serialize)]
pub struct CriterionResult {
    pub reason: String,
    pub id: Option<String>,
    pub report_directory: Option<String>,
    pub iteration_count: Option<Vec<f64>>,
    pub measured_values: Option<Vec<f64>>,
    pub unit: Option<String>,
    pub throughput: Option<Vec<Throughput>>,
    pub typical: Option<Estimate>,
    pub mean: Option<Estimate>,
    pub median: Option<Estimate>,
    pub slope: Option<Estimate>,
    pub change: Option<Change>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Throughput {
    pub per_iteration: u64,
    pub unit: String,
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
