//! Fuzz tests.

use crate::config::*;
use alloy_primitives::U256;
use forge::result::{SuiteResult, TestStatus};
use foundry_test_utils::Filter;
use std::collections::BTreeMap;

#[tokio::test(flavor = "multi_thread")]
async fn test_fuzz() {
    let filter = Filter::new(".*", ".*", ".*fuzz/")
        .exclude_tests(r"invariantCounter|testIncrement\(address\)|testNeedle\(uint256\)|testSuccessChecker\(uint256\)|testSuccessChecker2\(int256\)|testSuccessChecker3\(uint32\)")
        .exclude_paths("invariant");
    let mut runner = runner().await;
    let suite_result = runner.test_collect(&filter).await;

    assert!(!suite_result.is_empty());

    for (_, SuiteResult { test_results, .. }) in suite_result {
        for (test_name, result) in test_results {
            match test_name.as_str() {
                "testPositive(uint256)" |
                "testPositive(int256)" |
                "testSuccessfulFuzz(uint128,uint128)" |
                "testToStringFuzz(bytes32)" => assert_eq!(
                    result.status,
                    TestStatus::Success,
                    "Test {} did not pass as expected.\nReason: {:?}\nLogs:\n{}",
                    test_name,
                    result.reason,
                    result.decoded_logs.join("\n")
                ),
                _ => assert_eq!(
                    result.status,
                    TestStatus::Failure,
                    "Test {} did not fail as expected.\nReason: {:?}\nLogs:\n{}",
                    test_name,
                    result.reason,
                    result.decoded_logs.join("\n")
                ),
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_successful_fuzz_cases() {
    let filter = Filter::new(".*", ".*", ".*fuzz/FuzzPositive")
        .exclude_tests(r"invariantCounter|testIncrement\(address\)|testNeedle\(uint256\)")
        .exclude_paths("invariant");
    let mut runner = runner().await;
    let suite_result = runner.test_collect(&filter).await;

    assert!(!suite_result.is_empty());

    for (_, SuiteResult { test_results, .. }) in suite_result {
        for (test_name, result) in test_results {
            match test_name.as_str() {
                "testSuccessChecker(uint256)" |
                "testSuccessChecker2(int256)" |
                "testSuccessChecker3(uint32)" => assert_eq!(
                    result.status,
                    TestStatus::Success,
                    "Test {} did not pass as expected.\nReason: {:?}\nLogs:\n{}",
                    test_name,
                    result.reason,
                    result.decoded_logs.join("\n")
                ),
                _ => {}
            }
        }
    }
}

/// Test that showcases PUSH collection on normal fuzzing. Ignored until we collect them in a
/// smarter way.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_fuzz_collection() {
    let filter = Filter::new(".*", ".*", ".*fuzz/FuzzCollection.t.sol");
    let mut runner = runner().await;
    runner.test_options.invariant.depth = 100;
    runner.test_options.invariant.runs = 1000;
    runner.test_options.fuzz.runs = 1000;
    runner.test_options.fuzz.seed = Some(U256::from(6u32));
    let results = runner.test_collect(&filter).await;

    assert_multiple(
        &results,
        BTreeMap::from([(
            "fuzz/FuzzCollection.t.sol:SampleContractTest",
            vec![
                ("invariantCounter", false, Some("broken counter.".into()), None, None),
                (
                    "testIncrement(address)",
                    false,
                    Some("Call did not revert as expected".into()),
                    None,
                    None,
                ),
                ("testNeedle(uint256)", false, Some("needle found.".into()), None, None),
            ],
        )]),
    );
}
