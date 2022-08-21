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
