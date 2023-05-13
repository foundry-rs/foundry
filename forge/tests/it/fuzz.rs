//! Tests for invariants

use crate::{config::*, test_helpers::filter::Filter};
use ethers::types::U256;
use forge::result::SuiteResult;
use std::collections::BTreeMap;

#[test]
fn test_fuzz() {
    let mut runner = runner();

    let suite_result = runner
        .test(
            &Filter::new(".*", ".*", ".*fuzz/")
                .exclude_tests(r#"invariantCounter|testIncrement\(address\)|testNeedle\(uint256\)"#)
                .exclude_paths("invariant"),
            None,
            test_opts(),
        )
        .unwrap();

    assert!(!suite_result.is_empty());

    for (_, SuiteResult { test_results, .. }) in suite_result {
        for (test_name, result) in test_results {
            match test_name.as_str() {
                "testPositive(uint256)" |
                "testPositive(int256)" |
                "testSuccessfulFuzz(uint128,uint128)" |
                "testToStringFuzz(bytes32)" => assert!(
                    result.success,
                    "Test {} did not pass as expected.\nReason: {:?}\nLogs:\n{}",
                    test_name,
                    result.reason,
                    result.decoded_logs.join("\n")
                ),
                _ => assert!(
                    !result.success,
                    "Test {} did not fail as expected.\nReason: {:?}\nLogs:\n{}",
                    test_name,
                    result.reason,
                    result.decoded_logs.join("\n")
                ),
            }
        }
    }
}

/// Test that showcases PUSH collection on normal fuzzing. Ignored until we collect them in a
/// smarter way.
#[test]
#[ignore]
fn test_fuzz_collection() {
    let mut runner = runner();

    let mut opts = test_opts();
    opts.invariant.depth = 100;
    opts.invariant.runs = 1000;
    opts.fuzz.runs = 1000;
    opts.fuzz.seed = Some(U256::from(6u32));
    runner.test_options = opts.clone();

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
