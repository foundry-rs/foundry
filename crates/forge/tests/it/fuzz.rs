//! Fuzz tests.

use crate::{config::*, test_helpers::TEST_DATA_DEFAULT};
use alloy_primitives::{Bytes, U256};
use forge::{
    decode::decode_console_logs,
    fuzz::CounterExample,
    result::{SuiteResult, TestStatus},
};
use foundry_test_utils::{Filter, forgetest_init, str};
use std::collections::BTreeMap;

#[tokio::test(flavor = "multi_thread")]
async fn test_fuzz() {
    let filter = Filter::new(".*", ".*", ".*fuzz/")
        .exclude_tests(r"invariantCounter|testIncrement\(address\)|testNeedle\(uint256\)|testSuccessChecker\(uint256\)|testSuccessChecker2\(int256\)|testSuccessChecker3\(uint32\)|testStorageOwner\(address\)|testImmutableOwner\(address\)")
        .exclude_paths("invariant");
    let mut runner = TEST_DATA_DEFAULT.runner();
    let suite_result = runner.test_collect(&filter).unwrap();

    assert!(!suite_result.is_empty());

    for (_, SuiteResult { test_results, .. }) in suite_result {
        for (test_name, result) in test_results {
            match test_name.as_str() {
                "testPositive(uint256)"
                | "testPositive(int256)"
                | "testSuccessfulFuzz(uint128,uint128)"
                | "testToStringFuzz(bytes32)" => assert_eq!(
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
    let suite_result = runner.test_collect(&filter).unwrap();

    assert!(!suite_result.is_empty());

    for (_, SuiteResult { test_results, .. }) in suite_result {
        for (test_name, result) in test_results {
            match test_name.as_str() {
                "testSuccessChecker(uint256)"
                | "testSuccessChecker2(int256)"
                | "testSuccessChecker3(uint32)" => assert_eq!(
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
    let mut runner = TEST_DATA_DEFAULT.runner_with(|config| {
        config.invariant.depth = 100;
        config.invariant.runs = 1000;
        config.fuzz.runs = 1000;
        config.fuzz.seed = Some(U256::from(6u32));
    });
    let results = runner.test_collect(&filter).unwrap();

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

    macro_rules! run_fail {
        () => { run_fail!(|config| {}) };
        (|$config:ident| $e:expr) => {{
            let mut runner = TEST_DATA_DEFAULT.runner_with(|$config| {
                $config.fuzz.runs = 1000;
                $e
            });
            runner
                .test_collect(&filter)
                .unwrap()
                .get("default/fuzz/FuzzFailurePersist.t.sol:FuzzFailurePersistTest")
                .unwrap()
                .test_results
                .get("test_persist_fuzzed_failure(uint256,int256,address,bool,string,(address,uint256),address[])")
                .unwrap()
                .counterexample
                .clone()
        }};
    }

    // record initial counterexample calldata
    let initial_counterexample = run_fail!();
    let initial_calldata = match initial_counterexample {
        Some(CounterExample::Single(counterexample)) => counterexample.calldata,
        _ => Bytes::new(),
    };

    // run several times and compare counterexamples calldata
    for i in 0..10 {
        let new_calldata = match run_fail!() {
            Some(CounterExample::Single(counterexample)) => counterexample.calldata,
            _ => Bytes::new(),
        };
        // calldata should be the same with the initial one
        assert_eq!(initial_calldata, new_calldata, "run {i}");
    }

    // write new failure in different dir.
    let persist_dir = tempfile::tempdir().unwrap().keep();
    let new_calldata = match run_fail!(|config| config.fuzz.failure_persist_dir = Some(persist_dir))
    {
        Some(CounterExample::Single(counterexample)) => counterexample.calldata,
        _ => Bytes::new(),
    };
    // empty file is used to load failure so new calldata is generated
    assert_ne!(initial_calldata, new_calldata);
}

forgetest_init!(test_can_scrape_bytecode, |prj, cmd| {
    prj.update_config(|config| config.optimizer = Some(true));
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
    );

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
    );

    // Test that immutable address is used as fuzzed input, causing test to fail.
    cmd.args(["test", "--fuzz-seed", "119", "--mt", "testImmutableOwner"]).assert_failure();
    // Test that storage address is used as fuzzed input, causing test to fail.
    cmd.forge_fuse()
        .args(["test", "--fuzz-seed", "119", "--mt", "testStorageOwner"])
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
        vm.assume(false);
    }
}
   "#,
    );

    cmd.args(["test"]).assert_failure().stdout_eq(str![[r#"
...
[FAIL: `vm.assume` rejected too many inputs (1 allowed)] test_fuzz_bound(uint256) (runs: 0, [AVG_GAS])
...
"#]]);
});

// Tests that test timeout config is properly applied.
// If test doesn't timeout after one second, then test will fail with `rejected too many inputs`.
forgetest_init!(test_fuzz_timeout, |prj, cmd| {
    prj.wipe_contracts();

    prj.add_test(
        "Contract.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract FuzzTimeoutTest is Test {
    /// forge-config: default.fuzz.max-test-rejects = 50000
    /// forge-config: default.fuzz.timeout = 1
    function test_fuzz_bound(uint256 a) public pure {
        vm.assume(a == 0);
    }
}
   "#,
    );

    cmd.args(["test"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/Contract.t.sol:FuzzTimeoutTest
[PASS] test_fuzz_bound(uint256) (runs: [..], [AVG_GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
});

forgetest_init!(test_fuzz_fail_on_revert, |prj, cmd| {
    prj.wipe_contracts();
    prj.update_config(|config| config.fuzz.fail_on_revert = false);
    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    uint256 public number;

    function setNumber(uint256 newNumber) public {
        require(number > 10000000000, "low number");
        number = newNumber;
    }
}
   "#,
    );

    prj.add_test(
        "CounterTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import "src/Counter.sol";

contract CounterTest is Test {
    Counter public counter;

    function setUp() public {
        counter = new Counter();
    }

    function testFuzz_SetNumberRequire(uint256 x) public {
        counter.setNumber(x);
        require(counter.number() == 1);
    }

    function testFuzz_SetNumberAssert(uint256 x) public {
        counter.setNumber(x);
        assertEq(counter.number(), 1);
    }
}
   "#,
    );

    // Tests should not fail as revert happens in Counter contract.
    cmd.args(["test", "--mc", "CounterTest"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 2 tests for test/CounterTest.t.sol:CounterTest
[PASS] testFuzz_SetNumberAssert(uint256) (runs: 256, [AVG_GAS])
[PASS] testFuzz_SetNumberRequire(uint256) (runs: 256, [AVG_GAS])
Suite result: ok. 2 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 2 tests passed, 0 failed, 0 skipped (2 total tests)

"#]]);

    // Tested contract does not revert.
    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    uint256 public number;

    function setNumber(uint256 newNumber) public {
        number = newNumber;
    }
}
   "#,
    );

    // Tests should fail as revert happens in cheatcode (assert) and test (require) contract.
    cmd.assert_failure().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 2 tests for test/CounterTest.t.sol:CounterTest
[FAIL: assertion failed: [..]] testFuzz_SetNumberAssert(uint256) (runs: 0, [AVG_GAS])
[FAIL: EvmError: Revert; [..]] testFuzz_SetNumberRequire(uint256) (runs: 0, [AVG_GAS])
Suite result: FAILED. 0 passed; 2 failed; 0 skipped; [ELAPSED]
...

"#]]);
});

// Test 256 runs regardless number of test rejects.
// <https://github.com/foundry-rs/foundry/issues/9054>
forgetest_init!(test_fuzz_runs_with_rejects, |prj, cmd| {
    prj.add_test(
        "FuzzWithRejectsTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract FuzzWithRejectsTest is Test {
    function testFuzzWithRejects(uint256 x) public pure {
        vm.assume(x < 1_000_000);
    }
}
   "#,
    );

    // Tests should not fail as revert happens in Counter contract.
    cmd.args(["test", "--mc", "FuzzWithRejectsTest"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/FuzzWithRejectsTest.t.sol:FuzzWithRejectsTest
[PASS] testFuzzWithRejects(uint256) (runs: 256, [AVG_GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
});
