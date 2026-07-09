//! Core test functionality tests

use foundry_test_utils::str;
use serde_json::Value;

forgetest_init!(failing_test_after_failed_setup, |prj, cmd| {
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
});

forgetest_init!(legacy_assertions, |prj, cmd| {
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

    function testFlagNotSetSuccess() public {}

    function testFlagSetFailure() public {
        failed = true;
    }
}
"#,
    );

    cmd.args(["test", "-j1"]).assert_failure().stdout_eq(str![[r#"
...
Ran 2 tests for test/LegacyAssertions.t.sol:LegacyAssertionsTest
[PASS] testFlagNotSetSuccess() ([GAS])
[FAIL] testFlagSetFailure() ([GAS])
Suite result: FAILED. 1 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/LegacyAssertions.t.sol:NoAssertionsRevertTest
[FAIL: assertion failed: 1 != 2] testMultipleAssertFailures() ([GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 2 test suites [ELAPSED]: 1 tests passed, 2 failed, 0 skipped (3 total tests)

Failing tests:
Encountered 1 failing test in test/LegacyAssertions.t.sol:LegacyAssertionsTest
[FAIL] testFlagSetFailure() ([GAS])

Encountered 1 failing test in test/LegacyAssertions.t.sol:NoAssertionsRevertTest
[FAIL: assertion failed: 1 != 2] testMultipleAssertFailures() ([GAS])

Encountered a total of 2 failing tests, 1 tests succeeded

Tip: Run `forge test --rerun` to retry only the 2 failed tests
Tip: Run `forge test --debug --match-test <TEST_NAME>` to inspect one failing test in the debugger

"#]]);
});

forgetest_init!(evm_profile_no_open_writes_profile_and_exits, |prj, cmd| {
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
});

forgetest_init!(evm_profile_conflicts_with_early_return_outputs, |_prj, cmd| {
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
});

forgetest_init!(flame_outputs_conflict_with_early_return_outputs, |_prj, cmd| {
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
});

forgetest_init!(test_list_outputs_matching_tests, |prj, cmd| {
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

    cmd.forge_fuse()
        .args(["test", "--list", "--match-test", "test_alpha", "--json"])
        .assert_success()
        .stdout_eq("{\"test/ListTests.t.sol\":{\"ListTests\":[\"test_alpha\"]}}\n");
});

forgetest_init!(evm_profile_requires_execution_trace, |prj, cmd| {
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
});

forgetest_init!(evm_profile_errors_when_no_tests_match, |prj, cmd| {
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
});

forgetest_init!(flamegraph_requires_execution_trace, |prj, cmd| {
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
});

forgetest_init!(payment_failure, |prj, cmd| {
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
});

forgetest_init!(rerun_filters_same_named_tests_by_contract, |prj, cmd| {
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
});

forgetest_init!(rerun_with_only_setup_failure_runs_all_tests, |prj, cmd| {
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
});
