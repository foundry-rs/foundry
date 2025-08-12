//! Contains various tests for `forge test`.

use alloy_primitives::U256;
use anvil::{NodeConfig, spawn};
use foundry_test_utils::{
    rpc, str,
    util::{OTHER_SOLC_VERSION, OutputExt, SOLC_VERSION},
};
use similar_asserts::assert_eq;
use std::{path::PathBuf, str::FromStr};

// tests that test filters are handled correctly
forgetest!(can_set_filter_values, |prj, cmd| {
    let patt = regex::Regex::new("test*").unwrap();
    let glob = globset::Glob::from_str("foo/bar/baz*").unwrap();

    // explicitly set patterns
    prj.update_config(|config| {
        config.test_pattern = Some(patt.clone().into());
        config.test_pattern_inverse = None;
        config.contract_pattern = Some(patt.clone().into());
        config.contract_pattern_inverse = None;
        config.path_pattern = Some(glob.clone());
        config.path_pattern_inverse = None;
        config.coverage_pattern_inverse = None;
    });

    let config = cmd.config();

    assert_eq!(config.test_pattern.unwrap().as_str(), patt.as_str());
    assert_eq!(config.test_pattern_inverse, None);
    assert_eq!(config.contract_pattern.unwrap().as_str(), patt.as_str());
    assert_eq!(config.contract_pattern_inverse, None);
    assert_eq!(config.path_pattern.unwrap(), glob);
    assert_eq!(config.path_pattern_inverse, None);
    assert_eq!(config.coverage_pattern_inverse, None);
});

// tests that warning is displayed when there are no tests in project
forgetest!(warn_no_tests, |prj, cmd| {
    prj.add_source(
        "dummy",
        r"
contract Dummy {}
",
    )
    .unwrap();
    // set up command
    cmd.args(["test"]);

    // run command and assert
    cmd.assert_failure().stdout_eq(str![[r#"
No tests found in project! Forge looks for functions that starts with `test`.

"#]]);
});

// tests that warning is displayed with pattern when no tests match
forgetest!(warn_no_tests_match, |prj, cmd| {
    prj.add_source(
        "dummy",
        r"
contract Dummy {}
",
    )
    .unwrap();

    // set up command
    cmd.args(["test", "--match-test", "testA.*", "--no-match-test", "testB.*"]);
    cmd.args(["--match-contract", "TestC.*", "--no-match-contract", "TestD.*"]);
    cmd.args(["--match-path", "*TestE*", "--no-match-path", "*TestF*"]);

    // run command and assert
    cmd.assert_failure().stdout_eq(str![[r#"
No tests match the provided pattern:
	match-test: `testA.*`
	no-match-test: `testB.*`
	match-contract: `TestC.*`
	no-match-contract: `TestD.*`
	match-path: `*TestE*`
	no-match-path: `*TestF*`

"#]]);
});

// tests that suggestion is provided with pattern when no tests match
forgetest!(suggest_when_no_tests_match, |prj, cmd| {
    // set up project
    prj.add_source(
        "TestE.t.sol",
        r"
contract TestC {
    function test1() public {
    }
}
   ",
    )
    .unwrap();

    // set up command
    cmd.args(["test", "--match-test", "testA.*", "--no-match-test", "testB.*"]);
    cmd.args(["--match-contract", "TestC.*", "--no-match-contract", "TestD.*"]);
    cmd.args(["--match-path", "*TestE*", "--no-match-path", "*TestF*"]);

    // run command and assert
    cmd.assert_failure().stdout_eq(str![[r#"
No tests match the provided pattern:
	match-test: `testA.*`
	no-match-test: `testB.*`
	match-contract: `TestC.*`
	no-match-contract: `TestD.*`
	match-path: `*TestE*`
	no-match-path: `*TestF*`

Did you mean `test1`?

"#]]);
});

// tests that direct import paths are handled correctly
forgetest!(can_fuzz_array_params, |prj, cmd| {
    prj.insert_ds_test();

    prj.add_source(
        "ATest.t.sol",
        r#"
import "./test.sol";
contract ATest is DSTest {
    function testArray(uint64[2] calldata) external {
        assertTrue(true);
    }
}
   "#,
    )
    .unwrap();

    cmd.arg("test").assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for src/ATest.t.sol:ATest
[PASS] testArray(uint64[2]) (runs: 256, [AVG_GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
});

// tests that `bytecode_hash` will be sanitized
forgetest!(can_test_pre_bytecode_hash, |prj, cmd| {
    prj.insert_ds_test();

    prj.add_source(
        "ATest.t.sol",
        r#"
// pre bytecode hash version, was introduced in 0.6.0
pragma solidity 0.5.17;
import "./test.sol";
contract ATest is DSTest {
    function testArray(uint64[2] calldata) external {
        assertTrue(true);
    }
}
   "#,
    )
    .unwrap();

    cmd.arg("test").assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for src/ATest.t.sol:ATest
[PASS] testArray(uint64[2]) (runs: 256, [AVG_GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
});

// tests that using the --match-path option only runs files matching the path
forgetest!(can_test_with_match_path, |prj, cmd| {
    prj.insert_ds_test();

    prj.add_source(
        "ATest.t.sol",
        r#"
import "./test.sol";
contract ATest is DSTest {
    function testPass() external {
        assertTrue(true);
    }
}
   "#,
    )
    .unwrap();

    prj.add_source(
        "FailTest.t.sol",
        r#"
import "./test.sol";
contract FailTest is DSTest {
    function testNothing() external {
        assertTrue(false);
    }
}
   "#,
    )
    .unwrap();

    cmd.args(["test", "--match-path", "*src/ATest.t.sol"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for src/ATest.t.sol:ATest
[PASS] testPass() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
});

// tests that using the --match-path option works with absolute paths
forgetest!(can_test_with_match_path_absolute, |prj, cmd| {
    prj.insert_ds_test();

    prj.add_source(
        "ATest.t.sol",
        r#"
import "./test.sol";
contract ATest is DSTest {
    function testPass() external {
        assertTrue(true);
    }
}
   "#,
    )
    .unwrap();

    prj.add_source(
        "FailTest.t.sol",
        r#"
import "./test.sol";
contract FailTest is DSTest {
    function testNothing() external {
        assertTrue(false);
    }
}
   "#,
    )
    .unwrap();

    let test_path = prj.root().join("src/ATest.t.sol");
    let test_path = test_path.to_string_lossy();

    cmd.args(["test", "--match-path", test_path.as_ref()]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for src/ATest.t.sol:ATest
[PASS] testPass() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
});

const SIMPLE_CONTRACT: &str = r#"
import "./test.sol";
import "./console.sol";

contract SimpleContract {
    uint256 public num;

    function setValues(uint256 _num) public {
        num = _num;
    }
}

contract SimpleContractTest is DSTest {
    function test() public {
        SimpleContract c = new SimpleContract();
        c.setValues(100);
        console.logUint(100);
    }
}
   "#;

forgetest!(can_run_test_with_json_output_verbose, |prj, cmd| {
    prj.insert_ds_test();
    prj.insert_console();

    prj.add_source("Simple.t.sol", SIMPLE_CONTRACT).unwrap();

    // Assert that with verbose output the json output includes the traces
    cmd.args(["test", "-vvv", "--json"])
        .assert_success()
        .stdout_eq(file!["../fixtures/SimpleContractTestVerbose.json": Json]);
});

forgetest!(can_run_test_with_json_output_non_verbose, |prj, cmd| {
    prj.insert_ds_test();
    prj.insert_console();

    prj.add_source("Simple.t.sol", SIMPLE_CONTRACT).unwrap();

    // Assert that without verbose output the json output does not include the traces
    cmd.args(["test", "--json"])
        .assert_success()
        .stdout_eq(file!["../fixtures/SimpleContractTestNonVerbose.json": Json]);
});

// tests that `forge test` will pick up tests that are stored in the `test = <path>` config value
forgetest!(can_run_test_in_custom_test_folder, |prj, cmd| {
    prj.insert_ds_test();

    // explicitly set the test folder
    prj.update_config(|config| config.test = "nested/forge-tests".into());

    let config = cmd.config();
    assert_eq!(config.test, PathBuf::from("nested/forge-tests"));

    prj.add_source(
        "nested/forge-tests/MyTest.t.sol",
        r#"
import "../../test.sol";
contract MyTest is DSTest {
    function testTrue() public {
        assertTrue(true);
    }
}
   "#,
    )
    .unwrap();

    cmd.arg("test").assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for src/nested/forge-tests/MyTest.t.sol:MyTest
[PASS] testTrue() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
});

// checks that forge test repeatedly produces the same output
#[cfg(not(feature = "isolate-by-default"))]
forgetest_init!(can_test_repeatedly, |prj, cmd| {
    prj.clear();

    cmd.arg("test").assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 2 tests for test/Counter.t.sol:CounterTest
[PASS] testFuzz_SetNumber(uint256) (runs: 256, [AVG_GAS])
[PASS] test_Increment() ([GAS])
Suite result: ok. 2 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 2 tests passed, 0 failed, 0 skipped (2 total tests)

"#]]);

    for _ in 0..5 {
        cmd.assert_success().stdout_eq(str![[r#"
No files changed, compilation skipped

Ran 2 tests for test/Counter.t.sol:CounterTest
[PASS] testFuzz_SetNumber(uint256) (runs: 256, [AVG_GAS])
[PASS] test_Increment() ([GAS])
Suite result: ok. 2 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 2 tests passed, 0 failed, 0 skipped (2 total tests)

"#]]);
    }
});

// tests that `forge test` will run a test only once after changing the version
forgetest!(runs_tests_exactly_once_with_changed_versions, |prj, cmd| {
    prj.insert_ds_test();

    prj.add_source(
        "Contract.t.sol",
        r#"
pragma solidity *;

import "./test.sol";

contract ContractTest is DSTest {
    function setUp() public {}

    function testExample() public {
        assertTrue(true);
    }
}
   "#,
    )
    .unwrap();

    // pin version
    prj.update_config(|config| {
        config.solc = Some(SOLC_VERSION.into());
    });

    cmd.arg("test").assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for src/Contract.t.sol:ContractTest
[PASS] testExample() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);

    // pin version
    prj.update_config(|config| {
        config.solc = Some(OTHER_SOLC_VERSION.into());
    });

    cmd.forge_fuse().arg("test").assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for src/Contract.t.sol:ContractTest
[PASS] testExample() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
});

// tests that libraries are handled correctly in multiforking mode
#[cfg(not(feature = "isolate-by-default"))]
forgetest_init!(can_use_libs_in_multi_fork, |prj, cmd| {
    prj.wipe_contracts();

    prj.add_source(
        "Contract.sol",
        r"
library Library {
    function f(uint256 a, uint256 b) public pure returns (uint256) {
        return a + b;
    }
}

contract Contract {
    uint256 c;

    constructor() {
        c = Library.f(1, 2);
    }
}
   ",
    )
    .unwrap();

    let endpoint = rpc::next_http_archive_rpc_url();

    prj.add_test(
        "Contract.t.sol",
        &r#"
import "forge-std/Test.sol";
import "src/Contract.sol";

contract ContractTest is Test {
    function setUp() public {
        vm.createSelectFork("<url>");
    }

    function test() public {
        new Contract();
    }
}
   "#
        .replace("<url>", &endpoint),
    )
    .unwrap();

    cmd.arg("test").assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/Contract.t.sol:ContractTest
[PASS] test() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
});

static FAILING_TEST: &str = r#"
import "forge-std/Test.sol";

contract FailingTest is Test {
    function testShouldFail() public {
        assertTrue(false);
    }
}
"#;

forgetest_init!(exit_code_error_on_fail_fast, |prj, cmd| {
    prj.wipe_contracts();
    prj.add_source("failing_test", FAILING_TEST).unwrap();

    // set up command
    cmd.args(["test", "--fail-fast"]);

    // run command and assert error exit code
    cmd.assert_empty_stderr();
});

forgetest_init!(exit_code_error_on_fail_fast_with_json, |prj, cmd| {
    prj.wipe_contracts();

    prj.add_source("failing_test", FAILING_TEST).unwrap();
    // set up command
    cmd.args(["test", "--fail-fast", "--json"]);

    // run command and assert error exit code
    cmd.assert_empty_stderr();
});

// https://github.com/foundry-rs/foundry/pull/6531
forgetest_init!(fork_traces, |prj, cmd| {
    prj.wipe_contracts();

    let endpoint = rpc::next_http_archive_rpc_url();

    prj.add_test(
        "Contract.t.sol",
        &r#"
import {Test} from "forge-std/Test.sol";

interface IERC20 {
    function name() external view returns (string memory);
}

contract USDTCallingTest is Test {
    function test() public {
        vm.createSelectFork("<url>");
        IERC20(0xdAC17F958D2ee523a2206206994597C13D831ec7).name();
    }
}
   "#
        .replace("<url>", &endpoint),
    )
    .unwrap();

    cmd.args(["test", "-vvvv"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/Contract.t.sol:USDTCallingTest
[PASS] test() ([GAS])
Traces:
  [..] USDTCallingTest::test()
    ├─ [0] VM::createSelectFork("[..]")
    │   └─ ← [Return] 0
    ├─ [3110] 0xdAC17F958D2ee523a2206206994597C13D831ec7::name() [staticcall]
    │   └─ ← [Return] "Tether USD"
    └─ ← [Stop]

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
});

// https://github.com/foundry-rs/foundry/issues/6579
forgetest_init!(include_custom_types_in_traces, |prj, cmd| {
    prj.wipe_contracts();

    prj.add_test(
        "Contract.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

error PoolNotInitialized();
event MyEvent(uint256 a);

contract CustomTypesTest is Test {
    function testErr() public pure {
       revert PoolNotInitialized();
    }
    function testEvent() public {
       emit MyEvent(100);
    }
}
   "#,
    )
    .unwrap();

    cmd.args(["test", "-vvvv"]).assert_failure().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 2 tests for test/Contract.t.sol:CustomTypesTest
[FAIL: PoolNotInitialized()] testErr() ([GAS])
Traces:
  [247] CustomTypesTest::testErr()
    └─ ← [Revert] PoolNotInitialized()

[PASS] testEvent() ([GAS])
Traces:
  [1524] CustomTypesTest::testEvent()
    ├─ emit MyEvent(a: 100)
    └─ ← [Stop]

Suite result: FAILED. 1 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 1 failed, 0 skipped (2 total tests)

Failing tests:
Encountered 1 failing test in test/Contract.t.sol:CustomTypesTest
[FAIL: PoolNotInitialized()] testErr() ([GAS])

Encountered a total of 1 failing tests, 1 tests succeeded

"#]]);
});

forgetest_init!(can_test_transient_storage_with_isolation, |prj, cmd| {
    prj.wipe_contracts();

    prj.add_test(
        "Contract.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract TransientTester {
    function locked() public view returns (bool isLocked) {
        assembly {
            isLocked := tload(0)
        }
    }

    modifier lock() {
        require(!locked(), "locked");
        assembly {
            tstore(0, 1)
        }
        _;
    }

    function maybeReentrant(address target, bytes memory data) public lock {
        (bool success, bytes memory ret) = target.call(data);
        if (!success) {
            // forwards revert reason
            assembly {
                let ret_size := mload(ret)
                revert(add(32, ret), ret_size)
            }
        }
    }
}

contract TransientTest is Test {
    function test() public {
        TransientTester t = new TransientTester();
        vm.expectRevert(bytes("locked"));
        t.maybeReentrant(address(t), abi.encodeCall(TransientTester.maybeReentrant, (address(0), new bytes(0))));

        t.maybeReentrant(address(0), new bytes(0));
        assertEq(t.locked(), false);
    }
}
   "#,
    )
    .unwrap();

    cmd.args(["test", "-vvvv", "--isolate", "--evm-version", "cancun"]).assert_success();
});

forgetest_init!(
    #[ignore = "Too slow"]
    can_disable_block_gas_limit,
    |prj, cmd| {
        prj.wipe_contracts();

        let endpoint = rpc::next_http_archive_rpc_url();

        prj.add_test(
            "Contract.t.sol",
            &r#"
import {Test} from "forge-std/Test.sol";

contract C is Test {}

contract GasWaster {
    function waste() public {
        for (uint256 i = 0; i < 100; i++) {
            new C();
        }
    }
}

contract GasLimitTest is Test {
    function test() public {
        vm.createSelectFork("<rpc>");

        GasWaster waster = new GasWaster();
        waster.waste();
    }
}
   "#
            .replace("<rpc>", &endpoint),
        )
        .unwrap();

        cmd.args(["test", "-vvvv", "--isolate", "--disable-block-gas-limit"]).assert_success();
    }
);

forgetest!(test_match_path, |prj, cmd| {
    prj.add_source(
        "dummy",
        r"
contract Dummy {
    function testDummy() public {}
}
",
    )
    .unwrap();

    cmd.args(["test", "--match-path", "src/dummy.sol"]);
    cmd.assert_success();
});

forgetest_init!(should_not_shrink_fuzz_failure, |prj, cmd| {
    prj.wipe_contracts();

    // deterministic test so we always have 54 runs until test fails with overflow
    prj.update_config(|config| {
        config.fuzz.runs = 256;
        config.fuzz.seed = Some(U256::from(100));
    });

    prj.add_test(
        "CounterFuzz.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract Counter {
    uint256 public number = 0;

    function addOne(uint256 x) external pure returns (uint256) {
        return x + 100_000_000;
    }
}

contract CounterTest is Test {
    Counter public counter;

    function setUp() public {
        counter = new Counter();
    }

    function testAddOne(uint256 x) public view {
        assertEq(counter.addOne(x), x + 100_000_000);
    }
}
     "#,
    )
    .unwrap();

    // make sure there are only 61 runs (with proptest shrinking same test results in 298 runs)
    cmd.args(["test"]).assert_failure().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/CounterFuzz.t.sol:CounterTest
[FAIL: panic: arithmetic underflow or overflow (0x11); counterexample: calldata=0xa76d58f5fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffd args=[115792089237316195423570985008687907853269984665640564039457584007913129639933 [1.157e77]]] testAddOne(uint256) (runs: 84, [AVG_GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 1 failed, 0 skipped (1 total tests)

Failing tests:
Encountered 1 failing test in test/CounterFuzz.t.sol:CounterTest
[FAIL: panic: arithmetic underflow or overflow (0x11); counterexample: calldata=0xa76d58f5fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffd args=[115792089237316195423570985008687907853269984665640564039457584007913129639933 [1.157e77]]] testAddOne(uint256) (runs: 84, [AVG_GAS])

Encountered a total of 1 failing tests, 0 tests succeeded

"#]]);
});

forgetest_init!(should_exit_early_on_invariant_failure, |prj, cmd| {
    prj.wipe_contracts();
    prj.add_test(
        "CounterInvariant.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract Counter {
    uint256 public number = 0;

    function inc() external {
        number += 1;
    }
}

contract CounterTest is Test {
    Counter public counter;

    function setUp() public {
        counter = new Counter();
    }

    function invariant_early_exit() public view {
        assertTrue(counter.number() == 10, "wrong count");
    }
}
     "#,
    )
    .unwrap();

    // make sure invariant test exit early with 0 runs
    cmd.args(["test"]).assert_failure().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/CounterInvariant.t.sol:CounterTest
[FAIL: failed to set up invariant testing environment: wrong count] invariant_early_exit() (runs: 0, calls: 0, reverts: 0)
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 1 failed, 0 skipped (1 total tests)

Failing tests:
Encountered 1 failing test in test/CounterInvariant.t.sol:CounterTest
[FAIL: failed to set up invariant testing environment: wrong count] invariant_early_exit() (runs: 0, calls: 0, reverts: 0)

Encountered a total of 1 failing tests, 0 tests succeeded

"#]]);
});

forgetest_init!(should_replay_failures_only, |prj, cmd| {
    prj.wipe_contracts();
    prj.add_test(
        "ReplayFailures.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract ReplayFailuresTest is Test {
    function testA() public pure {
        require(2 > 1);
    }

    function testB() public pure {
        require(1 > 2, "testB failed");
    }

    function testC() public pure {
        require(2 > 1);
    }

    function testD() public pure {
        require(1 > 2, "testD failed");
    }
}
     "#,
    )
    .unwrap();

    cmd.args(["test"]).assert_failure().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 4 tests for test/ReplayFailures.t.sol:ReplayFailuresTest
[PASS] testA() ([GAS])
[FAIL: testB failed] testB() ([GAS])
[PASS] testC() ([GAS])
[FAIL: testD failed] testD() ([GAS])
Suite result: FAILED. 2 passed; 2 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 2 tests passed, 2 failed, 0 skipped (4 total tests)

Failing tests:
Encountered 2 failing tests in test/ReplayFailures.t.sol:ReplayFailuresTest
[FAIL: testB failed] testB() ([GAS])
[FAIL: testD failed] testD() ([GAS])

Encountered a total of 2 failing tests, 2 tests succeeded

"#]]);

    // Test failure filter should be persisted.
    assert!(prj.root().join("cache/test-failures").exists());

    // Perform only the 2 failing tests from last run.
    cmd.forge_fuse().args(["test", "--rerun"]).assert_failure().stdout_eq(str![[r#"
No files changed, compilation skipped

Ran 2 tests for test/ReplayFailures.t.sol:ReplayFailuresTest
[FAIL: testB failed] testB() ([GAS])
[FAIL: testD failed] testD() ([GAS])
Suite result: FAILED. 0 passed; 2 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 2 failed, 0 skipped (2 total tests)

Failing tests:
Encountered 2 failing tests in test/ReplayFailures.t.sol:ReplayFailuresTest
[FAIL: testB failed] testB() ([GAS])
[FAIL: testD failed] testD() ([GAS])

Encountered a total of 2 failing tests, 0 tests succeeded

"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/9285>
forgetest_init!(should_not_record_setup_failures, |prj, cmd| {
    prj.add_test(
        "ReplayFailures.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract SetupFailureTest is Test {
    function setUp() public {
        require(2 > 1);
    }

    function testA() public pure {
    }
}
     "#,
    )
    .unwrap();

    cmd.args(["test"]).assert_success();
    // Test failure filter should not be persisted if `setUp` failed.
    assert!(!prj.root().join("cache/test-failures").exists());
});

// https://github.com/foundry-rs/foundry/issues/7530
forgetest_init!(should_show_precompile_labels, |prj, cmd| {
    prj.wipe_contracts();

    prj.add_test(
        "Contract.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract PrecompileLabelsTest is Test {
    function testPrecompileLabels() public {
        vm.deal(address(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D), 1 ether);
        vm.deal(address(0x000000000000000000636F6e736F6c652e6c6f67), 1 ether);
        vm.deal(address(0x4e59b44847b379578588920cA78FbF26c0B4956C), 1 ether);
        vm.deal(address(0x1804c8AB1F12E6bbf3894d4083f33e07309d1f38), 1 ether);
        vm.deal(address(0xb4c79daB8f259C7Aee6E5b2Aa729821864227e84), 1 ether);
        vm.deal(address(1), 1 ether);
        vm.deal(address(2), 1 ether);
        vm.deal(address(3), 1 ether);
        vm.deal(address(4), 1 ether);
        vm.deal(address(5), 1 ether);
        vm.deal(address(6), 1 ether);
        vm.deal(address(7), 1 ether);
        vm.deal(address(8), 1 ether);
        vm.deal(address(9), 1 ether);
        vm.deal(address(10), 1 ether);
    }
}
   "#,
    )
    .unwrap();

    cmd.args(["test", "-vvvv"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/Contract.t.sol:PrecompileLabelsTest
[PASS] testPrecompileLabels() ([GAS])
Traces:
  [..] PrecompileLabelsTest::testPrecompileLabels()
    ├─ [0] VM::deal(VM: [0x7109709ECfa91a80626fF3989D68f67F5b1DD12D], 1000000000000000000 [1e18])
    │   └─ ← [Return]
    ├─ [0] VM::deal(console: [0x000000000000000000636F6e736F6c652e6c6f67], 1000000000000000000 [1e18])
    │   └─ ← [Return]
    ├─ [0] VM::deal(Create2Deployer: [0x4e59b44847b379578588920cA78FbF26c0B4956C], 1000000000000000000 [1e18])
    │   └─ ← [Return]
    ├─ [0] VM::deal(DefaultSender: [0x1804c8AB1F12E6bbf3894d4083f33e07309d1f38], 1000000000000000000 [1e18])
    │   └─ ← [Return]
    ├─ [0] VM::deal(DefaultTestContract: [0xb4c79daB8f259C7Aee6E5b2Aa729821864227e84], 1000000000000000000 [1e18])
    │   └─ ← [Return]
    ├─ [0] VM::deal(ECRecover: [0x0000000000000000000000000000000000000001], 1000000000000000000 [1e18])
    │   └─ ← [Return]
    ├─ [0] VM::deal(SHA-256: [0x0000000000000000000000000000000000000002], 1000000000000000000 [1e18])
    │   └─ ← [Return]
    ├─ [0] VM::deal(RIPEMD-160: [0x0000000000000000000000000000000000000003], 1000000000000000000 [1e18])
    │   └─ ← [Return]
    ├─ [0] VM::deal(Identity: [0x0000000000000000000000000000000000000004], 1000000000000000000 [1e18])
    │   └─ ← [Return]
    ├─ [0] VM::deal(ModExp: [0x0000000000000000000000000000000000000005], 1000000000000000000 [1e18])
    │   └─ ← [Return]
    ├─ [0] VM::deal(ECAdd: [0x0000000000000000000000000000000000000006], 1000000000000000000 [1e18])
    │   └─ ← [Return]
    ├─ [0] VM::deal(ECMul: [0x0000000000000000000000000000000000000007], 1000000000000000000 [1e18])
    │   └─ ← [Return]
    ├─ [0] VM::deal(ECPairing: [0x0000000000000000000000000000000000000008], 1000000000000000000 [1e18])
    │   └─ ← [Return]
    ├─ [0] VM::deal(Blake2F: [0x0000000000000000000000000000000000000009], 1000000000000000000 [1e18])
    │   └─ ← [Return]
    ├─ [0] VM::deal(PointEvaluation: [0x000000000000000000000000000000000000000A], 1000000000000000000 [1e18])
    │   └─ ← [Return]
    └─ ← [Stop]

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
});

// tests that `forge test` with config `show_logs: true` for fuzz tests will
// display `console.log` info
forgetest_init!(should_show_logs_when_fuzz_test, |prj, cmd| {
    prj.wipe_contracts();

    // run fuzz test 3 times
    prj.update_config(|config| {
        config.fuzz.runs = 3;
        config.fuzz.show_logs = true;
    });
    let config = cmd.config();
    assert_eq!(config.fuzz.runs, 3);

    prj.add_test(
        "ContractFuzz.t.sol",
        r#"
import {Test, console} from "forge-std/Test.sol";

contract ContractFuzz is Test {
    function testFuzzConsoleLog(uint256 x) public pure {
        console.log("inside fuzz test, x is:", x);
    }
}
    "#,
    )
    .unwrap();
    cmd.args(["test", "-vv"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/ContractFuzz.t.sol:ContractFuzz
[PASS] testFuzzConsoleLog(uint256) (runs: 3, [AVG_GAS])
Logs:
  inside fuzz test, x is: [..]
  inside fuzz test, x is: [..]
  inside fuzz test, x is: [..]

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
});

// tests that `forge test` with inline config `show_logs = true` for fuzz tests will
// display `console.log` info
forgetest_init!(should_show_logs_when_fuzz_test_inline_config, |prj, cmd| {
    prj.wipe_contracts();

    // run fuzz test 3 times
    prj.update_config(|config| {
        config.fuzz.runs = 3;
    });
    let config = cmd.config();
    assert_eq!(config.fuzz.runs, 3);

    prj.add_test(
        "ContractFuzz.t.sol",
        r#"
import {Test, console} from "forge-std/Test.sol";

contract ContractFuzz is Test {
    /// forge-config: default.fuzz.show-logs = true
    function testFuzzConsoleLog(uint256 x) public pure {
        console.log("inside fuzz test, x is:", x);
    }
}
    "#,
    )
    .unwrap();
    cmd.args(["test", "-vv"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/ContractFuzz.t.sol:ContractFuzz
[PASS] testFuzzConsoleLog(uint256) (runs: 3, [AVG_GAS])
Logs:
  inside fuzz test, x is: [..]
  inside fuzz test, x is: [..]
  inside fuzz test, x is: [..]

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
});

// tests that `forge test` with config `show_logs: false` for fuzz tests will not display
// `console.log` info
forgetest_init!(should_not_show_logs_when_fuzz_test, |prj, cmd| {
    prj.wipe_contracts();

    // run fuzz test 3 times
    prj.update_config(|config| {
        config.fuzz.runs = 3;
        config.fuzz.show_logs = false;
    });
    let config = cmd.config();
    assert_eq!(config.fuzz.runs, 3);

    prj.add_test(
        "ContractFuzz.t.sol",
        r#"
        import {Test, console} from "forge-std/Test.sol";
    contract ContractFuzz is Test {

      function testFuzzConsoleLog(uint256 x) public pure {
        console.log("inside fuzz test, x is:", x);
      }
    }
     "#,
    )
    .unwrap();
    cmd.args(["test", "-vv"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/ContractFuzz.t.sol:ContractFuzz
[PASS] testFuzzConsoleLog(uint256) (runs: 3, [AVG_GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
});

// tests that `forge test` with inline config `show_logs = false` for fuzz tests will not
// display `console.log` info
forgetest_init!(should_not_show_logs_when_fuzz_test_inline_config, |prj, cmd| {
    prj.wipe_contracts();

    // run fuzz test 3 times
    prj.update_config(|config| {
        config.fuzz.runs = 3;
    });
    let config = cmd.config();
    assert_eq!(config.fuzz.runs, 3);

    prj.add_test(
        "ContractFuzz.t.sol",
        r#"
import {Test, console} from "forge-std/Test.sol";

contract ContractFuzz is Test {
    /// forge-config: default.fuzz.show-logs = false
    function testFuzzConsoleLog(uint256 x) public pure {
        console.log("inside fuzz test, x is:", x);
    }
}
     "#,
    )
    .unwrap();
    cmd.args(["test", "-vv"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/ContractFuzz.t.sol:ContractFuzz
[PASS] testFuzzConsoleLog(uint256) (runs: 3, [AVG_GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
});

// tests internal functions trace
#[cfg(not(feature = "isolate-by-default"))]
forgetest_init!(internal_functions_trace, |prj, cmd| {
    prj.wipe_contracts();
    prj.clear();

    prj.add_test(
        "Simple",
        r#"
import {Test, console} from "forge-std/Test.sol";

contract SimpleContract {
    uint256 public num;
    address public addr;

    function _setNum(uint256 _num) internal returns(uint256 prev) {
        prev = num;
        num = _num;
    }

    function _setAddr(address _addr) internal returns(address prev) {
        prev = addr;
        addr = _addr;
    }

    function increment() public {
        _setNum(num + 1);
    }

    function setValues(uint256 _num, address _addr) public {
        _setNum(_num);
        _setAddr(_addr);
    }
}

contract SimpleContractTest is Test {
    function test() public {
        SimpleContract c = new SimpleContract();
        c.increment();
        c.setValues(100, address(0x123));
    }
}
     "#,
    )
    .unwrap();
    cmd.args(["test", "-vvvv", "--decode-internal"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/Simple.sol:SimpleContractTest
[PASS] test() ([GAS])
Traces:
  [..] SimpleContractTest::test()
    ├─ [165406] → new SimpleContract@0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f
    │   └─ ← [Return] 826 bytes of code
    ├─ [22630] SimpleContract::increment()
    │   ├─ [20147] SimpleContract::_setNum(1)
    │   │   └─ ← 0
    │   └─ ← [Stop]
    ├─ [23204] SimpleContract::setValues(100, 0x0000000000000000000000000000000000000123)
    │   ├─ [247] SimpleContract::_setNum(100)
    │   │   └─ ← 1
    │   ├─ [22336] SimpleContract::_setAddr(0x0000000000000000000000000000000000000123)
    │   │   └─ ← 0x0000000000000000000000000000000000000000
    │   └─ ← [Stop]
    └─ ← [Stop]

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
});

// tests internal functions trace with memory decoding
#[cfg(not(feature = "isolate-by-default"))]
forgetest_init!(internal_functions_trace_memory, |prj, cmd| {
    prj.wipe_contracts();
    prj.clear();

    prj.add_test(
        "Simple",
        r#"
import {Test, console} from "forge-std/Test.sol";

contract SimpleContract {
    string public str = "initial value";

    function _setStr(string memory _str) internal returns(string memory prev) {
        prev = str;
        str = _str;
    }

    function setStr(string memory _str) public {
        _setStr(_str);
    }
}

contract SimpleContractTest is Test {
    function test() public {
        SimpleContract c = new SimpleContract();
        c.setStr("new value");
    }
}
     "#,
    )
    .unwrap();
    cmd.args(["test", "-vvvv", "--decode-internal"]).assert_success().stdout_eq(str![[r#"
...
Traces:
  [..] SimpleContractTest::test()
    ├─ [370554] → new SimpleContract@0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f
    │   └─ ← [Return] 1737 bytes of code
    ├─ [2511] SimpleContract::setStr("new value")
    │   ├─ [1588] SimpleContract::_setStr("new value")
    │   │   └─ ← "initial value"
    │   └─ ← [Stop]
    └─ ← [Stop]
...
"#]]);
});

// tests that `forge test` with a seed produces deterministic random values for uint and addresses.
forgetest_init!(deterministic_randomness_with_seed, |prj, cmd| {
    prj.wipe_contracts();
    prj.add_test(
        "DeterministicRandomnessTest.t.sol",
        r#"
import {Test, console} from "forge-std/Test.sol";

contract DeterministicRandomnessTest is Test {

    function testDeterministicRandomUint() public {
        console.log(vm.randomUint());
        console.log(vm.randomUint());
        console.log(vm.randomUint());
    }

    function testDeterministicRandomUintRange() public {
        uint256 min = 0;
        uint256 max = 1000000000;
        console.log(vm.randomUint(min, max));
        console.log(vm.randomUint(min, max));
        console.log(vm.randomUint(min, max));
    }

    function testDeterministicRandomAddress() public {
        console.log(vm.randomAddress());
        console.log(vm.randomAddress());
        console.log(vm.randomAddress());
    }
}
"#,
    )
    .unwrap();

    // Extracts the test result section from the DeterministicRandomnessTest contract output.
    fn extract_test_result(out: &str) -> &str {
        let start = out
            .find("for test/DeterministicRandomnessTest.t.sol:DeterministicRandomnessTest")
            .unwrap();
        let end = out.find("Suite result: ok.").unwrap();
        &out[start..end]
    }

    // Run the test twice with the same seed and verify the outputs are the same.
    let seed1 = "0xa1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5b6c7d8e9f0a1b2";
    let out1 = cmd
        .args(["test", "--fuzz-seed", seed1, "-vv"])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let res1 = extract_test_result(&out1);

    let out2 = cmd
        .forge_fuse()
        .args(["test", "--fuzz-seed", seed1, "-vv"])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let res2 = extract_test_result(&out2);

    assert_eq!(res1, res2);

    // Run the test with another seed and verify the output differs.
    let seed2 = "0xb1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5b6c7d8e9f0a1b2";
    let out3 = cmd
        .forge_fuse()
        .args(["test", "--fuzz-seed", seed2, "-vv"])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let res3 = extract_test_result(&out3);
    assert_ne!(res3, res1);

    // Run the test without a seed and verify the outputs differs once again.
    cmd.forge_fuse();
    let out4 = cmd.args(["test", "-vv"]).assert_success().get_output().stdout_lossy();
    let res4 = extract_test_result(&out4);
    assert_ne!(res4, res1);
    assert_ne!(res4, res3);
});

// Tests that `pauseGasMetering` used at the end of test does not produce meaningless values.
// https://github.com/foundry-rs/foundry/issues/5491
forgetest_init!(gas_metering_pause_last_call, |prj, cmd| {
    prj.wipe_contracts();

    prj.add_test(
        "ATest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract ATest is Test {
    function testWeirdGas1() public {
        vm.pauseGasMetering();
    }

    function testWeirdGas2() public {
        uint256 a = 1;
        uint256 b = a + 1;
        require(b == 2, "b is not 2");
        vm.pauseGasMetering();
    }

    function testNormalGas() public {
        vm.pauseGasMetering();
        vm.resumeGasMetering();
    }

    function testWithAssembly() public {
        vm.pauseGasMetering();
        assembly {
            return(0, 0)
        }
    }
}
   "#,
    )
    .unwrap();

    cmd.args(["test"]).with_no_redact().assert_success().stdout_eq(str![[r#"
...
[PASS] testNormalGas() (gas: 3148)
[PASS] testWeirdGas1() (gas: 2986)
[PASS] testWeirdGas2() (gas: 3213)
[PASS] testWithAssembly() (gas: 3029)
...
"#]]);
});

// https://github.com/foundry-rs/foundry/issues/5564
forgetest_init!(gas_metering_expect_revert, |prj, cmd| {
    prj.wipe_contracts();

    prj.add_test(
        "ATest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
contract ATest is Test {
    error MyError();
    function testSelfMeteringRevert() public {
        vm.pauseGasMetering();
        vm.expectRevert(MyError.selector);
        this.selfReverts();
    }
    function selfReverts() external {
        vm.resumeGasMetering();
        revert MyError();
    }
}
   "#,
    )
    .unwrap();

    cmd.args(["test"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/ATest.t.sol:ATest
[PASS] testSelfMeteringRevert() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
});

// https://github.com/foundry-rs/foundry/issues/4523
forgetest_init!(gas_metering_gasleft, |prj, cmd| {
    prj.wipe_contracts();

    prj.add_test(
        "ATest.t.sol",
        r#"
import "forge-std/Test.sol";

contract ATest is Test {
    mapping(uint256 => bytes32) map;

    function test_GasMeter() public {
        vm.pauseGasMetering();
        consumeGas();
        vm.resumeGasMetering();

        consumeGas();
    }

    function test_GasLeft() public {
        consumeGas();

        uint256 start = gasleft();
        consumeGas();
        console.log("Gas cost:", start - gasleft());
    }

    function consumeGas() private {
        for (uint256 i = 0; i < 100; i++) {
            map[i] = keccak256(abi.encode(i));
        }
    }
}
   "#,
    )
    .unwrap();

    // Log and test gas cost should be similar.
    cmd.args(["test", "-vvvv"]).with_no_redact().assert_success().stdout_eq(str![[r#"
...
Logs:
  Gas cost: 50068

Traces:
  [2303684] ATest::test_GasLeft()
    ├─ [0] console::log("Gas cost:", 50068 [5.006e4]) [staticcall]
    │   └─ ← [Stop]
    └─ ← [Stop]

[PASS] test_GasMeter() (gas: 53097)
Traces:
  [53097] ATest::test_GasMeter()
    ├─ [0] VM::pauseGasMetering()
    │   └─ ← [Return]
    ├─ [0] VM::resumeGasMetering()
    │   └─ ← [Return]
    └─ ← [Stop]
...
"#]]);
});

// https://github.com/foundry-rs/foundry/issues/4370
forgetest_init!(pause_gas_metering_with_delete, |prj, cmd| {
    prj.wipe_contracts();

    prj.add_test(
        "ATest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
contract ATest is Test {
    uint a;
    function test_negativeGas () public {
        vm.pauseGasMetering();
        a = 100;
        vm.resumeGasMetering();
        delete a;
    }
}
   "#,
    )
    .unwrap();

    cmd.args(["test"]).with_no_redact().assert_success().stdout_eq(str![[r#"
...
[PASS] test_negativeGas() (gas: 96)
...
"#]]);
});

// tests `pauseTracing` and `resumeTracing` functions
#[cfg(not(feature = "isolate-by-default"))]
forgetest_init!(pause_tracing, |prj, cmd| {
    prj.wipe_contracts();
    prj.insert_ds_test();
    prj.insert_vm();
    prj.clear();

    prj.add_source(
        "Pause.t.sol",
        r#"
import {Vm} from "./Vm.sol";
import {DSTest} from "./test.sol";
contract TraceGenerator is DSTest {
    Vm vm = Vm(HEVM_ADDRESS);
    event DummyEvent(uint256 i);
    function call(uint256 i) public {
        emit DummyEvent(i);
    }
    function generate() public {
        for (uint256 i = 0; i < 10; i++) {
            if (i == 3) {
                vm.pauseTracing();
            }
            this.call(i);
            if (i == 7) {
                vm.resumeTracing();
            }
        }
    }
}
contract PauseTracingTest is DSTest {
    Vm vm = Vm(HEVM_ADDRESS);
    event DummyEvent(uint256 i);
    function setUp() public {
        emit DummyEvent(1);
        vm.pauseTracing();
        emit DummyEvent(2);
    }
    function test() public {
        emit DummyEvent(3);
        TraceGenerator t = new TraceGenerator();
        vm.resumeTracing();
        t.generate();
    }
}
     "#,
    )
    .unwrap();
    cmd.args(["test", "-vvvvv"]).assert_success().stdout_eq(str![[r#"
...
Traces:
  [7757] PauseTracingTest::setUp()
    ├─ emit DummyEvent(i: 1)
    ├─ [0] VM::pauseTracing() [staticcall]
    │   └─ ← [Return]
    └─ ← [Stop]

  [449649] PauseTracingTest::test()
    ├─ [0] VM::resumeTracing() [staticcall]
    │   └─ ← [Return]
    ├─ [22896] TraceGenerator::generate()
    │   ├─ [1589] TraceGenerator::call(0)
    │   │   ├─ emit DummyEvent(i: 0)
    │   │   └─ ← [Stop]
    │   ├─ [1589] TraceGenerator::call(1)
    │   │   ├─ emit DummyEvent(i: 1)
    │   │   └─ ← [Stop]
    │   ├─ [1589] TraceGenerator::call(2)
    │   │   ├─ emit DummyEvent(i: 2)
    │   │   └─ ← [Stop]
    │   ├─ [0] VM::pauseTracing() [staticcall]
    │   │   └─ ← [Return]
    │   ├─ [0] VM::resumeTracing() [staticcall]
    │   │   └─ ← [Return]
    │   ├─ [1589] TraceGenerator::call(8)
    │   │   ├─ emit DummyEvent(i: 8)
    │   │   └─ ← [Stop]
    │   ├─ [1589] TraceGenerator::call(9)
    │   │   ├─ emit DummyEvent(i: 9)
    │   │   └─ ← [Stop]
    │   └─ ← [Stop]
    └─ ← [Stop]
...
"#]]);
});

forgetest_init!(gas_metering_reset, |prj, cmd| {
    prj.wipe_contracts();
    prj.insert_ds_test();
    prj.insert_vm();
    prj.clear();

    prj.add_source(
        "ATest.t.sol",
        r#"
import {Vm} from "./Vm.sol";
import {DSTest} from "./test.sol";
contract B {
    function a() public returns (uint256) {
        return 100;
    }
}
contract ATest is DSTest {
    Vm vm = Vm(HEVM_ADDRESS);
    B b;
    uint256 a;

    function testResetGas() public {
        vm.resetGasMetering();
    }

    function testResetGas1() public {
        vm.resetGasMetering();
        b = new B();
        vm.resetGasMetering();
    }

    function testResetGas2() public {
        b = new B();
        b = new B();
        vm.resetGasMetering();
    }

    function testResetGas3() public {
        vm.resetGasMetering();
        b = new B();
        b = new B();
    }

    function testResetGas4() public {
        vm.resetGasMetering();
        b = new B();
        vm.resetGasMetering();
        b = new B();
    }

    function testResetGas5() public {
        vm.resetGasMetering();
        b = new B();
        vm.resetGasMetering();
        b = new B();
        vm.resetGasMetering();
    }

    function testResetGas6() public {
        vm.resetGasMetering();
        b = new B();
        b = new B();
        _reset();
        vm.resetGasMetering();
    }

    function testResetGas7() public {
        vm.resetGasMetering();
        b = new B();
        b = new B();
        _reset();
    }

    function testResetGas8() public {
        this.resetExternal();
    }

    function testResetGas9() public {
        this.resetExternal();
        vm.resetGasMetering();
    }

    function testResetNegativeGas() public {
        a = 100;
        vm.resetGasMetering();

        delete a;
    }

    function _reset() internal {
        vm.resetGasMetering();
    }

    function resetExternal() external {
        b = new B();
        b = new B();
        vm.resetGasMetering();
    }
}
     "#,
    )
    .unwrap();

    cmd.args(["test"]).with_no_redact().assert_success().stdout_eq(str![[r#"
...
[PASS] testResetGas() (gas: 96)
[PASS] testResetGas1() (gas: 96)
[PASS] testResetGas2() (gas: 96)
[PASS] testResetGas3() (gas: [..])
[PASS] testResetGas4() (gas: [..])
[PASS] testResetGas5() (gas: 96)
[PASS] testResetGas6() (gas: 96)
[PASS] testResetGas7() (gas: 96)
[PASS] testResetGas8() (gas: [..])
[PASS] testResetGas9() (gas: 96)
[PASS] testResetNegativeGas() (gas: 96)
...
"#]]);
});

// https://github.com/foundry-rs/foundry/issues/8705
forgetest_init!(test_expect_revert_decode, |prj, cmd| {
    prj.wipe_contracts();

    prj.add_test(
        "Counter.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
contract Counter {
    uint256 public number;
    error NumberNotEven(uint256 number);
    error RandomError();
    function setNumber(uint256 newNumber) public {
        if (newNumber % 2 != 0) {
            revert NumberNotEven(newNumber);
        }
        number = newNumber;
    }
}
contract CounterTest is Test {
    Counter public counter;
    function setUp() public {
        counter = new Counter();
        counter.setNumber(0);
    }
    function test_decode() public {
        vm.expectRevert(Counter.RandomError.selector);
        counter.setNumber(1);
    }
    function test_decode_with_args() public {
        vm.expectRevert(abi.encodePacked(Counter.NumberNotEven.selector, uint(2)));
        counter.setNumber(1);
    }
}
   "#,
    )
    .unwrap();

    cmd.args(["test"]).assert_failure().stdout_eq(str![[r#"
...
[FAIL: Error != expected error: NumberNotEven(1) != RandomError()] test_decode() ([GAS])
[FAIL: Error != expected error: NumberNotEven(1) != NumberNotEven(2)] test_decode_with_args() ([GAS])
...
"#]]);
});

// Tests that `expectPartialRevert` cheatcode partially matches revert data.
forgetest_init!(test_expect_partial_revert, |prj, cmd| {
    prj.wipe_contracts();
    prj.insert_ds_test();
    prj.insert_vm();
    prj.clear();

    prj.add_source(
        "Counter.t.sol",
        r#"
import {Vm} from "./Vm.sol";
import {DSTest} from "./test.sol";
contract Counter {
    error WrongNumber(uint256 number);
    function count() public pure {
        revert WrongNumber(0);
    }
}
contract CounterTest is DSTest {
    Vm vm = Vm(HEVM_ADDRESS);
    function testExpectPartialRevertWithSelector() public {
        Counter counter = new Counter();
        vm.expectPartialRevert(Counter.WrongNumber.selector);
        counter.count();
    }
    function testExpectPartialRevertWith4Bytes() public {
        Counter counter = new Counter();
        vm.expectPartialRevert(bytes4(0x238ace70));
        counter.count();
    }
    function testExpectRevert() public {
        Counter counter = new Counter();
        vm.expectRevert(Counter.WrongNumber.selector);
        counter.count();
    }
}
     "#,
    )
    .unwrap();

    cmd.args(["test"]).assert_failure().stdout_eq(str![[r#"
...
[PASS] testExpectPartialRevertWith4Bytes() ([GAS])
[PASS] testExpectPartialRevertWithSelector() ([GAS])
[FAIL: Error != expected error: WrongNumber(0) != custom error 0x238ace70] testExpectRevert() ([GAS])
...
"#]]);
});

forgetest_init!(test_assume_no_revert, |prj, cmd| {
    prj.wipe_contracts();
    prj.insert_ds_test();
    prj.insert_vm();
    prj.clear();

    prj.update_config(|config| {
        config.fuzz.runs = 100;
        config.fuzz.seed = Some(U256::from(100));
    });

    prj.add_source(
        "Counter.t.sol",
        r#"
import {Vm} from "./Vm.sol";
import {DSTest} from "./test.sol";
contract CounterWithRevert {
    error CountError();
    error CheckError();

    function count(uint256 a) public pure returns (uint256) {
        if (a > 1000 || a < 10) {
            revert CountError();
        }
        return 99999999;
    }
    function check(uint256 a) public pure {
        if (a == 99999999) {
            revert CheckError();
        }
    }
    function dummy() public pure {}
}

contract CounterRevertTest is DSTest {
    Vm vm = Vm(HEVM_ADDRESS);

    function test_assume_no_revert_pass(uint256 a) public {
        CounterWithRevert counter = new CounterWithRevert();
        vm.assumeNoRevert();
        a = counter.count(a);
        assertEq(a, 99999999);
    }
    function test_assume_no_revert_fail_assert(uint256 a) public {
        CounterWithRevert counter = new CounterWithRevert();
        vm.assumeNoRevert();
        a = counter.count(a);
        // Test should fail on next assertion.
        assertEq(a, 1);
    }
    function test_assume_no_revert_fail_in_2nd_call(uint256 a) public {
        CounterWithRevert counter = new CounterWithRevert();
        vm.assumeNoRevert();
        a = counter.count(a);
        // Test should revert here (not in scope of `assumeNoRevert` cheatcode).
        counter.check(a);
        assertEq(a, 99999999);
    }
    function test_assume_no_revert_fail_in_3rd_call(uint256 a) public {
        CounterWithRevert counter = new CounterWithRevert();
        vm.assumeNoRevert();
        a = counter.count(a);
        // Test `assumeNoRevert` applied to non reverting call should not be available for next reverting call.
        vm.assumeNoRevert();
        counter.dummy();
        // Test will revert here (not in scope of `assumeNoRevert` cheatcode).
        counter.check(a);
        assertEq(a, 99999999);
    }
}
     "#,
    )
    .unwrap();

    cmd.args(["test"]).with_no_redact().assert_failure().stdout_eq(str![[r#"
...
[FAIL; counterexample: [..]] test_assume_no_revert_fail_assert(uint256) [..]
[FAIL: CheckError(); counterexample: [..]] test_assume_no_revert_fail_in_2nd_call(uint256) [..]
[FAIL: CheckError(); counterexample: [..]] test_assume_no_revert_fail_in_3rd_call(uint256) [..]
[PASS] test_assume_no_revert_pass(uint256) [..]
...
"#]]);
});

forgetest_init!(skip_output, |prj, cmd| {
    prj.wipe_contracts();
    prj.insert_ds_test();
    prj.insert_vm();
    prj.clear();

    prj.add_source(
        "Counter.t.sol",
        r#"
        import {Vm} from "./Vm.sol";
        import {DSTest} from "./test.sol";

        contract Skips is DSTest {
            Vm constant vm = Vm(HEVM_ADDRESS);

            function test_skipUnit() public {
                vm.skip(true);
            }
            function test_skipUnitReason() public {
                vm.skip(true, "unit");
            }

            function test_skipFuzz(uint) public {
                vm.skip(true);
            }
            function test_skipFuzzReason(uint) public {
                vm.skip(true, "fuzz");
            }

            function invariant_skipInvariant() public {
                vm.skip(true);
            }
            function invariant_skipInvariantReason() public {
                vm.skip(true, "invariant");
            }
        }
    "#,
    )
    .unwrap();

    cmd.arg("test").assert_success().stdout_eq(str![[r#"
...
Ran 6 tests for src/Counter.t.sol:Skips
[SKIP] invariant_skipInvariant() (runs: 1, calls: 1, reverts: 1)
[SKIP: invariant] invariant_skipInvariantReason() (runs: 1, calls: 1, reverts: 1)
[SKIP] test_skipFuzz(uint256) (runs: 0, [AVG_GAS])
[SKIP: fuzz] test_skipFuzzReason(uint256) (runs: 0, [AVG_GAS])
[SKIP] test_skipUnit() ([GAS])
[SKIP: unit] test_skipUnitReason() ([GAS])
Suite result: ok. 0 passed; 0 failed; 6 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 0 failed, 6 skipped (6 total tests)

"#]]);
});

forgetest_init!(skip_setup, |prj, cmd| {
    prj.add_test(
        "Counter.t.sol",
        r#"
import "forge-std/Test.sol";

contract SkipCounterSetup is Test {

    function setUp() public {
        vm.skip(true, "skip counter test");
    }

    function test_require1() public pure {
        require(1 > 2);
    }

    function test_require2() public pure {
        require(1 > 2);
    }

    function test_require3() public pure {
        require(1 > 2);
    }
}
    "#,
    )
    .unwrap();

    cmd.args(["test", "--mc", "SkipCounterSetup"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/Counter.t.sol:SkipCounterSetup
[SKIP: skipped: skip counter test] setUp() ([GAS])
Suite result: ok. 0 passed; 0 failed; 1 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 0 failed, 1 skipped (1 total tests)

"#]]);
});

forgetest_init!(should_generate_junit_xml_report, |prj, cmd| {
    prj.wipe_contracts();
    prj.insert_ds_test();
    prj.insert_vm();
    prj.clear();

    prj.add_source(
        "JunitReportTest.t.sol",
        r#"
        import {Vm} from "./Vm.sol";
        import {DSTest} from "./test.sol";

        contract AJunitReportTest is DSTest {
            function test_junit_assert_fail() public {
                assert(1 > 2);
            }

            function test_junit_revert_fail() public {
                require(1 > 2, "Revert");
            }
        }

        contract BJunitReportTest is DSTest {
            Vm constant vm = Vm(HEVM_ADDRESS);
            function test_junit_pass() public {
                require(1 < 2, "Revert");
            }

            function test_junit_skip() public {
                vm.skip(true);
            }

            function test_junit_skip_with_message() public {
                vm.skip(true, "skipped test");
            }

            function test_junit_pass_fuzz(uint256 a) public {
            }
        }
   "#,
    )
    .unwrap();

    cmd.args(["test", "--junit"]).assert_failure().stdout_eq(str![[r#"
<?xml version="1.0" encoding="UTF-8"?>
<testsuites name="Test run" tests="6" failures="2" errors="0" timestamp="[..]" time="[..]">
    <testsuite name="src/JunitReportTest.t.sol:AJunitReportTest" tests="2" disabled="0" errors="0" failures="2" time="[..]">
        <testcase name="test_junit_assert_fail()" time="[..]">
            <failure message="panic: assertion failed (0x01)"/>
            <system-out>[FAIL: panic: assertion failed (0x01)] test_junit_assert_fail() ([GAS])</system-out>
        </testcase>
        <testcase name="test_junit_revert_fail()" time="[..]">
            <failure message="Revert"/>
            <system-out>[FAIL: Revert] test_junit_revert_fail() ([GAS])</system-out>
        </testcase>
        <system-out>Suite result: FAILED. 0 passed; 2 failed; 0 skipped; [ELAPSED]</system-out>
    </testsuite>
    <testsuite name="src/JunitReportTest.t.sol:BJunitReportTest" tests="4" disabled="2" errors="0" failures="0" time="[..]">
        <testcase name="test_junit_pass()" time="[..]">
            <system-out>[PASS] test_junit_pass() ([GAS])</system-out>
        </testcase>
        <testcase name="test_junit_pass_fuzz(uint256)" time="[..]">
            <system-out>[PASS] test_junit_pass_fuzz(uint256) (runs: 256, [AVG_GAS])</system-out>
        </testcase>
        <testcase name="test_junit_skip()" time="[..]">
            <skipped/>
            <system-out>[SKIP] test_junit_skip() ([GAS])</system-out>
        </testcase>
        <testcase name="test_junit_skip_with_message()" time="[..]">
            <skipped message="skipped test"/>
            <system-out>[SKIP: skipped test] test_junit_skip_with_message() ([GAS])</system-out>
        </testcase>
        <system-out>Suite result: ok. 2 passed; 0 failed; 2 skipped; [ELAPSED]</system-out>
    </testsuite>
</testsuites>


"#]]);
});

forgetest_init!(should_generate_junit_xml_report_with_logs, |prj, cmd| {
    prj.wipe_contracts();
    prj.add_source(
        "JunitReportTest.t.sol",
        r#"
import "forge-std/Test.sol";
contract JunitReportTest is Test {
    function test_junit_with_logs() public {
        console.log("Step1");
        console.log("Step2");
        console.log("Step3");
        assert(2 > 1);
    }
}
   "#,
    )
    .unwrap();

    cmd.args(["test", "--junit", "-vvvv"]).assert_success().stdout_eq(str![[r#"
<?xml version="1.0" encoding="UTF-8"?>
<testsuites name="Test run" tests="1" failures="0" errors="0" timestamp="[..]" time="[..]">
    <testsuite name="src/JunitReportTest.t.sol:JunitReportTest" tests="1" disabled="0" errors="0" failures="0" time="[..]">
        <testcase name="test_junit_with_logs()" time="[..]">
            <system-out>[PASS] test_junit_with_logs() ([GAS])/nLogs:/n  Step1/n  Step2/n  Step3/n</system-out>
        </testcase>
        <system-out>Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]</system-out>
    </testsuite>
</testsuites>


"#]]);
});

forgetest_init!(
    // Enable this if no cheatcodes are deprecated.
    // #[ignore = "no cheatcodes are deprecated"]
    test_deprecated_cheatcode_warning,
    |prj, cmd| {
        prj.add_test(
            "DeprecatedCheatcodeTest.t.sol",
            r#"
        import "forge-std/Test.sol";
        contract DeprecatedCheatcodeTest is Test {
            function test_deprecated_cheatcode() public view {
                vm.keyExists('{"a": 123}', ".a");
                vm.keyExists('{"a": 123}', ".a");
            }
        }

        contract DeprecatedCheatcodeFuzzTest is Test {
            function test_deprecated_cheatcode(uint256 a) public view {
                vm.keyExists('{"a": 123}', ".a");
            }
        }

        contract Counter {
            uint256 a;

            function count() public {
                a++;
            }
        }

        contract DeprecatedCheatcodeInvariantTest is Test {
            function setUp() public {
                Counter counter = new Counter();
            }

            /// forge-config: default.invariant.runs = 1
            function invariant_deprecated_cheatcode() public {
                vm.keyExists('{"a": 123}', ".a");
            }
        }
   "#,
        )
        .unwrap();

        // Tests deprecated cheatcode warning for unit tests.
        cmd.args(["test", "--mc", "DeprecatedCheatcodeTest"]).assert_success().stderr_eq(str![[
            r#"
Warning: the following cheatcode(s) are deprecated and will be removed in future versions:
  keyExists(string,string): replaced by `keyExistsJson`

"#
        ]]);

        // Tests deprecated cheatcode warning for fuzz tests.
        cmd.forge_fuse()
            .args(["test", "--mc", "DeprecatedCheatcodeFuzzTest"])
            .assert_success()
            .stderr_eq(str![[r#"
Warning: the following cheatcode(s) are deprecated and will be removed in future versions:
  keyExists(string,string): replaced by `keyExistsJson`

"#]]);

        // Tests deprecated cheatcode warning for invariant tests.
        cmd.forge_fuse()
            .args(["test", "--mc", "DeprecatedCheatcodeInvariantTest"])
            .assert_success()
            .stderr_eq(str![[r#"
Warning: the following cheatcode(s) are deprecated and will be removed in future versions:
  keyExists(string,string): replaced by `keyExistsJson`

"#]]);
    }
);

forgetest_init!(requires_single_test, |prj, cmd| {
    cmd.args(["test", "--debug"]).assert_failure().stderr_eq(str![[r#"
Error: 2 tests matched your criteria, but exactly 1 test must match in order to run the debugger.

Use --match-contract and --match-path to further limit the search.

"#]]);
    cmd.forge_fuse().args(["test", "--flamegraph"]).assert_failure().stderr_eq(str![[r#"
Error: 2 tests matched your criteria, but exactly 1 test must match in order to generate a flamegraph.

Use --match-contract and --match-path to further limit the search.

"#]]);
    cmd.forge_fuse().args(["test", "--flamechart"]).assert_failure().stderr_eq(str![[r#"
Error: 2 tests matched your criteria, but exactly 1 test must match in order to generate a flamechart.

Use --match-contract and --match-path to further limit the search.

"#]]);
});

// Test a script that calls vm.rememberKeys
forgetest_init!(script_testing, |prj, cmd| {
    prj
    .add_source(
        "Foo",
        r#"
import "forge-std/Script.sol";

interface Vm {
function rememberKeys(string calldata mnemonic, string calldata derivationPath, uint32 count) external returns (address[] memory keyAddrs);
}

contract WalletScript is Script {
function run() public {
    string memory mnemonic = "test test test test test test test test test test test junk";
    string memory derivationPath = "m/44'/60'/0'/0/";
    address[] memory wallets = Vm(address(vm)).rememberKeys(mnemonic, derivationPath, 3);
    for (uint256 i = 0; i < wallets.length; i++) {
        console.log(wallets[i]);
    }
}
}

contract FooTest {
    WalletScript public script;


    function setUp() public {
        script = new WalletScript();
    }

    function testWalletScript() public {
        script.run();
    }
}

"#,
    )
    .unwrap();

    cmd.args(["test", "--mt", "testWalletScript", "-vvv"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for src/Foo.sol:FooTest
[PASS] testWalletScript() ([GAS])
Logs:
  0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266
  0x70997970C51812dc3A010C7d01b50e0d17dc79C8
  0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC
...
"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/8995>
forgetest_init!(metadata_bytecode_traces, |prj, cmd| {
    prj.add_source(
        "ParentProxy.sol",
        r#"
import {Counter} from "./Counter.sol";

abstract contract ParentProxy {
    Counter impl;
    bytes data;

    constructor(Counter _implementation, bytes memory _data) {
        impl = _implementation;
        data = _data;
    }
}
   "#,
    )
    .unwrap();
    prj.add_source(
        "Proxy.sol",
        r#"
import {ParentProxy} from "./ParentProxy.sol";
import {Counter} from "./Counter.sol";

contract Proxy is ParentProxy {
    constructor(Counter _implementation, bytes memory _data)
        ParentProxy(_implementation, _data)
    {}
}
   "#,
    )
    .unwrap();

    prj.add_test(
        "MetadataTraceTest.t.sol",
        r#"
import {Counter} from "src/Counter.sol";
import {Proxy} from "src/Proxy.sol";

import {Test} from "forge-std/Test.sol";

contract MetadataTraceTest is Test {
    function test_proxy_trace() public {
        Counter counter = new Counter();
        new Proxy(counter, "");
    }
}
   "#,
    )
    .unwrap();

    cmd.args(["test", "--mt", "test_proxy_trace", "-vvvv"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/MetadataTraceTest.t.sol:MetadataTraceTest
[PASS] test_proxy_trace() ([GAS])
Traces:
  [..] MetadataTraceTest::test_proxy_trace()
    ├─ [..] → new Counter@0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f
    │   └─ ← [Return] 481 bytes of code
    ├─ [..] → new Proxy@0x2e234DAe75C793f67A35089C9d99245E1C58470b
    │   └─ ← [Return] 62 bytes of code
    └─ ← [Stop]

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);

    // Check consistent traces for running with no metadata.
    cmd.forge_fuse()
        .args(["test", "--mt", "test_proxy_trace", "-vvvv", "--no-metadata"])
        .assert_success()
        .stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/MetadataTraceTest.t.sol:MetadataTraceTest
[PASS] test_proxy_trace() ([GAS])
Traces:
  [..] MetadataTraceTest::test_proxy_trace()
    ├─ [..] → new Counter@0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f
    │   └─ ← [Return] 427 bytes of code
    ├─ [..] → new Proxy@0x2e234DAe75C793f67A35089C9d99245E1C58470b
    │   └─ ← [Return] 8 bytes of code
    └─ ← [Stop]

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
});

// Tests if dump of execution was created.
forgetest!(test_debug_with_dump, |prj, cmd| {
    prj.add_source(
        "dummy",
        r"
contract Dummy {
    function testDummy() public {}
}
",
    )
    .unwrap();

    let dump_path = prj.root().join("dump.json");

    cmd.args(["test", "--mt", "testDummy", "--debug", "--dump", dump_path.to_str().unwrap()]);
    cmd.assert_success();

    assert!(dump_path.exists());
});

forgetest_init!(test_assume_no_revert_with_data, |prj, cmd| {
    prj.update_config(|config| {
        config.fuzz.seed = Some(U256::from(100));
    });

    prj.add_source(
        "AssumeNoRevertTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

interface Vm {
    struct PotentialRevert {
        address reverter;
        bool partialMatch;
        bytes revertData;
    }
    function expectRevert() external;
    function assumeNoRevert() external pure;
    function assumeNoRevert(PotentialRevert calldata revertData) external pure;
    function assumeNoRevert(PotentialRevert[] calldata revertData) external pure;
    function expectRevert(bytes4 revertData, uint64 count) external;
    function assume(bool condition) external pure;
}

contract ReverterB {
    /// @notice has same error selectors as contract below to test the `reverter` param
    error MyRevert();
    error SpecialRevertWithData(uint256 x);

    function revertIf2(uint256 x) public pure returns (bool) {
        if (x == 2) {
            revert MyRevert();
        }
        return true;
    }

    function revertWithData() public pure returns (bool) {
        revert SpecialRevertWithData(2);
    }
}

contract Reverter {
    error MyRevert();
    error RevertWithData(uint256 x);
    error UnusedError();
    error ExpectedRevertCountZero();

    ReverterB public immutable subReverter;

    constructor() {
        subReverter = new ReverterB();
    }

    function myFunction() public pure returns (bool) {
        revert MyRevert();
    }

    function revertIf2(uint256 value) public pure returns (bool) {
        if (value == 2) {
            revert MyRevert();
        }
        return true;
    }

    function revertWithDataIf2(uint256 value) public pure returns (bool) {
        if (value == 2) {
            revert RevertWithData(2);
        }
        return true;
    }

    function twoPossibleReverts(uint256 x) public pure returns (bool) {
        if (x == 2) {
            revert MyRevert();
        } else if (x == 3) {
            revert RevertWithData(3);
        }
        return true;
    }

    function revertIf2Or3ExpectedRevertZero(uint256 x) public pure returns (bool) {
        if (x == 2) {
            revert ExpectedRevertCountZero();
        } else if (x == 3) {
            revert MyRevert();
        }
        return true;
    }
}

contract ReverterTest is Test {
    Reverter reverter;
    Vm _vm = Vm(VM_ADDRESS);

    function setUp() public {
        reverter = new Reverter();
    }

    /// @dev Test that `assumeNoRevert` does not reject an unanticipated error selector
    function testAssume_wrongSelector_fails(uint256 x) public view {
        _vm.assumeNoRevert(Vm.PotentialRevert({revertData: abi.encodeWithSelector(Reverter.UnusedError.selector), partialMatch: false, reverter: address(0)}));
        reverter.revertIf2(x);
    }

    /// @dev Test that `assumeNoRevert` does not reject an unanticipated error with extra data
    function testAssume_wrongData_fails(uint256 x) public view {
        _vm.assumeNoRevert(Vm.PotentialRevert({revertData: abi.encodeWithSelector(Reverter.RevertWithData.selector, 3), partialMatch: false, reverter: address(0)}));
        reverter.revertWithDataIf2(x);
    }

    /// @dev Test that `assumeNoRevert` correctly rejects an error selector from a different contract
    function testAssumeWithReverter_fails(uint256 x) public view {
        ReverterB subReverter = (reverter.subReverter());
        _vm.assumeNoRevert(Vm.PotentialRevert({revertData: abi.encodeWithSelector(Reverter.MyRevert.selector), partialMatch: false, reverter: address(reverter)}));
        subReverter.revertIf2(x);
    }

    /// @dev Test that `assumeNoRevert` correctly rejects one of two different error selectors when supplying a specific reverter
    function testMultipleAssumes_OneWrong_fails(uint256 x) public view {
        Vm.PotentialRevert[] memory revertData = new Vm.PotentialRevert[](2);
        revertData[0] = Vm.PotentialRevert({revertData: abi.encodeWithSelector(Reverter.MyRevert.selector), partialMatch: false, reverter: address(reverter)});
        revertData[1] = Vm.PotentialRevert({revertData: abi.encodeWithSelector(Reverter.RevertWithData.selector, 4), partialMatch: false, reverter: address(reverter)});
        _vm.assumeNoRevert(revertData);
        reverter.twoPossibleReverts(x);
    }

    /// @dev Test that `assumeNoRevert` assumptions are cleared after the first non-cheatcode external call
    function testMultipleAssumesClearAfterCall_fails(uint256 x) public view {
        _vm.assume(x != 3);
        Vm.PotentialRevert[] memory revertData = new Vm.PotentialRevert[](2);
        revertData[0] = Vm.PotentialRevert({revertData: abi.encodeWithSelector(Reverter.MyRevert.selector), partialMatch: false, reverter: address(0)});
        revertData[1] = Vm.PotentialRevert({revertData: abi.encodeWithSelector(Reverter.RevertWithData.selector, 4), partialMatch: false, reverter: address(reverter)});
        _vm.assumeNoRevert(revertData);
        reverter.twoPossibleReverts(x);

        reverter.twoPossibleReverts(2);
    }

    /// @dev Test that `assumeNoRevert` correctly rejects a generic assumeNoRevert call after any specific reason is provided
    function testMultipleAssumes_ThrowOnGenericNoRevert_AfterSpecific_fails(bytes4 selector) public view {
        _vm.assumeNoRevert(Vm.PotentialRevert({revertData: abi.encode(selector), partialMatch: false, reverter: address(0)}));
        _vm.assumeNoRevert();
        reverter.twoPossibleReverts(2);
    }

    function testAssumeThenExpectCountZeroFails(uint256 x) public {
        _vm.assumeNoRevert(
            Vm.PotentialRevert({
                revertData: abi.encodeWithSelector(Reverter.MyRevert.selector),
                partialMatch: false,
                reverter: address(0)
            })
        );
        _vm.expectRevert(Reverter.ExpectedRevertCountZero.selector, 0);
        reverter.revertIf2Or3ExpectedRevertZero(x);
    }

    function testExpectCountZeroThenAssumeFails(uint256 x) public {
        _vm.expectRevert(Reverter.ExpectedRevertCountZero.selector, 0);
        _vm.assumeNoRevert(
            Vm.PotentialRevert({
                revertData: abi.encodeWithSelector(Reverter.MyRevert.selector),
                partialMatch: false,
                reverter: address(0)
            })
        );
        reverter.revertIf2Or3ExpectedRevertZero(x);
    }

}"#,
    )
    .unwrap();
    cmd.args(["test", "--mc", "ReverterTest"]).assert_failure().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 8 tests for src/AssumeNoRevertTest.t.sol:ReverterTest
[FAIL: call reverted with 'FOUNDRY::ASSUME' when it was expected not to revert; counterexample: [..] testAssumeThenExpectCountZeroFails(uint256) (runs: [..], [AVG_GAS])
[FAIL: MyRevert(); counterexample: calldata=[..]] testAssumeWithReverter_fails(uint256) (runs: [..], [AVG_GAS])
[FAIL: RevertWithData(2); counterexample: [..]] testAssume_wrongData_fails(uint256) (runs: [..], [AVG_GAS])
[FAIL: MyRevert(); counterexample: [..]] testAssume_wrongSelector_fails(uint256) (runs: [..], [AVG_GAS])
[FAIL: call reverted with 'FOUNDRY::ASSUME' when it was expected not to revert; counterexample: [..]] testExpectCountZeroThenAssumeFails(uint256) (runs: [..], [AVG_GAS])
[FAIL: MyRevert(); counterexample: [..]] testMultipleAssumesClearAfterCall_fails(uint256) (runs: 0, [AVG_GAS])
[FAIL: RevertWithData(3); counterexample: [..]] testMultipleAssumes_OneWrong_fails(uint256) (runs: [..], [AVG_GAS])
[FAIL: vm.assumeNoRevert: you must make another external call prior to calling assumeNoRevert again; counterexample: [..]] testMultipleAssumes_ThrowOnGenericNoRevert_AfterSpecific_fails(bytes4) (runs: [..], [AVG_GAS])
...

"#]]);
});

forgetest_async!(can_get_broadcast_txs, |prj, cmd| {
    foundry_test_utils::util::initialize(prj.root());

    let (_api, handle) = spawn(NodeConfig::test().silent()).await;

    prj.insert_vm();
    prj.insert_ds_test();
    prj.insert_console();

    prj.add_source(
        "Counter.sol",
        r#"
        contract Counter {
    uint256 public number;

    function setNumber(uint256 newNumber) public {
        number = newNumber;
    }

    function increment() public {
        number++;
    }
}
    "#,
    )
    .unwrap();

    prj.add_script(
        "DeployCounter",
        r#"
        import "forge-std/Script.sol";
        import "src/Counter.sol";

        contract DeployCounter is Script {
            function run() public {
                vm.startBroadcast();

                Counter counter = new Counter();

                counter.increment();

                counter.setNumber(10);

                vm.stopBroadcast();
            }
        }
    "#,
    )
    .unwrap();

    prj.add_script(
        "DeployCounterWithCreate2",
        r#"
        import "forge-std/Script.sol";
        import "src/Counter.sol";

        contract DeployCounterWithCreate2 is Script {
            function run() public {
                vm.startBroadcast();

                bytes32 salt = bytes32(uint256(1337));
                Counter counter = new Counter{salt: salt}();

                counter.increment();

                counter.setNumber(20);

                vm.stopBroadcast();
            }
        }
    "#,
    )
    .unwrap();

    let test = r#"
        import {Vm} from "../src/Vm.sol";
        import {DSTest} from "../src/test.sol";
        import {console} from "../src/console.sol";

        contract GetBroadcastTest is DSTest {
            Vm constant vm = Vm(HEVM_ADDRESS);

            function test_getLatestBroadcast() external {
                // Gets the latest create
                Vm.BroadcastTxSummary memory broadcast = vm.getBroadcast(
                    "Counter",
                    31337,
                    Vm.BroadcastTxType.Create
                );

                console.log("latest create");
                console.log(broadcast.blockNumber);

                assertEq(broadcast.blockNumber, 1);

                // Gets the latest create2
                Vm.BroadcastTxSummary memory broadcast2 = vm.getBroadcast(
                    "Counter",
                    31337,
                    Vm.BroadcastTxType.Create2
                );

                console.log("latest create2");
                console.log(broadcast2.blockNumber);
                assertEq(broadcast2.blockNumber, 4);

                // Gets the latest call
                Vm.BroadcastTxSummary memory broadcast3 = vm.getBroadcast(
                    "Counter",
                    31337,
                    Vm.BroadcastTxType.Call
                );

                console.log("latest call");
                assertEq(broadcast3.blockNumber, 6);
            }

            function test_getBroadcasts() public {
                // Gets all calls
                Vm.BroadcastTxSummary[] memory broadcasts = vm.getBroadcasts(
                    "Counter",
                    31337,
                    Vm.BroadcastTxType.Call
                );

                assertEq(broadcasts.length, 4);
            }

            function test_getAllBroadcasts() public {
                // Gets all broadcasts
                Vm.BroadcastTxSummary[] memory broadcasts2 = vm.getBroadcasts(
                    "Counter",
                    31337
                );

                assertEq(broadcasts2.length, 6);
            }

            function test_getLatestDeployment() public {
                address deployedAddress = vm.getDeployment(
                    "Counter",
                    31337
                );

                assertEq(deployedAddress, address(0xD32c10E38A626Db0b0978B1A5828eb2957665668));
            }

            function test_getDeployments() public {
                address[] memory deployments = vm.getDeployments(
                    "Counter",
                    31337
                );

                assertEq(deployments.length, 2);
                assertEq(deployments[0], address(0xD32c10E38A626Db0b0978B1A5828eb2957665668)); // Create2 address - latest deployment
                assertEq(deployments[1], address(0x5FbDB2315678afecb367f032d93F642f64180aa3)); // Create address - oldest deployment
            }
}
    "#;

    prj.add_test("GetBroadcast", test).unwrap();

    let sender = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";

    cmd.args([
        "script",
        "DeployCounter",
        "--rpc-url",
        &handle.http_endpoint(),
        "--sender",
        sender,
        "--unlocked",
        "--broadcast",
        "--slow",
    ])
    .assert_success();

    cmd.forge_fuse()
        .args([
            "script",
            "DeployCounterWithCreate2",
            "--rpc-url",
            &handle.http_endpoint(),
            "--sender",
            sender,
            "--unlocked",
            "--broadcast",
            "--slow",
        ])
        .assert_success();

    let broadcast_path = prj.root().join("broadcast");

    // Check if the broadcast folder exists
    assert!(broadcast_path.exists() && broadcast_path.is_dir());

    cmd.forge_fuse().args(["test", "--mc", "GetBroadcastTest", "-vvv"]).assert_success();
});

// See <https://github.com/foundry-rs/foundry/issues/9297>
forgetest_init!(
    #[ignore = "RPC Service Unavailable"]
    test_roll_scroll_fork_with_cancun,
    |prj, cmd| {
        prj.add_test(
            "ScrollForkTest.t.sol",
            r#"

import {Test} from "forge-std/Test.sol";

contract ScrollForkTest is Test {
    function test_roll_scroll_fork_to_tx() public {
        vm.createSelectFork("https://scroll-mainnet.chainstacklabs.com/");
        bytes32 targetTxHash = 0xf94774a1f69bba76892141190293ffe85dd8d9ac90a0a2e2b114b8c65764014c;
        vm.rollFork(targetTxHash);
    }
}
   "#,
        )
        .unwrap();

        cmd.args(["test", "--mt", "test_roll_scroll_fork_to_tx", "--evm-version", "cancun"])
            .assert_success();
    }
);

// Test that only provider is included in failed fork error.
forgetest_init!(test_display_provider_on_error, |prj, cmd| {
    prj.add_test(
        "ForkTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract ForkTest is Test {
    function test_fork_err_message() public {
        vm.createSelectFork("https://eth-mainnet.g.alchemy.com/v2/DUMMY_KEY");
    }
}
   "#,
    )
    .unwrap();

    cmd.args(["test", "--mt", "test_fork_err_message"]).assert_failure().stdout_eq(str![[r#"
...
Ran 1 test for test/ForkTest.t.sol:ForkTest
[FAIL: vm.createSelectFork: could not instantiate forked environment with provider eth-mainnet.g.alchemy.com; failed to get latest block number; [..]] test_fork_err_message() ([GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]
...

"#]]);
});

// Tests that test traces display state changes when running with verbosity.
#[cfg(not(feature = "isolate-by-default"))]
forgetest_init!(should_show_state_changes, |prj, cmd| {
    cmd.args(["test", "--mt", "test_Increment", "-vvvvv"]).assert_success().stdout_eq(str![[r#"
...
Ran 1 test for test/Counter.t.sol:CounterTest
[PASS] test_Increment() ([GAS])
Traces:
  [137242] CounterTest::setUp()
    ├─ [96345] → new Counter@0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f
    │   └─ ← [Return] 481 bytes of code
    ├─ [2592] Counter::setNumber(0)
    │   └─ ← [Stop]
    └─ ← [Stop]

  [28783] CounterTest::test_Increment()
    ├─ [22418] Counter::increment()
    │   ├─  storage changes:
    │   │   @ 0: 0 → 1
    │   └─ ← [Stop]
    ├─ [424] Counter::number() [staticcall]
    │   └─ ← [Return] 1
    └─ ← [Stop]

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
});

// Tests that chained errors are properly displayed.
// <https://github.com/foundry-rs/foundry/issues/9161>
forgetest!(displays_chained_error, |prj, cmd| {
    prj.add_test(
        "Foo.t.sol",
        r#"
contract ContractTest {
    function test_anything(uint) public {}
}
   "#,
    )
    .unwrap();

    cmd.arg("test").arg("--gas-limit=100").assert_failure().stdout_eq(str![[r#"
...
Failing tests:
Encountered 1 failing test in test/Foo.t.sol:ContractTest
[FAIL: EVM error; transaction validation error: call [GAS_COST] exceeds the [GAS_LIMIT]] setUp() ([GAS])

Encountered a total of 1 failing tests, 0 tests succeeded

"#]]);
});

// Tests that `start/stopAndReturn` debugTraceRecording does not panic when running with
// verbosity > 3. <https://github.com/foundry-rs/foundry/issues/9526>
forgetest_init!(should_not_panic_on_debug_trace_verbose, |prj, cmd| {
    prj.add_test(
        "DebugTraceRecordingTest.t.sol",
        r#"
import "forge-std/Test.sol";
import {Counter} from "../src/Counter.sol";

contract DebugTraceRecordingTest is Test {
    function test_start_stop_recording() public {
        vm.startDebugTraceRecording();
        Counter counter = new Counter();
        counter.increment();
        vm.stopAndReturnDebugTraceRecording();
    }
}
     "#,
    )
    .unwrap();

    cmd.args(["test", "--mt", "test_start_stop_recording", "-vvvv"]).assert_success().stdout_eq(
        str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/DebugTraceRecordingTest.t.sol:DebugTraceRecordingTest
[PASS] test_start_stop_recording() ([GAS])
Traces:
  [..] DebugTraceRecordingTest::test_start_stop_recording()
    └─ ← [Stop]

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]],
    );
});

#[cfg(not(feature = "isolate-by-default"))]
forgetest_init!(colored_traces, |prj, cmd| {
    cmd.args(["test", "--mt", "test_Increment", "--color", "always", "-vvvvv"])
        .assert_success()
        .stdout_eq(file!["../fixtures/colored_traces.svg": TermSvg]);
});

// Tests that traces for successful tests can be suppressed by using `-s` flag.
// <https://github.com/foundry-rs/foundry/issues/9864>
#[cfg(not(feature = "isolate-by-default"))]
forgetest_init!(should_only_show_failed_tests_trace, |prj, cmd| {
    prj.add_test(
        "SuppressTracesTest.t.sol",
        r#"
import "forge-std/Test.sol";
import {Counter} from "../src/Counter.sol";

contract SuppressTracesTest is Test {
    Counter public counter;

    function setUp() public {
        counter = new Counter();
        counter.setNumber(0);
    }

    function test_increment_success() public {
        console.log("test increment success");
        counter.increment();
        assertEq(counter.number(), 1);
    }

    function test_increment_failure() public {
        console.log("test increment failure");
        counter.increment();
        assertEq(counter.number(), 100);
    }
}
     "#,
    )
    .unwrap();

    // Show traces and logs for failed test only.
    cmd.args(["test", "--mc", "SuppressTracesTest", "-vvvv", "-s"]).assert_failure().stdout_eq(
        str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 2 tests for test/SuppressTracesTest.t.sol:SuppressTracesTest
[FAIL: assertion failed: 1 != 100] test_increment_failure() ([GAS])
Logs:
  test increment failure

Traces:
  [137242] SuppressTracesTest::setUp()
    ├─ [96345] → new Counter@0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f
    │   └─ ← [Return] 481 bytes of code
    ├─ [2592] Counter::setNumber(0)
    │   └─ ← [Stop]
    └─ ← [Stop]

  [35200] SuppressTracesTest::test_increment_failure()
    ├─ [0] console::log("test increment failure") [staticcall]
    │   └─ ← [Stop]
    ├─ [22418] Counter::increment()
    │   └─ ← [Stop]
    ├─ [424] Counter::number() [staticcall]
    │   └─ ← [Return] 1
    ├─ [0] VM::assertEq(1, 100) [staticcall]
    │   └─ ← [Revert] assertion failed: 1 != 100
    └─ ← [Revert] assertion failed: 1 != 100

[PASS] test_increment_success() ([GAS])
Suite result: FAILED. 1 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 1 failed, 0 skipped (2 total tests)

Failing tests:
Encountered 1 failing test in test/SuppressTracesTest.t.sol:SuppressTracesTest
[FAIL: assertion failed: 1 != 100] test_increment_failure() ([GAS])

Encountered a total of 1 failing tests, 1 tests succeeded

"#]],
    );

    // Show traces and logs for all tests.
    cmd.forge_fuse()
        .args(["test", "--mc", "SuppressTracesTest", "-vvvv"])
        .assert_failure()
        .stdout_eq(str![[r#"
No files changed, compilation skipped

Ran 2 tests for test/SuppressTracesTest.t.sol:SuppressTracesTest
[FAIL: assertion failed: 1 != 100] test_increment_failure() ([GAS])
Logs:
  test increment failure

Traces:
  [137242] SuppressTracesTest::setUp()
    ├─ [96345] → new Counter@0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f
    │   └─ ← [Return] 481 bytes of code
    ├─ [2592] Counter::setNumber(0)
    │   └─ ← [Stop]
    └─ ← [Stop]

  [35200] SuppressTracesTest::test_increment_failure()
    ├─ [0] console::log("test increment failure") [staticcall]
    │   └─ ← [Stop]
    ├─ [22418] Counter::increment()
    │   └─ ← [Stop]
    ├─ [424] Counter::number() [staticcall]
    │   └─ ← [Return] 1
    ├─ [0] VM::assertEq(1, 100) [staticcall]
    │   └─ ← [Revert] assertion failed: 1 != 100
    └─ ← [Revert] assertion failed: 1 != 100

[PASS] test_increment_success() ([GAS])
Logs:
  test increment success

Traces:
  [32164] SuppressTracesTest::test_increment_success()
    ├─ [0] console::log("test increment success") [staticcall]
    │   └─ ← [Stop]
    ├─ [22418] Counter::increment()
    │   └─ ← [Stop]
    ├─ [424] Counter::number() [staticcall]
    │   └─ ← [Return] 1
    └─ ← [Stop]

Suite result: FAILED. 1 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 1 failed, 0 skipped (2 total tests)

Failing tests:
Encountered 1 failing test in test/SuppressTracesTest.t.sol:SuppressTracesTest
[FAIL: assertion failed: 1 != 100] test_increment_failure() ([GAS])

Encountered a total of 1 failing tests, 1 tests succeeded

"#]]);
});

forgetest_init!(catch_test_deployment_failure, |prj, cmd| {
    prj.add_test(
        "TestDeploymentFailure.t.sol",
        r#"
import "forge-std/Test.sol";
contract TestDeploymentFailure is Test {

    constructor() {
        require(false);
    }

    function setUp() public {
        require(true);
    }

    function test_something() public {
        require(1 == 1);
    }
}
    "#,
    )
    .unwrap();

    cmd.args(["t", "--mt", "test_something"]).assert_failure().stdout_eq(str![[r#"
...
Failing tests:
Encountered 1 failing test in test/TestDeploymentFailure.t.sol:TestDeploymentFailure
[FAIL: EvmError: Revert] constructor() ([GAS])
..."#]]);
});

// <https://github.com/foundry-rs/foundry/issues/10012>
forgetest_init!(state_diff_recording_with_revert, |prj, cmd| {
    prj.add_test(
        "TestStateDiffRevertFailure.t.sol",
        r#"
import "forge-std/Test.sol";
contract StateDiffRevertAtSameDepthTest is Test {
    function test_something() public {
        CounterTestA counter = new CounterTestA();
        counter.doSomething();
    }
}

contract CounterTestA is Test {
    function doSomething() public {
        vm.startStateDiffRecording();
        require(1 > 2);
    }
}
    "#,
    )
    .unwrap();

    cmd.args(["t", "--mt", "test_something"]).assert_failure();
});

// <https://github.com/foundry-rs/foundry/issues/5521>
forgetest_init!(should_apply_pranks_per_recorded_depth, |prj, cmd| {
    prj.add_test(
        "Counter.t.sol",
        r#"
import "forge-std/Test.sol";
contract CounterTest is Test {
    function test_stackPrank() public {
        address player = makeAddr("player");
        SenderLogger senderLogger = new SenderLogger();
        Contract c = new Contract();

        senderLogger.log(); // Log(ContractTest, DefaultSender)
        vm.startPrank(player, player);
        senderLogger.log(); // Log(player, player)
        c.f(); // vm.startPrank(player)
        senderLogger.log(); // Log(ContractTest, player) <- ContractTest should be player
        vm.stopPrank();
    }
}

contract Contract {
    Vm public constant vm = Vm(address(bytes20(uint160(uint256(keccak256("hevm cheat code"))))));

    function f() public {
        vm.startPrank(msg.sender);
    }
}

contract SenderLogger {
    event Log(address, address);

    function log() public {
        emit Log(msg.sender, tx.origin);
    }
}
    "#,
    )
    .unwrap();
    // Emits
    // Log(: player: [], : player: []) instead
    // Log(: ContractTest: [], : player: [])
    cmd.args(["test", "--mt", "test_stackPrank", "-vvvv"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/Counter.t.sol:CounterTest
[PASS] test_stackPrank() ([GAS])
Traces:
  [..] CounterTest::test_stackPrank()
    ├─ [..] VM::addr(<pk>) [staticcall]
    │   └─ ← [Return] player: [0x44E97aF4418b7a17AABD8090bEA0A471a366305C]
    ├─ [..] VM::label(player: [0x44E97aF4418b7a17AABD8090bEA0A471a366305C], "player")
    │   └─ ← [Return]
    ├─ [..] → new SenderLogger@0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f
    │   └─ ← [Return] 255 bytes of code
    ├─ [..] → new Contract@0x2e234DAe75C793f67A35089C9d99245E1C58470b
    │   └─ ← [Return] 542 bytes of code
    ├─ [..] SenderLogger::log()
    │   ├─ emit Log(: CounterTest: [0x7FA9385bE102ac3EAc297483Dd6233D62b3e1496], : DefaultSender: [0x1804c8AB1F12E6bbf3894d4083f33e07309d1f38])
    │   └─ ← [Stop]
    ├─ [..] VM::startPrank(player: [0x44E97aF4418b7a17AABD8090bEA0A471a366305C], player: [0x44E97aF4418b7a17AABD8090bEA0A471a366305C])
    │   └─ ← [Return]
    ├─ [..] SenderLogger::log()
    │   ├─ emit Log(: player: [0x44E97aF4418b7a17AABD8090bEA0A471a366305C], : player: [0x44E97aF4418b7a17AABD8090bEA0A471a366305C])
    │   └─ ← [Stop]
    ├─ [..] Contract::f()
    │   ├─ [..] VM::startPrank(player: [0x44E97aF4418b7a17AABD8090bEA0A471a366305C])
    │   │   └─ ← [Return]
    │   └─ ← [Stop]
    ├─ [..] SenderLogger::log()
    │   ├─ emit Log(: player: [0x44E97aF4418b7a17AABD8090bEA0A471a366305C], : player: [0x44E97aF4418b7a17AABD8090bEA0A471a366305C])
    │   └─ ← [Stop]
    ├─ [..] VM::stopPrank()
    │   └─ ← [Return]
    └─ ← [Stop]

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/10060>
forgetest_init!(should_redact_pk_in_sign_delegation, |prj, cmd| {
    prj.add_test(
        "Counter.t.sol",
        r#"
import "forge-std/Test.sol";
contract CounterTest is Test {
    function testCheckDelegation() external {
        (address alice, uint256 key) = makeAddrAndKey("alice");
        vm.signDelegation(address(0), key);
        vm.signAndAttachDelegation(address(0), key);
    }
}
    "#,
    )
    .unwrap();

    cmd.args(["test", "--mt", "testCheckDelegation", "-vvvv"]).assert_success().stdout_eq(str![[r#"
...
Ran 1 test for test/Counter.t.sol:CounterTest
[PASS] testCheckDelegation() ([GAS])
Traces:
  [..] CounterTest::testCheckDelegation()
    ├─ [0] VM::addr(<pk>) [staticcall]
    │   └─ ← [Return] alice: [0x328809Bc894f92807417D2dAD6b7C998c1aFdac6]
    ├─ [0] VM::label(alice: [0x328809Bc894f92807417D2dAD6b7C998c1aFdac6], "alice")
    │   └─ ← [Return]
    ├─ [0] VM::signDelegation(0x0000000000000000000000000000000000000000, "<pk>")
    │   └─ ← [Return] (0, 0x3d6ad67cc3dc94101a049f85f96937513a05485ae0f8b27545d25c4f71b12cf9, 0x3c0f2d62834f59d6ef0209e8a935f80a891a236eb18ac0e3700dd8f7ac8ae279, 0, 0x0000000000000000000000000000000000000000)
    ├─ [0] VM::signAndAttachDelegation(0x0000000000000000000000000000000000000000, "<pk>")
    │   └─ ← [Return] (0, 0x3d6ad67cc3dc94101a049f85f96937513a05485ae0f8b27545d25c4f71b12cf9, 0x3c0f2d62834f59d6ef0209e8a935f80a891a236eb18ac0e3700dd8f7ac8ae279, 0, 0x0000000000000000000000000000000000000000)
    └─ ← [Stop]
...

"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/10068>
forgetest_init!(can_upload_selectors_with_path, |prj, cmd| {
    prj.add_source(
        "CounterV1.sol",
        r#"
contract Counter {
    uint256 public number;

    function setNumberV1(uint256 newNumber) public {
        number = newNumber;
    }

    function incrementV1() public {
        number++;
    }
}
    "#,
    )
    .unwrap();

    prj.add_source(
        "CounterV2.sol",
        r#"
contract CounterV2 {
    uint256 public number;

    function setNumberV2(uint256 newNumber) public {
        number = newNumber;
    }

    function incrementV2() public {
        number++;
    }
}
    "#,
    )
    .unwrap();

    // Upload Counter without path fails as there are multiple contracts with same name.
    cmd.args(["selectors", "upload", "Counter"]).assert_failure().stderr_eq(str![[r#"
...
Error: Multiple contracts found with the name `Counter`
...

"#]]);

    // Upload without contract name should fail.
    cmd.forge_fuse().args(["selectors", "upload", "src/Counter.sol"]).assert_failure().stderr_eq(
        str![[r#"
...
Error: No contract name provided.
...

"#]],
    );

    // Upload single CounterV2.
    cmd.forge_fuse().args(["selectors", "upload", "CounterV2"]).assert_success().stdout_eq(str![[
        r#"
...
Uploading selectors for CounterV2...
...
Selectors successfully uploaded to OpenChain
...

"#
    ]]);

    // Upload CounterV1 with path.
    cmd.forge_fuse()
        .args(["selectors", "upload", "src/CounterV1.sol:Counter"])
        .assert_success()
        .stdout_eq(str![[r#"
...
Uploading selectors for Counter...
...
Selectors successfully uploaded to OpenChain
...

"#]]);

    // Upload Counter with path.
    cmd.forge_fuse()
        .args(["selectors", "upload", "src/Counter.sol:Counter"])
        .assert_success()
        .stdout_eq(str![[r#"
...
Uploading selectors for Counter...
...
Selectors successfully uploaded to OpenChain
...

"#]]);
});

// tests `interceptInitcode` function
forgetest_init!(intercept_initcode, |prj, cmd| {
    prj.wipe_contracts();
    prj.insert_ds_test();
    prj.insert_vm();
    prj.clear();

    prj.add_source(
        "InterceptInitcode.t.sol",
        r#"
import {Vm} from "./Vm.sol";
import {DSTest} from "./test.sol";

contract SimpleContract {
    uint256 public value;
    constructor(uint256 _value) {
        value = _value;
    }
}

contract InterceptInitcodeTest is DSTest {
    Vm vm = Vm(HEVM_ADDRESS);

    function testInterceptRegularCreate() public {
        // Set up interception
        vm.interceptInitcode();

        // Try to create a contract - this should revert with the initcode
        bytes memory initcode;
        try new SimpleContract(42) {
            assert(false);
        } catch (bytes memory interceptedInitcode) {
            initcode = interceptedInitcode;
        }

        // Verify the initcode contains the constructor argument
        assertTrue(initcode.length > 0, "initcode should not be empty");

        // The constructor argument is encoded as a 32-byte value at the end of the initcode
        // We need to convert the last 32 bytes to uint256
        uint256 value;
        assembly {
            value := mload(add(add(initcode, 0x20), sub(mload(initcode), 32)))
        }
        assertEq(value, 42, "initcode should contain constructor arg");
    }

    function testInterceptCreate2() public {
        // Set up interception
        vm.interceptInitcode();

        // Try to create a contract with CREATE2 - this should revert with the initcode
        bytes memory initcode;
        try new SimpleContract(1337) {
            assert(false);
        } catch (bytes memory interceptedInitcode) {
            initcode = interceptedInitcode;
        }

        // Verify the initcode contains the constructor argument
        assertTrue(initcode.length > 0, "initcode should not be empty");

        // The constructor argument is encoded as a 32-byte value at the end of the initcode
        uint256 value;
        assembly {
            value := mload(add(add(initcode, 0x20), sub(mload(initcode), 32)))
        }
        assertEq(value, 1337, "initcode should contain constructor arg");
    }

    function testInterceptMultiple() public {
        // First interception
        vm.interceptInitcode();
        bytes memory initcode1;
        try new SimpleContract(1) {
            assert(false);
        } catch (bytes memory interceptedInitcode) {
            initcode1 = interceptedInitcode;
        }

        // Second interception
        vm.interceptInitcode();
        bytes memory initcode2;
        try new SimpleContract(2) {
            assert(false);
        } catch (bytes memory interceptedInitcode) {
            initcode2 = interceptedInitcode;
        }

        // Verify different initcodes
        assertTrue(initcode1.length > 0, "first initcode should not be empty");
        assertTrue(initcode2.length > 0, "second initcode should not be empty");

        // Extract constructor arguments from both initcodes
        uint256 value1;
        uint256 value2;
        assembly {
            value1 := mload(add(add(initcode1, 0x20), sub(mload(initcode1), 32)))
            value2 := mload(add(add(initcode2, 0x20), sub(mload(initcode2), 32)))
        }
        assertEq(value1, 1, "first initcode should contain first arg");
        assertEq(value2, 2, "second initcode should contain second arg");
    }
}
     "#,
    )
    .unwrap();
    cmd.args(["test", "-vvvvv"]).assert_success();
});

// <https://github.com/foundry-rs/foundry/issues/10296>
// <https://github.com/foundry-rs/foundry/issues/10552>
forgetest_init!(should_preserve_fork_state_setup, |prj, cmd| {
    prj.wipe_contracts();
    prj.add_test(
        "Counter.t.sol",
        r#"
import "forge-std/Test.sol";
import {StdChains} from "forge-std/StdChains.sol";

contract CounterTest is Test {
    struct Domain {
        StdChains.Chain chain;
        uint256 forkId;
    }

    struct Bridge {
        Domain source;
        Domain destination;
        uint256 someVal;
    }

    struct SomeStruct {
        Domain domain;
        Bridge[] bridges;
    }

    mapping(uint256 => SomeStruct) internal data;

    function setUp() public {
        // Temporary workaround for `https://eth.llamarpc.com/` being down
        setChain("mainnet", ChainData({
            name: "mainnet",
            rpcUrl: "https://reth-ethereum.ithaca.xyz/rpc",
            chainId: 1
        }));

        StdChains.Chain memory chain1 = getChain("mainnet");
        StdChains.Chain memory chain2 = getChain("base");
        Domain memory domain1 = Domain(chain1, vm.createFork(chain1.rpcUrl, 22253716));
        Domain memory domain2 = Domain(chain2, vm.createFork(chain2.rpcUrl, 28839981));
        data[1].domain = domain1;
        data[2].domain = domain2;

        vm.selectFork(domain1.forkId);

        data[2].bridges.push(Bridge(domain1, domain2, 123));
        vm.selectFork(data[2].domain.forkId);
        vm.selectFork(data[1].domain.forkId);
        data[2].bridges.push(Bridge(domain1, domain2, 456));

        assertEq(data[2].bridges.length, 2);
    }

    function test_assert_storage() public {
        vm.selectFork(data[2].domain.forkId);
        assertEq(data[2].bridges.length, 2);
    }

    function test_modify_and_storage() public {
        data[3].domain = Domain(getChain("base"), vm.createFork(getChain("base").rpcUrl, 28839981));
        data[3].bridges.push(Bridge(data[1].domain, data[2].domain, 123));
        data[3].bridges.push(Bridge(data[1].domain, data[2].domain, 456));

        vm.selectFork(data[2].domain.forkId);
        assertEq(data[3].bridges.length, 2);
    }
}
    "#,
    )
    .unwrap();

    cmd.args(["test", "--mc", "CounterTest"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 2 tests for test/Counter.t.sol:CounterTest
[PASS] test_assert_storage() ([GAS])
[PASS] test_modify_and_storage() ([GAS])
Suite result: ok. 2 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 2 tests passed, 0 failed, 0 skipped (2 total tests)

"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/10544>
forgetest_init!(should_not_panic_on_cool, |prj, cmd| {
    prj.add_test(
        "Counter.t.sol",
        r#"
import "forge-std/Test.sol";
import {Counter} from "../src/Counter.sol";

contract CounterTest is Test {
    Counter counter = new Counter();

    function testCoolPanic() public {
        address alice = makeAddr("alice");
        vm.deal(alice, 10000 ether);
        counter.setNumber(1);
        vm.cool(address(counter));
        vm.prank(alice);
        payable(address(counter)).transfer(1 ether);
    }
}
    "#,
    )
    .unwrap();

    cmd.args(["test", "--mc", "CounterTest"]).assert_failure().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/Counter.t.sol:CounterTest
[FAIL: EvmError: Revert] testCoolPanic() ([GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 1 failed, 0 skipped (1 total tests)

Failing tests:
Encountered 1 failing test in test/Counter.t.sol:CounterTest
[FAIL: EvmError: Revert] testCoolPanic() ([GAS])

Encountered a total of 1 failing tests, 0 tests succeeded

"#]]);
});

#[cfg(not(feature = "isolate-by-default"))]
forgetest_init!(detailed_revert_when_calling_non_contract_address, |prj, cmd| {
    prj.add_test(
        "NonContractCallRevertTest.t.sol",
        r#"
import "forge-std/Test.sol";
import {Counter} from "../src/Counter.sol";

interface ICounter {
    function increment() external;
    function number() external returns (uint256);
    function random() external returns (uint256);
}

contract NonContractCallRevertTest is Test {
    Counter public counter;
    address constant ADDRESS = 0xdEADBEeF00000000000000000000000000000000;

    function setUp() public {
        counter = new Counter();
        counter.setNumber(1);
    }

    function test_non_supported_selector_call_failure() public {
        console.log("test non supported fn selector call failure");
        ICounter(address(counter)).random();
    }

    function test_non_contract_call_failure() public {
        console.log("test non contract call failure");
        ICounter(ADDRESS).number();
    }

    function test_non_contract_void_call_failure() public {
        console.log("test non contract (void) call failure");
        ICounter(ADDRESS).increment();
    }
}
     "#,
    )
    .unwrap();

    cmd.args(["test", "--mc", "NonContractCallRevertTest", "-vvv"])
        .assert_failure()
        .stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 3 tests for test/NonContractCallRevertTest.t.sol:NonContractCallRevertTest
[FAIL: call to non-contract address 0xdEADBEeF00000000000000000000000000000000] test_non_contract_call_failure() ([GAS])
Logs:
  test non contract call failure

Traces:
  [6350] NonContractCallRevertTest::test_non_contract_call_failure()
    ├─ [0] console::log("test non contract call failure") [staticcall]
    │   └─ ← [Stop]
    ├─ [0] 0xdEADBEeF00000000000000000000000000000000::number()
    │   └─ ← [Stop]
    └─ ← [Revert] call to non-contract address 0xdEADBEeF00000000000000000000000000000000

[FAIL: call to non-contract address 0xdEADBEeF00000000000000000000000000000000] test_non_contract_void_call_failure() ([GAS])
Logs:
  test non contract (void) call failure

Traces:
  [6215] NonContractCallRevertTest::test_non_contract_void_call_failure()
    ├─ [0] console::log("test non contract (void) call failure") [staticcall]
    │   └─ ← [Stop]
    └─ ← [Revert] call to non-contract address 0xdEADBEeF00000000000000000000000000000000

[FAIL: EvmError: Revert] test_non_supported_selector_call_failure() ([GAS])
Logs:
  test non supported fn selector call failure

Traces:
  [8620] NonContractCallRevertTest::test_non_supported_selector_call_failure()
    ├─ [0] console::log("test non supported fn selector call failure") [staticcall]
    │   └─ ← [Stop]
    ├─ [145] Counter::random()
    │   └─ ← [Revert] unrecognized function selector 0x5ec01e4d for contract 0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f, which has no fallback function.
    └─ ← [Revert] EvmError: Revert

Suite result: FAILED. 0 passed; 3 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 3 failed, 0 skipped (3 total tests)

Failing tests:
Encountered 3 failing tests in test/NonContractCallRevertTest.t.sol:NonContractCallRevertTest
[FAIL: call to non-contract address 0xdEADBEeF00000000000000000000000000000000] test_non_contract_call_failure() ([GAS])
[FAIL: call to non-contract address 0xdEADBEeF00000000000000000000000000000000] test_non_contract_void_call_failure() ([GAS])
[FAIL: EvmError: Revert] test_non_supported_selector_call_failure() ([GAS])

Encountered a total of 3 failing tests, 0 tests succeeded

"#]]);
});

#[cfg(not(feature = "isolate-by-default"))]
forgetest_init!(detailed_revert_when_delegatecalling_unlinked_library, |prj, cmd| {
    prj.add_test(
        "NonContractDelegateCallRevertTest.t.sol",
        r#"
import "forge-std/Test.sol";

library TestLibrary {
    function foo(uint256 a) public pure returns (uint256) {
        return a * 2;
    }
}

contract LibraryCaller {
    address public lib;

    constructor(address _lib) {
        lib = _lib;
    }

    function foobar(uint256 val) public returns (uint256) {
        (bool success, bytes memory data) = lib.delegatecall(
            abi.encodeWithSelector(TestLibrary.foo.selector, val)
        );

        assert(success);
        return abi.decode(data, (uint256));
    }
}

contract NonContractDelegateCallRevertTest is Test {
    function test_unlinked_library_call_failure() public {
        console.log("Test: Simulating call to unlinked library");
        LibraryCaller caller = new LibraryCaller(0xdEADBEeF00000000000000000000000000000000);

        caller.foobar(10);
    }
}
     "#,
    )
    .unwrap();

    cmd.args(["test", "--mc", "NonContractDelegateCallRevertTest", "-vvv"])
        .assert_failure()
        .stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/NonContractDelegateCallRevertTest.t.sol:NonContractDelegateCallRevertTest
[FAIL: delegatecall to non-contract address 0xdEADBEeF00000000000000000000000000000000 (usually an unliked library)] test_unlinked_library_call_failure() ([GAS])
Logs:
  Test: Simulating call to unlinked library

Traces:
  [255303] NonContractDelegateCallRevertTest::test_unlinked_library_call_failure()
    ├─ [0] console::log("Test: Simulating call to unlinked library") [staticcall]
    │   └─ ← [Stop]
    ├─ [214746] → new LibraryCaller@0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f
    │   └─ ← [Return] 960 bytes of code
    ├─ [3896] LibraryCaller::foobar(10)
    │   ├─ [0] 0xdEADBEeF00000000000000000000000000000000::foo(10) [delegatecall]
    │   │   └─ ← [Stop]
    │   └─ ← [Revert] delegatecall to non-contract address 0xdEADBEeF00000000000000000000000000000000 (usually an unliked library)
    └─ ← [Revert] delegatecall to non-contract address 0xdEADBEeF00000000000000000000000000000000 (usually an unliked library)

Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 1 failed, 0 skipped (1 total tests)

Failing tests:
Encountered 1 failing test in test/NonContractDelegateCallRevertTest.t.sol:NonContractDelegateCallRevertTest
[FAIL: delegatecall to non-contract address 0xdEADBEeF00000000000000000000000000000000 (usually an unliked library)] test_unlinked_library_call_failure() ([GAS])

Encountered a total of 1 failing tests, 0 tests succeeded

"#]]);
});

// This test is a copy of `error_event_decode_with_cache` in cast/tests/cli/selectors.rs
// but it uses `forge build` to check that the project selectors are cached by default.
forgetest_init!(build_with_selectors_cache, |prj, cmd| {
    prj.add_source(
        "LocalProjectContract",
        r#"
contract ContractWithCustomError {
    error AnotherValueTooHigh(uint256, address);
    event MyUniqueEventWithinLocalProject(uint256 a, address b);
}
   "#,
    )
    .unwrap();
    // Build and cache project selectors.
    cmd.forge_fuse().args(["build"]).assert_success();

    // Assert cast can decode custom error with local cache.
    cmd.cast_fuse()
        .args(["decode-error", "0x7191bc6200000000000000000000000000000000000000000000000000000000000000650000000000000000000000000000000000000000000000000000000000D0004F"])
        .assert_success()
        .stdout_eq(str![[r#"
AnotherValueTooHigh(uint256,address)
101
0x0000000000000000000000000000000000D0004F

"#]]);
    // Assert cast can decode event with local cache.
    cmd.cast_fuse()
        .args(["decode-event", "0xbd3699995dcc867b64dbb607be2c33be38df9134bef1178df13bfb9446e73104000000000000000000000000000000000000000000000000000000000000004e00000000000000000000000000000000000000000000000000000dd00000004e"])
        .assert_success()
        .stdout_eq(str![[r#"
MyUniqueEventWithinLocalProject(uint256,address)
78
0x00000000000000000000000000000DD00000004e

"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/11021>
forgetest_init!(revm_27_prank_bug_fix, |prj, cmd| {
    prj.add_test(
        "PrankBug.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Counter} from "../src/Counter.sol";

contract PrankTest is Test {
    Counter public counter;

    function setUp() public {
        vm.startPrank(address(0x123));
        counter = new Counter();
        vm.stopPrank();
    }

    function test_Increment() public {
        vm.startPrank(address(0x123));
        counter = new Counter();
        vm.stopPrank();

        counter.increment();
        assertEq(counter.number(), 1);
    }
}
"#,
    )
    .unwrap();

    cmd.args(["test", "--mc", "PrankTest", "-vvvvv"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/PrankBug.t.sol:PrankTest
[PASS] test_Increment() ([GAS])
Traces:
  [..] PrankTest::setUp()
    ├─ [0] VM::startPrank(0x0000000000000000000000000000000000000123)
    │   └─ ← [Return]
    ├─ [..] → new Counter@0x6cdBd1b486b8FBD4140e8cd6daAED05bE13eD914
    │   └─ ← [Return] 481 bytes of code
    ├─ [0] VM::stopPrank()
    │   └─ ← [Return]
    └─ ← [Stop]

  [..] PrankTest::test_Increment()
    ├─ [0] VM::startPrank(0x0000000000000000000000000000000000000123)
    │   └─ ← [Return]
    ├─ [..] → new Counter@0xc4B957Cd61beB9b9afD76204b30683EDAaaB51Ec
    │   └─ ← [Return] 481 bytes of code
    ├─ [0] VM::stopPrank()
    │   └─ ← [Return]
    ├─ [..] Counter::increment()
    │   ├─  storage changes:
    │   │   @ 0: 0 → 1
    │   └─ ← [Stop]
    ├─ [..] Counter::number() [staticcall]
    │   └─ ← [Return] 1
    ├─  storage changes:
    │   @ 31: 0x00000000000000000000006cdbd1b486b8fbd4140e8cd6daaed05be13ed91401 → 0x0000000000000000000000c4b957cd61beb9b9afd76204b30683edaaab51ec01
    └─ ← [Stop]

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
});

// tests proper reverts in fork mode for contracts with non-existent linked libraries.
// <https://github.com/foundry-rs/foundry/issues/11185>
#[cfg(not(feature = "isolate-by-default"))]
forgetest_init!(can_fork_test_with_non_existent_linked_library, |prj, cmd| {
    prj.update_config(|config| {
        config.libraries =
            vec!["src/Counter.sol:LibCounter:0x530008d2b058137d9c475b1b7d83984f1fcf1dd0".into()];
    });
    prj.add_source(
        "Counter.sol",
        r"
library LibCounter {
    function dummy() external pure returns (uint) {
        return 1;
    }
}

contract Counter {
    uint256 public number;

    constructor() {
        LibCounter.dummy();
    }

    function setNumber(uint256 newNumber) public {
        number = newNumber;
    }

    function increment() public {
        number++;
    }

    function dummy() external pure returns (uint) {
        return LibCounter.dummy();
    }
}
   ",
    )
    .unwrap();

    let endpoint = rpc::next_http_archive_rpc_url();

    prj.add_test(
        "Counter.t.sol",
        &r#"
import "forge-std/Test.sol";
import "src/Counter.sol";

contract CounterTest is Test {
    function test_select_fork() public {
        vm.createSelectFork("<url>");
        new Counter();
    }

    function test_roll_fork() public {
        vm.rollFork(block.number - 100);
        new Counter();
    }
}
   "#
        .replace("<url>", &endpoint),
    )
    .unwrap();

    cmd.args(["test", "--fork-url", &endpoint]).assert_failure().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 2 tests for test/Counter.t.sol:CounterTest
[FAIL: EvmError: Revert] test_roll_fork() ([GAS])
[FAIL: Contract 0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f does not exist and is not marked as persistent, see `vm.makePersistent()`] test_select_fork() ([GAS])
Suite result: FAILED. 0 passed; 2 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 2 failed, 0 skipped (2 total tests)

Failing tests:
Encountered 2 failing tests in test/Counter.t.sol:CounterTest
[FAIL: EvmError: Revert] test_roll_fork() ([GAS])
[FAIL: Contract 0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f does not exist and is not marked as persistent, see `vm.makePersistent()`] test_select_fork() ([GAS])

Encountered a total of 2 failing tests, 0 tests succeeded

"#]]);
});
