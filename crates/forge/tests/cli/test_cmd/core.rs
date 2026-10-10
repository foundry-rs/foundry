//! Core test functionality tests

use foundry_compilers::artifacts::output_selection::ContractOutputSelection;
use foundry_test_utils::str;
use serde_json::Value;

#[forgetest_init]
fn failing_test_after_failed_setup(prj: _, cmd: _) {
    prj.add_test(
        "FailingTestAfterFailedSetup.t.sol",
        r#"
import "forge-std/Test.sol";

contract FailingTestAfterFailedSetupTest is Test {
    function setUp() public {
        assertTrue(false);
    }

    function testAssertSuccess() public {
        assertTrue(true);
    }

    function testAssertFailure() public {
        assertTrue(false);
    }
}
"#,
    );

    cmd.arg("test").assert_failure().stdout_eq(str![[r#"
...
Ran 1 test for test/FailingTestAfterFailedSetup.t.sol:FailingTestAfterFailedSetupTest
[FAIL: assertion failed] setUp() ([GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 1 failed, 0 skipped (1 total tests)

Failing tests:
Encountered 1 failing test in test/FailingTestAfterFailedSetup.t.sol:FailingTestAfterFailedSetupTest
[FAIL: assertion failed] setUp() ([GAS])

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test
Tip: Run `forge test --debug --match-test <TEST_NAME>` to inspect one failing test in the debugger

"#]]);
}

#[forgetest_init]
fn legacy_assertions(prj: _, cmd: _) {
    prj.add_test(
        "LegacyAssertions.t.sol",
        r#"
import "forge-std/Test.sol";

contract NoAssertionsRevertTest is Test {
    function testMultipleAssertFailures() public {
        vm.assertEq(uint256(1), uint256(2));
        vm.assertLt(uint256(5), uint256(4));
    }
}

/// forge-config: default.legacy_assertions = true
contract LegacyAssertionsTest {
    bool public failed;

    function setFailed() external {
        failed = true;
    }

    function testFlagNotSetSuccess() public {}

    function testFlagSetFailure() public {
        failed = true;
    }

    function testFlagSetInCallFailure() public {
        this.setFailed();
    }
}

/// forge-config: default.legacy_assertions = true
/// forge-config: default.isolate = false
contract LegacyAssertionsNonIsolatedTest is LegacyAssertionsTest {}

// Non-view on purpose: calls into it must be CALLs, which run isolated.
contract Asserter {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));

    function fail() external {
        vm.assertTrue(false);
    }

    function failThenRevert() external {
        vm.assertTrue(false);
        revert();
    }
}

/// forge-config: default.assertions_revert = false
contract NonRevertingAssertionsTest is Test {
    Asserter asserter = new Asserter();

    function testBodyFailure() public {
        vm.assertTrue(false);
    }

    function testCallFailure() public {
        asserter.fail();
    }

    function testRevertedCallFailureIsDropped() public {
        try asserter.failThenRevert() {} catch {}
    }
}

/// forge-config: default.assertions_revert = false
/// forge-config: default.isolate = false
contract NonRevertingAssertionsNonIsolatedTest is NonRevertingAssertionsTest {}
"#,
    );

    cmd.args(["test", "-j1"]).assert_failure().stdout_eq(str![[r#"
...
Ran 3 tests for test/LegacyAssertions.t.sol:LegacyAssertionsNonIsolatedTest
[PASS] testFlagNotSetSuccess() ([GAS])
[FAIL] testFlagSetFailure() ([GAS])
[FAIL] testFlagSetInCallFailure() ([GAS])
Suite result: FAILED. 1 passed; 2 failed; 0 skipped; [ELAPSED]

Ran 3 tests for test/LegacyAssertions.t.sol:LegacyAssertionsTest
[PASS] testFlagNotSetSuccess() ([GAS])
[FAIL] testFlagSetFailure() ([GAS])
[FAIL] testFlagSetInCallFailure() ([GAS])
Suite result: FAILED. 1 passed; 2 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/LegacyAssertions.t.sol:NoAssertionsRevertTest
[FAIL: assertion failed: 1 != 2] testMultipleAssertFailures() ([GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 3 tests for test/LegacyAssertions.t.sol:NonRevertingAssertionsNonIsolatedTest
[FAIL] testBodyFailure() ([GAS])
[FAIL] testCallFailure() ([GAS])
[PASS] testRevertedCallFailureIsDropped() ([GAS])
Suite result: FAILED. 1 passed; 2 failed; 0 skipped; [ELAPSED]

Ran 3 tests for test/LegacyAssertions.t.sol:NonRevertingAssertionsTest
[FAIL] testBodyFailure() ([GAS])
[FAIL] testCallFailure() ([GAS])
[PASS] testRevertedCallFailureIsDropped() ([GAS])
Suite result: FAILED. 1 passed; 2 failed; 0 skipped; [ELAPSED]

Ran 5 test suites [ELAPSED]: 4 tests passed, 9 failed, 0 skipped (13 total tests)

Failing tests:
Encountered 2 failing tests in test/LegacyAssertions.t.sol:LegacyAssertionsNonIsolatedTest
[FAIL] testFlagSetFailure() ([GAS])
[FAIL] testFlagSetInCallFailure() ([GAS])

Encountered 2 failing tests in test/LegacyAssertions.t.sol:LegacyAssertionsTest
[FAIL] testFlagSetFailure() ([GAS])
[FAIL] testFlagSetInCallFailure() ([GAS])

Encountered 1 failing test in test/LegacyAssertions.t.sol:NoAssertionsRevertTest
[FAIL: assertion failed: 1 != 2] testMultipleAssertFailures() ([GAS])

Encountered 2 failing tests in test/LegacyAssertions.t.sol:NonRevertingAssertionsNonIsolatedTest
[FAIL] testBodyFailure() ([GAS])
[FAIL] testCallFailure() ([GAS])

Encountered 2 failing tests in test/LegacyAssertions.t.sol:NonRevertingAssertionsTest
[FAIL] testBodyFailure() ([GAS])
[FAIL] testCallFailure() ([GAS])

Encountered a total of 9 failing tests, 4 tests succeeded

Tip: Run `forge test --rerun` to retry only the 9 failed tests
Tip: Run `forge test --debug --match-test <TEST_NAME>` to inspect one failing test in the debugger

"#]]);
}

#[forgetest_init]
fn evm_profile_no_open_writes_profile_and_exits(prj: _, cmd: _) {
    prj.add_test(
        "EvmProfileNoOpen.t.sol",
        r#"
contract EvmProfileNoOpenTest {
    function testProfile() public {}
}
"#,
    );

    cmd.args(["test", "--match-test", "testProfile", "--evm-profile", "--no-open"])
        .assert_success()
        .stdout_eq(str![[r#"
...
Profile saved to cache/evm_profile_EvmProfileNoOpenTest_testProfile.json

"#]]);

    let profile_path = prj.root().join("cache/evm_profile_EvmProfileNoOpenTest_testProfile.json");
    let profile: Value = serde_json::from_str(&std::fs::read_to_string(profile_path).unwrap())
        .expect("profile should be valid JSON");
    assert_eq!(profile["exporter"], "foundry");
    assert_eq!(profile["profiles"][0]["type"], "evented");
}

#[forgetest_init]
fn evm_profile_conflicts_with_early_return_outputs(cmd: _) {
    cmd.args(["test", "--evm-profile", "--json"]).assert_failure().stderr_eq(str![[r#"
error: the argument '--evm-profile [<FORMAT>]' cannot be used with '--json'

Usage: forge[..] test --evm-profile [<FORMAT>] [PATH]

For more information, try '--help'.

"#]]);

    cmd.forge_fuse().args(["test", "--evm-profile", "--junit"]).assert_failure().stderr_eq(str![[
        r#"
error: the argument '--evm-profile [<FORMAT>]' cannot be used with '--junit'

Usage: forge[..] test --evm-profile [<FORMAT>] [PATH]

For more information, try '--help'.

"#
    ]]);

    cmd.forge_fuse().args(["test", "--evm-profile", "--list"]).assert_failure().stderr_eq(str![[
        r#"
error: the argument '--evm-profile [<FORMAT>]' cannot be used with '--list'

Usage: forge[..] test --evm-profile [<FORMAT>] [PATH]

For more information, try '--help'.

"#
    ]]);
}

#[forgetest_init]
fn flame_outputs_conflict_with_early_return_outputs(cmd: _) {
    cmd.args(["test", "--flamegraph", "--json"]).assert_failure().stderr_eq(str![[r#"
error: the argument '--flamegraph' cannot be used with '--json'

Usage: forge[..] test --flamegraph [PATH]

For more information, try '--help'.

"#]]);

    cmd.forge_fuse().args(["test", "--flamechart", "--list"]).assert_failure().stderr_eq(str![[
        r#"
error: the argument '--flamechart' cannot be used with '--list'

Usage: forge[..] test --flamechart [PATH]

For more information, try '--help'.

"#
    ]]);
}

#[forgetest_init]
fn test_list_outputs_matching_tests(prj: _, cmd: _) {
    prj.add_test(
        "ListTests.t.sol",
        r#"
contract ListTests {
    function test_alpha() public pure {}
    function test_beta() public pure {}
    function testFuzz_value(uint256 value) public pure {
        value;
    }
}
"#,
    );
    prj.add_test(
        "ConstructorArgListTests.t.sol",
        r#"
contract ConstructorArgListTests {
    constructor(uint256 value) {
        value;
    }

    function test_constructor_arg() public pure {}
}
"#,
    );

    cmd.args(["test", "--list", "--match-test", "test_"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!
test/ListTests.t.sol
  ListTests
    test_alpha
    test_beta


"#]]);

    // Path-filtered listing should not compile unrelated sources.
    prj.add_test("Broken.t.sol", "contract");
    cmd.forge_fuse()
        .args(["test", "--list", "--match-test", "test_alpha", "--json"])
        .arg("test/ListTests.t.sol")
        .assert_success()
        .stdout_eq("{\"test/ListTests.t.sol\":{\"ListTests\":[\"test_alpha\"]}}\n");
}

// Listing tests must not write ABI-only artifacts that later cached builds treat as fresh.
#[forgetest]
fn test_list_does_not_poison_build_cache(prj: _, cmd: _) {
    let artifact = prj.root().join("out/ListCache.t.sol/ListCacheTest.json");
    let cache = prj.root().join("cache/solidity-files-cache.json");
    // Extra output files bypass the ABI cache and exercise the uncached fallback.
    for extra_output_files in [vec![], vec![ContractOutputSelection::Metadata]] {
        prj.update_config(|config| config.extra_output_files = extra_output_files.clone());
        prj.add_test(
            "ListCache.t.sol",
            "contract ListCacheTest { function test_value() public pure { require(1 == 1); } }",
        );
        cmd.forge_fuse().arg("build").assert_success();
        let artifact_before = std::fs::read_to_string(&artifact).unwrap();
        let cache_before = std::fs::read_to_string(&cache).unwrap();

        prj.add_test(
            "ListCache.t.sol",
            "contract ListCacheTest { function test_value() public pure { require(1 == 2); } }",
        );
        cmd.forge_fuse().args(["test", "--list"]).assert_success();
        assert_eq!(std::fs::read_to_string(&artifact).unwrap(), artifact_before);
        assert_eq!(std::fs::read_to_string(&cache).unwrap(), cache_before);
        cmd.forge_fuse().arg("test").assert_failure().stdout_eq(str![[r#"
...
Ran 1 test for test/ListCache.t.sol:ListCacheTest
[FAIL: EvmError: Revert] test_value() ([GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 1 failed, 0 skipped (1 total tests)
...
"#]]);
    }
}

#[forgetest_init]
fn evm_profile_requires_execution_trace(prj: _, cmd: _) {
    prj.add_test(
        "EvmProfileNoExecutionTrace.t.sol",
        r#"
contract EvmProfileNoExecutionTraceTest {
    function setUp() public {
        revert("setUp failed");
    }

    function testProfile() public {}
}
"#,
    );

    cmd.args(["test", "--match-test", "testProfile", "--evm-profile", "--no-open"])
        .assert_failure()
        .stderr_eq(str![[r#"
Error: cannot generate EVM profile for EvmProfileNoExecutionTraceTest::setUp: no execution trace (test may have failed in setUp/constructor or been skipped)

"#]]);
}

#[forgetest_init]
fn evm_profile_errors_when_no_tests_match(prj: _, cmd: _) {
    prj.add_test(
        "EvmProfileNoMatch.t.sol",
        r#"
contract EvmProfileNoMatchTest {
    function testProfile() public {}
}
"#,
    );

    cmd.args(["test", "--match-test", "missing", "--evm-profile", "--no-open"])
        .assert_failure()
        .stderr_eq(str![[r#"
...
Error: cannot generate EVM profile: no tests were executed

"#]]);
}

#[forgetest_init]
fn flamegraph_requires_execution_trace(prj: _, cmd: _) {
    prj.add_test(
        "FlamegraphNoExecutionTrace.t.sol",
        r#"
contract FlamegraphNoExecutionTraceTest {
    function setUp() public {
        revert("setUp failed");
    }

    function testProfile() public {}
}
"#,
    );

    cmd.args(["test", "--match-test", "testProfile", "--flamegraph"])
        .assert_failure()
        .stderr_eq(str![[r#"
Error: cannot generate flamegraph for FlamegraphNoExecutionTraceTest::setUp: no execution trace (test may have failed in setUp/constructor or been skipped)

"#]]);
}

#[forgetest_init]
fn flame_outputs_profile_test_after_before_test_setup(prj: _, cmd: _) {
    prj.add_test(
        "FlameBeforeTestSetup.t.sol",
        r#"
contract FlameBeforeTestSetupTest {
    function beforeTestSetup(bytes4 testSelector)
        public
        view
        returns (bytes[] memory beforeTestCalldata)
    {
        if (testSelector == this.testProfile.selector) {
            beforeTestCalldata = new bytes[](1);
            beforeTestCalldata[0] = abi.encodeCall(this.beforeOnly, ());
        }
    }

    function beforeOnly() public {}

    function testProfile() public {}
}
"#,
    );

    cmd.args(["test", "--match-test", "testProfile", "--flamegraph", "--no-open"]).assert_success();
    let flamegraph = std::fs::read_to_string(
        prj.root().join("cache/flamegraph_FlameBeforeTestSetupTest_testProfile.svg"),
    )
    .unwrap();
    assert!(flamegraph.contains("FlameBeforeTestSetupTest.testProfile()"));
    assert!(!flamegraph.contains("FlameBeforeTestSetupTest.beforeOnly()"));

    cmd.forge_fuse()
        .args(["test", "--match-test", "testProfile", "--flamechart", "--no-open"])
        .assert_success();
    let flamechart = std::fs::read_to_string(
        prj.root().join("cache/flamechart_FlameBeforeTestSetupTest_testProfile.svg"),
    )
    .unwrap();
    assert!(flamechart.contains("FlameBeforeTestSetupTest.testProfile()"));
    assert!(!flamechart.contains("FlameBeforeTestSetupTest.beforeOnly()"));
}

#[forgetest_init]
fn payment_failure(prj: _, cmd: _) {
    prj.add_test(
        "PaymentFailure.t.sol",
        r#"
import "forge-std/Test.sol";

contract Payable {
    function pay() public payable {}
}

contract PaymentFailureTest is Test {
    function testCantPay() public {
        Payable target = new Payable();
        vm.prank(address(1));
        target.pay{value: 1}();
    }
}
"#,
    );

    cmd.arg("test").assert_failure().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/PaymentFailure.t.sol:PaymentFailureTest
[FAIL: EvmError: Revert] testCantPay() ([GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 1 failed, 0 skipped (1 total tests)

Failing tests:
Encountered 1 failing test in test/PaymentFailure.t.sol:PaymentFailureTest
[FAIL: EvmError: Revert] testCantPay() ([GAS])

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test
Tip: Run `forge test --debug --match-test <TEST_NAME>` to inspect one failing test in the debugger

"#]]);
}

#[forgetest_init]
fn rerun_filters_same_named_tests_by_contract(prj: _, cmd: _) {
    prj.add_test(
        "RerunSameName.t.sol",
        r#"
import "forge-std/Test.sol";

contract FailingSameNameTest is Test {
    function testSharedName() public {
        assertTrue(false);
    }
}

contract PassingSameNameTest is Test {
    function testSharedName() public {
        assertTrue(true);
    }
}
"#,
    );

    cmd.args(["test", "-j1"]).assert_failure().stdout_eq(str![[r#"
...
Ran 1 test for test/RerunSameName.t.sol:FailingSameNameTest
[FAIL: assertion failed] testSharedName() ([GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/RerunSameName.t.sol:PassingSameNameTest
[PASS] testSharedName() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 2 test suites [ELAPSED]: 1 tests passed, 1 failed, 0 skipped (2 total tests)
...
"#]]);

    cmd.forge_fuse().args(["test", "--rerun", "-j1"]).assert_failure().stdout_eq(str![[r#"
No files changed, compilation skipped

Ran 1 test for test/RerunSameName.t.sol:FailingSameNameTest
[FAIL: assertion failed] testSharedName() ([GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 1 failed, 0 skipped (1 total tests)
...
"#]]);
}

#[forgetest_init]
fn rerun_with_only_setup_failure_runs_all_tests(prj: _, cmd: _) {
    prj.add_test(
        "RerunSetupFail.t.sol",
        r#"
import "forge-std/Test.sol";

contract OnlySetupFails is Test {
    function setUp() public {
        assertTrue(false);
    }

    function testA() public {
        assertTrue(true);
    }
}

contract HealthyContract is Test {
    function testC() public {
        assertTrue(true);
    }
}
"#,
    );

    cmd.args(["test", "-j1"]).assert_failure();

    // With no replayable failures recorded, `--rerun` falls back to a regular run instead of
    // selecting zero tests.
    cmd.forge_fuse().args(["test", "--rerun", "-j1"]).assert_failure().stdout_eq(str![[r#"
No files changed, compilation skipped

Ran 1 test for test/RerunSetupFail.t.sol:HealthyContract
[PASS] testC() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/RerunSetupFail.t.sol:OnlySetupFails
[FAIL: assertion failed] setUp() ([GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 2 test suites [ELAPSED]: 1 tests passed, 1 failed, 0 skipped (2 total tests)
...
"#]]);
}

#[forgetest_init]
fn rerun_cache_tracks_completed_invocation(prj: _, cmd: _) {
    let failures_file = prj.root().join("cache/test-failures");
    let recorded_failure = r#"{"version":1,"failures":[{"contract":"test/RerunLifecycle.t.sol:RerunLifecycleTest","test":"testBroken"}]}"#;
    let passing_test = r#"
contract RerunLifecycleTest {
    function testBroken() public {}
    function testOther() public {}
}
"#;

    // A compilation error does not complete a test invocation and must preserve the last record.
    std::fs::create_dir_all(failures_file.parent().unwrap()).unwrap();
    std::fs::write(&failures_file, recorded_failure).unwrap();
    prj.add_test("RerunLifecycle.t.sol", "contract Broken {");
    cmd.args(["test", "-j1"]).assert_failure();
    assert_eq!(std::fs::read_to_string(&failures_file).unwrap(), recorded_failure);

    // Passing runs clear the record in both human-readable and serialized output modes.
    for output_args in
        [vec![], vec!["--rerun"], vec!["--rerun", "--json"], vec!["--rerun", "--junit"]]
    {
        prj.add_test("RerunLifecycle.t.sol", passing_test);
        std::fs::write(&failures_file, recorded_failure).unwrap();
        cmd.forge_fuse().args(["test", "-j1"]).args(output_args).assert_success();
        assert!(!failures_file.exists());
    }

    // With no recorded failures, rerun falls back to the full test suite.
    cmd.forge_fuse().args(["test", "--rerun", "-j1"]).assert_success().stdout_eq(str![[r#"
...
Ran 2 tests for test/RerunLifecycle.t.sol:RerunLifecycleTest
[PASS] testBroken() ([GAS])
[PASS] testOther() ([GAS])
Suite result: ok. 2 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 2 tests passed, 0 failed, 0 skipped (2 total tests)

"#]]);

    // A stale recorded identity that no longer matches a renamed test clears the cache.
    std::fs::write(&failures_file, recorded_failure).unwrap();
    prj.add_test(
        "RerunLifecycle.t.sol",
        r#"
contract RerunLifecycleTest {
    function testRenamed() public {}
}
"#,
    );
    cmd.forge_fuse().args(["test", "--rerun", "-j1"]).assert_success();
    assert!(!failures_file.exists());
}

#[forgetest_init]
fn rerun_cache_merges_network_pass_failures(prj: _, cmd: _) {
    prj.add_test(
        "RerunNetworks.t.sol",
        r#"
contract RerunNetworksTest {
    function testDefaultFailure() public pure {
        require(false, "default failure");
    }

    /// forge-config: default.networks.network = "tempo"
    function testTempoSuccess() public pure {}
}
"#,
    );

    cmd.args(["test", "-j1"]).assert_failure();
    let failures_file = prj.root().join("cache/test-failures");
    let failures: Value =
        serde_json::from_str(&std::fs::read_to_string(&failures_file).unwrap()).unwrap();
    assert_eq!(failures["failures"].as_array().unwrap().len(), 1);
    assert_eq!(failures["failures"][0]["test"], "testDefaultFailure");

    prj.add_test(
        "RerunNetworks.t.sol",
        r#"
contract RerunNetworksTest {
    function testDefaultFailure() public pure {
        require(false, "default failure");
    }

    /// forge-config: default.networks.network = "tempo"
    function testTempoFailure() public pure {
        require(false, "tempo failure");
    }
}
"#,
    );
    cmd.forge_fuse().args(["test", "-j1"]).assert_failure();
    let failures: Value =
        serde_json::from_str(&std::fs::read_to_string(&failures_file).unwrap()).unwrap();
    let failed_tests = failures["failures"]
        .as_array()
        .unwrap()
        .iter()
        .map(|failure| failure["test"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(failed_tests.len(), 2);
    assert!(failed_tests.contains(&"testDefaultFailure"));
    assert!(failed_tests.contains(&"testTempoFailure"));
}
