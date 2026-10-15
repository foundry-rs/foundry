//! Setup discovery and diagnostics shared by test runners.

use super::*;

#[forgetest]
fn setup_discovery_ignores_parameterized_functions(prj: _, cmd: _) {
    prj.add_test(
        "SetupDiscovery.t.sol",
        r#"
contract CanonicalSetupTest {
    uint256 calls;

    function setUp() public { calls++; }
    function testSetupCalledOnce() public view { require(calls == 1, "setup call count"); }
}

contract NoSetupTest {
    function testWithoutSetup() public pure {}
}

contract OverloadedSetupTest {
    uint256 calls;

    function setUp() public { calls++; }
    function setUp(uint256) public pure { revert("parameterized setup called"); }
    function setup(uint256) public pure { revert("parameterized setup called"); }
    function testSetupCalledOnce() public view { require(calls == 1, "setup call count"); }
}

contract ParameterizedSetupTest {
    function setUp(uint256) public pure { revert("parameterized setup called"); }
    function setup(uint256) public pure { revert("parameterized setup called"); }
    function testWithoutSetup() public pure {}
}
"#,
    );

    for network in [
        "ethereum",
        #[cfg(feature = "optimism")]
        "optimism",
    ] {
        cmd.forge_fuse()
            .args(["test", "--network", network])
            .assert_success()
            .stderr_eq("")
            .stdout_eq(
                str![[r#"
...
Ran 1 test for test/SetupDiscovery.t.sol:CanonicalSetupTest
[PASS] testSetupCalledOnce() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/SetupDiscovery.t.sol:NoSetupTest
[PASS] testWithoutSetup() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/SetupDiscovery.t.sol:OverloadedSetupTest
[PASS] testSetupCalledOnce() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/SetupDiscovery.t.sol:ParameterizedSetupTest
[PASS] testWithoutSetup() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 4 test suites [ELAPSED]: 4 tests passed, 0 failed, 0 skipped (4 total tests)

"#]]
                .unordered(),
            );
    }
}

#[forgetest]
fn miscased_setup_warns_without_running_and_survives_constructor_failure(prj: _, cmd: _) {
    prj.add_test(
        "MiscasedSetup.t.sol",
        r#"
contract ConstructorFailureTest {
    constructor() { revert("constructor failed"); }
    function setup() public pure { revert("miscased setup called"); }
    function testBody() public pure { revert("test body called"); }
}

contract MiscasedSetupTest {
    function setup() public pure { revert("miscased setup called"); }
    function testBody() public pure {}
}
"#,
    );

    cmd.arg("test")
        .assert_failure()
        .stderr_eq(str![[r#"
Warning: Found invalid setup function "setup()" did you mean "setUp()"?
Warning: Found invalid setup function "setup()" did you mean "setUp()"?

"#]])
        .stdout_eq(str![[r#"
...
Ran 1 test for test/MiscasedSetup.t.sol:ConstructorFailureTest
[FAIL: constructor failed] constructor() ([GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/MiscasedSetup.t.sol:MiscasedSetupTest
[PASS] testBody() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 2 test suites [ELAPSED]: 1 tests passed, 1 failed, 0 skipped (2 total tests)

Failing tests:
Encountered 1 failing test in test/MiscasedSetup.t.sol:ConstructorFailureTest
[FAIL: constructor failed] constructor() ([GAS])

Encountered a total of 1 failing tests, 1 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test
Tip: Run `forge test --debug --match-test <TEST_NAME>` to inspect one failing test in the debugger

"#]]);

    cmd.forge_fuse()
        .args(["test", "--json"])
        .assert_json_stdout_with_status(
            false,
            str![[r#"
{
  "test/MiscasedSetup.t.sol:ConstructorFailureTest": {
    "duration": "{...}",
    "test_results": {
      "constructor()": {
        "status": "Failure",
        "reason": "constructor failed",
        "counterexample": null,
        "logs": [],
        "decoded_logs": [],
        "kind": { "Unit": { "gas": "{...}" } },
        "traces": [],
        "labeled_addresses": {},
        "duration": "{...}",
        "breakpoints": {},
        "gas_snapshots": {}
      }
    },
    "warnings": ["Found invalid setup function \"setup()\" did you mean \"setUp()\"?"]
  },
  "test/MiscasedSetup.t.sol:MiscasedSetupTest": {
    "duration": "{...}",
    "test_results": {
      "testBody()": {
        "status": "Success",
        "reason": null,
        "counterexample": null,
        "logs": [],
        "decoded_logs": [],
        "kind": { "Unit": { "gas": "{...}" } },
        "traces": [],
        "labeled_addresses": {},
        "duration": "{...}",
        "breakpoints": {},
        "gas_snapshots": {}
      }
    },
    "warnings": ["Found invalid setup function \"setup()\" did you mean \"setUp()\"?"]
  }
}
"#]],
        )
        .stderr_eq("");
}

#[forgetest]
fn duplicate_setups_fail_before_deployment(prj: _, cmd: _) {
    prj.add_test(
        "DuplicateSetup.t.sol",
        r#"
contract DuplicateSetupTest {
    constructor() { revert("constructor must not run"); }
    function setUp() public pure { revert("setup must not run"); }
    function setup() public pure { revert("setup must not run"); }
    function testBody() public pure { revert("test body must not run"); }
}
"#,
    );

    cmd.arg("test")
        .assert_failure()
        .stderr_eq(str![[r#"
Warning: Found invalid setup function "setup()" did you mean "setUp()"?

"#]])
        .stdout_eq(str![[r#"
...
Ran 1 test for test/DuplicateSetup.t.sol:DuplicateSetupTest
[FAIL: multiple setUp functions] setUp() ([GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 1 failed, 0 skipped (1 total tests)

Failing tests:
Encountered 1 failing test in test/DuplicateSetup.t.sol:DuplicateSetupTest
[FAIL: multiple setUp functions] setUp() ([GAS])

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test
Tip: Run `forge test --debug --match-test <TEST_NAME>` to inspect one failing test in the debugger

"#]]);

    cmd.forge_fuse()
        .args(["test", "--json"])
        .assert_json_stdout_with_status(
            false,
            str![[r#"
{
  "test/DuplicateSetup.t.sol:DuplicateSetupTest": {
    "duration": "{...}",
    "test_results": {
      "setUp()": {
        "status": "Failure",
        "reason": "multiple setUp functions",
        "counterexample": null,
        "logs": [],
        "decoded_logs": [],
        "kind": { "Unit": { "gas": 0 } },
        "traces": [],
        "labeled_addresses": {},
        "duration": "{...}",
        "breakpoints": {},
        "gas_snapshots": {}
      }
    },
    "warnings": ["Found invalid setup function \"setup()\" did you mean \"setUp()\"?"]
  }
}
"#]],
        )
        .stderr_eq("");
}

#[forgetest]
fn filtered_suites_do_not_validate_setup(prj: _, cmd: _) {
    prj.add_test(
        "FilteredSetup.t.sol",
        r#"
contract ExcludedSetupTest {
    constructor() { revert("excluded constructor called"); }
    function setUp() public pure { revert("excluded setup called"); }
    function setup() public pure { revert("excluded setup called"); }
    function testExcluded() public pure { revert("excluded test called"); }
}

contract SelectedSetupTest {
    function testSelected() public pure {}
}
"#,
    );

    cmd.args(["test", "--match-test", r"^testSelected\(\)$"])
        .assert_success()
        .stderr_eq("")
        .stdout_eq(str![[r#"
...
Ran 1 test for test/FilteredSetup.t.sol:SelectedSetupTest
[PASS] testSelected() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);

    cmd.forge_fuse()
        .args(["test", "--match-test", r"^testSelected\(\)$", "--json"])
        .assert_json_stdout_with_status(
            true,
            str![[r#"
{
  "test/FilteredSetup.t.sol:SelectedSetupTest": {
    "duration": "{...}",
    "test_results": {
      "testSelected()": {
        "status": "Success",
        "reason": null,
        "counterexample": null,
        "logs": [],
        "decoded_logs": [],
        "kind": { "Unit": { "gas": "{...}" } },
        "traces": [],
        "labeled_addresses": {},
        "duration": "{...}",
        "breakpoints": {},
        "gas_snapshots": {}
      }
    },
    "warnings": []
  }
}
"#]],
        )
        .stderr_eq("");
}
