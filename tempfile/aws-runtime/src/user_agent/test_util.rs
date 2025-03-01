/*
 * Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! Utilities for testing the User-Agent header

use once_cell::sync::Lazy;
use regex_lite::Regex;

// regular expression pattern for base64 numeric values
#[allow(dead_code)]
static RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"m/([A-Za-z0-9+/=_,-]+)").unwrap());

/// Asserts `user_agent` contains all metric values `values`
///
/// Refer to the end of the parent module file `user_agent.rs` for the complete ABNF specification
/// of `business-metrics`.
pub fn assert_ua_contains_metric_values(user_agent: &str, values: &[&str]) {
    match extract_ua_values(user_agent) {
        Some(metrics) => {
            let mut missed = vec![];

            for value in values.iter() {
                if !metrics.contains(value) {
                    missed.push(value);
                }
            }
            assert!(
                missed.is_empty(),
                "{}",
                format!("metric values {missed:?} not found in `{user_agent}`")
            );
        }
        None => {
            panic!("{}", format!("the pattern for business-metrics `m/(metric_id) *(comma metric_id)` not found in `{user_agent}`"))
        }
    }
}

/// Extract the metric values from the `user_agent` string
pub fn extract_ua_values(user_agent: &str) -> Option<Vec<&str>> {
    RE.find(user_agent).map(|matched| {
        matched
            .as_str()
            .strip_prefix("m/")
            .expect("prefix `m/` is guaranteed to exist by regex match")
            .split(',')
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_assert_ua_contains_metric_values() {
        assert_ua_contains_metric_values("m/A", &[]);
        assert_ua_contains_metric_values("m/A", &["A"]);
        assert_ua_contains_metric_values(" m/A", &["A"]);
        assert_ua_contains_metric_values("m/A ", &["A"]);
        assert_ua_contains_metric_values(" m/A ", &["A"]);
        assert_ua_contains_metric_values("m/A,B", &["B"]);
        assert_ua_contains_metric_values("m/A,B", &["A", "B"]);
        assert_ua_contains_metric_values("m/A,B", &["B", "A"]);
        assert_ua_contains_metric_values("m/A,B,C", &["B"]);
        assert_ua_contains_metric_values("m/A,B,C", &["B", "C"]);
        assert_ua_contains_metric_values("m/A,B,C", &["A", "B", "C"]);
        assert_ua_contains_metric_values("m/A,B,C,AA", &["AA"]);
        assert_ua_contains_metric_values("m/A,B,C=,AA", &["C="]);
        assert_ua_contains_metric_values(
            "aws-sdk-rust/0.123.test api/test-service/0.123 os/windows/XPSP3 lang/rust/1.50.0 m/A",
            &["A"],
        );
        assert_ua_contains_metric_values(
            "aws-sdk-rust/0.123.test api/test-service/0.123 os/windows/XPSP3 lang/rust/1.50.0 m/A md/http#capture-request-handler",
            &["A"]
        );
    }

    #[test]
    #[should_panic(expected = "the pattern for business-metrics")]
    fn empty_ua_fails_assert() {
        assert_ua_contains_metric_values("", &["A"]);
    }

    #[test]
    #[should_panic(expected = "the pattern for business-metrics")]
    fn invalid_business_metrics_pattern_fails_assert() {
        assert_ua_contains_metric_values("mA", &["A"]);
    }

    #[test]
    #[should_panic(expected = "the pattern for business-metrics")]
    fn another_invalid_business_metrics_pattern_fails_assert() {
        assert_ua_contains_metric_values("m/", &["A"]);
    }

    #[test]
    #[should_panic(expected = "metric values [\"\"] not found in `m/A`")]
    fn empty_metric_value_fails_assert() {
        assert_ua_contains_metric_values("m/A", &[""]);
    }

    #[test]
    #[should_panic(expected = "metric values [\"A\"] not found in `m/AA`")]
    fn business_metrics_do_not_contain_given_metric_value() {
        assert_ua_contains_metric_values("m/AA", &["A"]);
    }

    #[test]
    #[should_panic(expected = "the pattern for business-metrics")]
    fn ua_containing_no_business_metrics_fails_assert() {
        assert_ua_contains_metric_values(
            "aws-sdk-rust/0.123.test api/test-service/0.123 os/windows/XPSP3 lang/rust/1.50.0",
            &["A"],
        );
    }

    #[test]
    #[should_panic(expected = "the pattern for business-metrics")]
    fn ua_containing_invalid_business_metrics_fails_assert() {
        assert_ua_contains_metric_values(
            "aws-sdk-rust/0.123.test api/test-service/0.123 os/windows/XPSP3 lang/rust/1.50.0 mA",
            &["A"],
        );
    }
}
