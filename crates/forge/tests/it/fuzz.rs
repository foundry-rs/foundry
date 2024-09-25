//! Fuzz tests.

use crate::{config::*, test_helpers::TEST_DATA_DEFAULT};
use alloy_primitives::{Bytes, U256};
use forge::{
    decode::decode_console_logs,
    fuzz::CounterExample,
    result::{SuiteResult, TestStatus},
};
use foundry_test_utils::{forgetest_init, str, Filter};
use std::collections::BTreeMap;

#[tokio::test(flavor = "multi_thread")]
async fn test_fuzz() {
    let filter = Filter::new(".*", ".*", ".*fuzz/")
        .exclude_tests(r"invariantCounter|testIncrement\(address\)|testNeedle\(uint256\)|testSuccessChecker\(uint256\)|testSuccessChecker2\(int256\)|testSuccessChecker3\(uint32\)|testStorageOwner\(address\)|testImmutableOwner\(address\)")
        .exclude_paths("invariant");
    let mut runner = TEST_DATA_DEFAULT.runner();
    let suite_result = runner.test_collect(&filter);

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
                    decode_console_logs(&result.logs).join("\n")
                ),
                _ => assert_eq!(
                    result.status,
                    TestStatus::Failure,
                    "Test {} did not fail as expected.\nReason: {:?}\nLogs:\n{}",
                    test_name,
                    result.reason,
                    decode_console_logs(&result.logs).join("\n")
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
    let mut runner = TEST_DATA_DEFAULT.runner();
    let suite_result = runner.test_collect(&filter);

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
                    decode_console_logs(&result.logs).join("\n")
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
    let mut runner = TEST_DATA_DEFAULT.runner();
    runner.test_options.invariant.depth = 100;
    runner.test_options.invariant.runs = 1000;
    runner.test_options.fuzz.runs = 1000;
    runner.test_options.fuzz.seed = Some(U256::from(6u32));
    let results = runner.test_collect(&filter);

    assert_multiple(
        &results,
        BTreeMap::from([(
            "default/fuzz/FuzzCollection.t.sol:SampleContractTest",
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

#[tokio::test(flavor = "multi_thread")]
async fn test_persist_fuzz_failure() {
    let filter = Filter::new(".*", ".*", ".*fuzz/FuzzFailurePersist.t.sol");
    let mut runner = TEST_DATA_DEFAULT.runner();
    runner.test_options.fuzz.runs = 1000;

    macro_rules! get_failure_result {
        () => {
            runner
                .test_collect(&filter)
                .get("default/fuzz/FuzzFailurePersist.t.sol:FuzzFailurePersistTest")
                .unwrap()
                .test_results
                .get("test_persist_fuzzed_failure(uint256,int256,address,bool,string,(address,uint256),address[])")
                .unwrap()
                .counterexample
                .clone()
        };
    }

    // record initial counterexample calldata
    let initial_counterexample = get_failure_result!();
    let initial_calldata = match initial_counterexample {
        Some(CounterExample::Single(counterexample)) => counterexample.calldata,
        _ => Bytes::new(),
    };

    // run several times and compare counterexamples calldata
    for i in 0..10 {
        let new_calldata = match get_failure_result!() {
            Some(CounterExample::Single(counterexample)) => counterexample.calldata,
            _ => Bytes::new(),
        };
        // calldata should be the same with the initial one
        assert_eq!(initial_calldata, new_calldata, "run {i}");
    }

    // write new failure in different file
    runner.test_options.fuzz.failure_persist_file = Some("failure1".to_string());
    let new_calldata = match get_failure_result!() {
        Some(CounterExample::Single(counterexample)) => counterexample.calldata,
        _ => Bytes::new(),
    };
    // empty file is used to load failure so new calldata is generated
    assert_ne!(initial_calldata, new_calldata);
}

forgetest_init!(test_can_scrape_bytecode, |prj, cmd| {
    prj.add_source(
        "FuzzerDict.sol",
        r#"
// https://github.com/foundry-rs/foundry/issues/1168
contract FuzzerDict {
    // Immutables should get added to the dictionary.
    address public immutable immutableOwner;
    // Regular storage variables should also get added to the dictionary.
    address public storageOwner;

    constructor(address _immutableOwner, address _storageOwner) {
        immutableOwner = _immutableOwner;
        storageOwner = _storageOwner;
    }
}
   "#,
    )
    .unwrap();

    prj.add_test(
        "FuzzerDictTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import "src/FuzzerDict.sol";

contract FuzzerDictTest is Test {
    FuzzerDict fuzzerDict;

    function setUp() public {
        fuzzerDict = new FuzzerDict(address(100), address(200));
    }

    /// forge-config: default.fuzz.runs = 2000
    function testImmutableOwner(address who) public {
        assertTrue(who != fuzzerDict.immutableOwner());
    }

    /// forge-config: default.fuzz.runs = 2000
    function testStorageOwner(address who) public {
        assertTrue(who != fuzzerDict.storageOwner());
    }
}
   "#,
    )
    .unwrap();

    // Test that immutable address is used as fuzzed input, causing test to fail.
    cmd.args(["test", "--fuzz-seed", "100", "--mt", "testImmutableOwner"]).assert_failure();
    // Test that storage address is used as fuzzed input, causing test to fail.
    cmd.forge_fuse()
        .args(["test", "--fuzz-seed", "100", "--mt", "testStorageOwner"])
        .assert_failure();
});

// tests that inline max-test-rejects config is properly applied
forgetest_init!(test_inline_max_test_rejects, |prj, cmd| {
    prj.wipe_contracts();

    prj.add_test(
        "Contract.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract InlineMaxRejectsTest is Test {
    /// forge-config: default.fuzz.max-test-rejects = 1
    function test_fuzz_bound(uint256 a) public {
        vm.assume(a == 0);
    }
}
   "#,
    )
    .unwrap();

    cmd.args(["test"]).assert_failure().stdout_eq(str![[r#"
...
[FAIL: `vm.assume` rejected too many inputs (1 allowed)] test_fuzz_bound(uint256) (runs: 0, [AVG_GAS])
...
"#]]);
});
