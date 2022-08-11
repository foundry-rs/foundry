//! Tests for invariants

use crate::{config::*, test_helpers::filter::Filter};
use forge::result::SuiteResult;
use foundry_evm::decode::decode_console_logs;
use std::collections::BTreeMap;

#[test]
fn test_fuzz() {
    let mut runner = runner();

    let suite_result =
        runner.test(&Filter::new(".*", ".*", ".*fuzz/[^invariant]"), None, TEST_OPTS).unwrap();

    assert!(!suite_result.is_empty());

    for (_, SuiteResult { test_results, .. }) in suite_result {
        for (test_name, result) in test_results {
            let logs = decode_console_logs(&result.logs);

            match test_name.as_str() {
                "testPositive(uint256)" |
                "testPositive(int256)" |
                "testSuccessfulFuzz(uint128,uint128)" |
                "testToStringFuzz(bytes32)" => assert!(
                    result.success,
                    "Test {} did not pass as expected.\nReason: {:?}\nLogs:\n{}",
                    test_name,
                    result.reason,
                    logs.join("\n")
                ),
                _ => assert!(
                    !result.success,
                    "Test {} did not fail as expected.\nReason: {:?}\nLogs:\n{}",
                    test_name,
                    result.reason,
                    logs.join("\n")
                ),
            }
        }
    }
}

#[test]
fn test_fuzz_collection() {
    let mut runner = runner();

    let mut opts = TEST_OPTS;
    opts.invariant_depth = 200;
    opts.fuzz_runs = 1000;
    runner.test_options = opts;

    let results =
        runner.test(&Filter::new(".*", ".*", ".*fuzz/FuzzCollection.t.sol"), None, opts).unwrap();

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
