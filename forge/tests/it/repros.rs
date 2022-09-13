//! Tests for reproducing issues

use crate::{config::*, test_helpers::filter::Filter};
use forge::result::SuiteResult;
use foundry_evm::decode::decode_console_logs;

// <https://github.com/foundry-rs/foundry/issues/2623>
#[test]
fn test_issue_2623() {
    let mut runner = runner();
    let suite_result =
        runner.test(&Filter::new(".*", ".*", ".*repros/Issue2623"), None, TEST_OPTS).unwrap();
    assert!(!suite_result.is_empty());

    for (_, SuiteResult { test_results, .. }) in suite_result {
        for (test_name, result) in test_results {
            let logs = decode_console_logs(&result.logs);
            assert!(
                result.success,
                "Test {} did not pass as expected.\nReason: {:?}\nLogs:\n{}",
                test_name,
                result.reason,
                logs.join("\n")
            );
        }
    }
}

// <https://github.com/foundry-rs/foundry/issues/2629>
#[test]
fn test_issue_2629() {
    let mut runner = runner();
    let suite_result =
        runner.test(&Filter::new(".*", ".*", ".*repros/Issue2629"), None, TEST_OPTS).unwrap();
    assert!(!suite_result.is_empty());

    for (_, SuiteResult { test_results, .. }) in suite_result {
        for (test_name, result) in test_results {
            let logs = decode_console_logs(&result.logs);
            assert!(
                result.success,
                "Test {} did not pass as expected.\nReason: {:?}\nLogs:\n{}",
                test_name,
                result.reason,
                logs.join("\n")
            );
        }
    }
}

// <https://github.com/foundry-rs/foundry/issues/2723>
#[test]
fn test_issue_2723() {
    let mut runner = runner();
    let suite_result =
        runner.test(&Filter::new(".*", ".*", ".*repros/Issue2723"), None, TEST_OPTS).unwrap();
    assert!(!suite_result.is_empty());

    for (_, SuiteResult { test_results, .. }) in suite_result {
        for (test_name, result) in test_results {
            let logs = decode_console_logs(&result.logs);
            assert!(
                result.success,
                "Test {} did not pass as expected.\nReason: {:?}\nLogs:\n{}",
                test_name,
                result.reason,
                logs.join("\n")
            );
        }
    }
}

// <https://github.com/foundry-rs/foundry/issues/2898>
#[test]
fn test_issue_2898() {
    let mut runner = runner();
    let suite_result =
        runner.test(&Filter::new(".*", ".*", ".*repros/Issue2898"), None, TEST_OPTS).unwrap();
    assert!(!suite_result.is_empty());

    for (_, SuiteResult { test_results, .. }) in suite_result {
        for (test_name, result) in test_results {
            let logs = decode_console_logs(&result.logs);
            assert!(
                result.success,
                "Test {} did not pass as expected.\nReason: {:?}\nLogs:\n{}",
                test_name,
                result.reason,
                logs.join("\n")
            );
        }
    }
}

// <https://github.com/foundry-rs/foundry/issues/2956>
#[test]
fn test_issue_2956() {
    let mut runner = runner();
    let suite_result =
        runner.test(&Filter::new(".*", ".*", ".*repros/Issue2956"), None, TEST_OPTS).unwrap();
    assert!(!suite_result.is_empty());

    for (_, SuiteResult { test_results, .. }) in suite_result {
        for (test_name, result) in test_results {
            let logs = decode_console_logs(&result.logs);
            assert!(
                result.success,
                "Test {} did not pass as expected.\nReason: {:?}\nLogs:\n{}",
                test_name,
                result.reason,
                logs.join("\n")
            );
        }
    }
}

// <https://github.com/foundry-rs/foundry/issues/2984>
#[test]
fn test_issue_2984() {
    let mut runner = runner();
    let suite_result =
        runner.test(&Filter::new(".*", ".*", ".*repros/Issue2984"), None, TEST_OPTS).unwrap();
    assert!(!suite_result.is_empty());

    for (_, SuiteResult { test_results, .. }) in suite_result {
        for (test_name, result) in test_results {
            let logs = decode_console_logs(&result.logs);
            assert!(
                result.success,
                "Test {} did not pass as expected.\nReason: {:?}\nLogs:\n{}",
                test_name,
                result.reason,
                logs.join("\n")
            );
        }
    }
}

// <https://github.com/foundry-rs/foundry/issues/3077>
#[test]
fn test_issue_3077() {
    let mut runner = runner();
    let suite_result =
        runner.test(&Filter::new(".*", ".*", ".*repros/Issue3077"), None, TEST_OPTS).unwrap();
    assert!(!suite_result.is_empty());

    for (_, SuiteResult { test_results, .. }) in suite_result {
        for (test_name, result) in test_results {
            let logs = decode_console_logs(&result.logs);
            assert!(
                result.success,
                "Test {} did not pass as expected.\nReason: {:?}\nLogs:\n{}",
                test_name,
                result.reason,
                logs.join("\n")
            );
        }
    }
}

// <https://github.com/foundry-rs/foundry/issues/3055>
#[test]
fn test_issue_3055() {
    let mut runner = runner();
    let suite_result =
        runner.test(&Filter::new(".*", ".*", ".*repros/Issue3055"), None, TEST_OPTS).unwrap();
    assert!(!suite_result.is_empty());

    for (_, SuiteResult { test_results, .. }) in suite_result {
        for (test_name, result) in test_results {
            let logs = decode_console_logs(&result.logs);
            assert!(
                !result.success,
                "Test {} did not fail as expected.\nReason: {:?}\nLogs:\n{}",
                test_name,
                result.reason,
                logs.join("\n")
            );
        }
    }
}

// <https://github.com/foundry-rs/foundry/issues/3110>
#[test]
fn test_issue_3110() {
    let mut runner = runner();
    let suite_result =
        runner.test(&Filter::new(".*", ".*", ".*repros/Issue3110"), None, TEST_OPTS).unwrap();
    assert!(!suite_result.is_empty());

    for (_, SuiteResult { test_results, .. }) in suite_result {
        for (test_name, result) in test_results {
            let logs = decode_console_logs(&result.logs);
            assert!(
                result.success,
                "Test {} did not pass as expected.\nReason: {:?}\nLogs:\n{}",
                test_name,
                result.reason,
                logs.join("\n")
            );
        }
    }
}

// <https://github.com/foundry-rs/foundry/issues/3119>
#[test]
fn test_issue_3119() {
    let mut runner = runner();
    let suite_result =
        runner.test(&Filter::new(".*", ".*", ".*repros/Issue3119"), None, TEST_OPTS).unwrap();
    assert!(!suite_result.is_empty());

    for (_, SuiteResult { test_results, .. }) in suite_result {
        for (test_name, result) in test_results {
            let logs = decode_console_logs(&result.logs);
            assert!(
                result.success,
                "Test {} did not pass as expected.\nReason: {:?}\nLogs:\n{}",
                test_name,
                result.reason,
                logs.join("\n")
            );
        }
    }
}

// <https://github.com/foundry-rs/foundry/issues/3190>
#[test]
fn test_issue_3190() {
    let mut runner = runner();
    let suite_result =
        runner.test(&Filter::new(".*", ".*", ".*repros/Issue3190"), None, TEST_OPTS).unwrap();
    assert!(!suite_result.is_empty());

    for (_, SuiteResult { test_results, .. }) in suite_result {
        for (test_name, result) in test_results {
            let logs = decode_console_logs(&result.logs);
            assert!(
                result.success,
                "Test {} did not pass as expected.\nReason: {:?}\nLogs:\n{}",
                test_name,
                result.reason,
                logs.join("\n")
            );
        }
    }
}
