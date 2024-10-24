//! Contains various tests for `forge test`.

use alloy_primitives::U256;
use foundry_config::{Config, FuzzConfig};
use foundry_test_utils::{
    rpc, str,
    util::{OutputExt, OTHER_SOLC_VERSION, SOLC_VERSION},
    TestCommand,
};
use similar_asserts::assert_eq;
use std::{path::PathBuf, str::FromStr};

// tests that test filters are handled correctly
forgetest!(can_set_filter_values, |prj, cmd| {
    let patt = regex::Regex::new("test*").unwrap();
    let glob = globset::Glob::from_str("foo/bar/baz*").unwrap();

    // explicitly set patterns
    let config = Config {
        test_pattern: Some(patt.clone().into()),
        test_pattern_inverse: None,
        contract_pattern: Some(patt.clone().into()),
        contract_pattern_inverse: None,
        path_pattern: Some(glob.clone()),
        path_pattern_inverse: None,
        coverage_pattern_inverse: None,
        ..Default::default()
    };
    prj.write_config(config);

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
        console.log("Value set: ", 100);
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
    let config = Config { test: "nested/forge-tests".into(), ..Default::default() };
    prj.write_config(config);
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
    let config = Config { solc: Some(SOLC_VERSION.into()), ..Default::default() };
    prj.write_config(config);

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
    let config = Config { solc: Some(OTHER_SOLC_VERSION.into()), ..Default::default() };
    prj.write_config(config);

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

    let endpoint = rpc::next_http_archive_rpc_endpoint();

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

    let endpoint = rpc::next_http_archive_rpc_endpoint();

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
  [9516] USDTCallingTest::test()
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
  [254] CustomTypesTest::testErr()
    └─ ← [Revert] PoolNotInitialized()

[PASS] testEvent() ([GAS])
Traces:
  [1268] CustomTypesTest::testEvent()
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

forgetest_init!(can_disable_block_gas_limit, |prj, cmd| {
    prj.wipe_contracts();

    let endpoint = rpc::next_http_archive_rpc_endpoint();

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
});

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
    let config = Config {
        fuzz: { FuzzConfig { runs: 256, seed: Some(U256::from(100)), ..Default::default() } },
        ..Default::default()
    };
    prj.write_config(config);

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
[FAIL: panic: arithmetic underflow or overflow (0x11); counterexample: calldata=0xa76d58f5ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff args=[115792089237316195423570985008687907853269984665640564039457584007913129639935 [1.157e77]]] testAddOne(uint256) (runs: 61, [AVG_GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 1 failed, 0 skipped (1 total tests)

Failing tests:
Encountered 1 failing test in test/CounterFuzz.t.sol:CounterTest
[FAIL: panic: arithmetic underflow or overflow (0x11); counterexample: calldata=0xa76d58f5ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff args=[115792089237316195423570985008687907853269984665640564039457584007913129639935 [1.157e77]]] testAddOne(uint256) (runs: 61, [AVG_GAS])

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
[FAIL: revert: testB failed] testB() ([GAS])
[PASS] testC() ([GAS])
[FAIL: revert: testD failed] testD() ([GAS])
Suite result: FAILED. 2 passed; 2 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 2 tests passed, 2 failed, 0 skipped (4 total tests)

Failing tests:
Encountered 2 failing tests in test/ReplayFailures.t.sol:ReplayFailuresTest
[FAIL: revert: testB failed] testB() ([GAS])
[FAIL: revert: testD failed] testD() ([GAS])

Encountered a total of 2 failing tests, 2 tests succeeded

"#]]);

    // Test failure filter should be persisted.
    assert!(prj.root().join("cache/test-failures").exists());

    // Perform only the 2 failing tests from last run.
    cmd.forge_fuse().args(["test", "--rerun"]).assert_failure().stdout_eq(str![[r#"
No files changed, compilation skipped

Ran 2 tests for test/ReplayFailures.t.sol:ReplayFailuresTest
[FAIL: revert: testB failed] testB() ([GAS])
[FAIL: revert: testD failed] testD() ([GAS])
Suite result: FAILED. 0 passed; 2 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 2 failed, 0 skipped (2 total tests)

Failing tests:
Encountered 2 failing tests in test/ReplayFailures.t.sol:ReplayFailuresTest
[FAIL: revert: testB failed] testB() ([GAS])
[FAIL: revert: testD failed] testD() ([GAS])

Encountered a total of 2 failing tests, 0 tests succeeded

"#]]);
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
  [9474] PrecompileLabelsTest::testPrecompileLabels()
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
    let config = Config {
        fuzz: { FuzzConfig { runs: 3, show_logs: true, ..Default::default() } },
        ..Default::default()
    };
    prj.write_config(config);
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
    let config =
        Config { fuzz: { FuzzConfig { runs: 3, ..Default::default() } }, ..Default::default() };
    prj.write_config(config);
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
    let config = Config {
        fuzz: { FuzzConfig { runs: 3, show_logs: false, ..Default::default() } },
        ..Default::default()
    };
    prj.write_config(config);
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
    let config =
        Config { fuzz: { FuzzConfig { runs: 3, ..Default::default() } }, ..Default::default() };
    prj.write_config(config);
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

    // Disable optimizer because for simple contract most functions will get inlined.
    prj.write_config(Config { optimizer: false, ..Default::default() });

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
  [250463] SimpleContractTest::test()
    ├─ [171014] → new SimpleContract@0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f
    │   └─ ← [Return] 854 bytes of code
    ├─ [22638] SimpleContract::increment()
    │   ├─ [20150] SimpleContract::_setNum(1)
    │   │   └─ ← 0
    │   └─ ← [Stop] 
    ├─ [23219] SimpleContract::setValues(100, 0x0000000000000000000000000000000000000123)
    │   ├─ [250] SimpleContract::_setNum(100)
    │   │   └─ ← 1
    │   ├─ [22339] SimpleContract::_setAddr(0x0000000000000000000000000000000000000123)
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

    // Disable optimizer because for simple contract most functions will get inlined.
    prj.write_config(Config { optimizer: false, ..Default::default() });

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
    cmd.args(["test", "-vvvv", "--decode-internal", "test"]).assert_success().stdout_eq(str![[
        r#"
...
Traces:
  [421947] SimpleContractTest::test()
    ├─ [385978] → new SimpleContract@0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f
    │   └─ ← [Return] 1814 bytes of code
    ├─ [2534] SimpleContract::setStr("new value")
    │   ├─ [1600] SimpleContract::_setStr("new value")
    │   │   └─ ← "initial value"
    │   └─ ← [Stop] 
    └─ ← [Stop] 
...
"#
    ]]);
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
[PASS] testNormalGas() (gas: 3202)
[PASS] testWeirdGas1() (gas: 3040)
[PASS] testWeirdGas2() (gas: 3148)
[PASS] testWithAssembly() (gas: 3083)
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
  Gas cost: 34468
...
[PASS] test_GasMeter() (gas: 37512)
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
[PASS] test_negativeGas() (gas: 0)
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
  [7285] PauseTracingTest::setUp()
    ├─ emit DummyEvent(i: 1)
    ├─ [0] VM::pauseTracing() [staticcall]
    │   └─ ← [Return] 
    └─ ← [Stop] 

  [294725] PauseTracingTest::test()
    ├─ [0] VM::resumeTracing() [staticcall]
    │   └─ ← [Return] 
    ├─ [18373] TraceGenerator::generate()
    │   ├─ [1280] TraceGenerator::call(0)
    │   │   ├─ emit DummyEvent(i: 0)
    │   │   └─ ← [Stop] 
    │   ├─ [1280] TraceGenerator::call(1)
    │   │   ├─ emit DummyEvent(i: 1)
    │   │   └─ ← [Stop] 
    │   ├─ [1280] TraceGenerator::call(2)
    │   │   ├─ emit DummyEvent(i: 2)
    │   │   └─ ← [Stop] 
    │   ├─ [0] VM::pauseTracing() [staticcall]
    │   │   └─ ← [Return] 
    │   ├─ [0] VM::resumeTracing() [staticcall]
    │   │   └─ ← [Return] 
    │   ├─ [1280] TraceGenerator::call(8)
    │   │   ├─ emit DummyEvent(i: 8)
    │   │   └─ ← [Stop] 
    │   ├─ [1280] TraceGenerator::call(9)
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
[PASS] testResetGas() (gas: 40)
[PASS] testResetGas1() (gas: 40)
[PASS] testResetGas2() (gas: 40)
[PASS] testResetGas3() (gas: [..])
[PASS] testResetGas4() (gas: [..])
[PASS] testResetGas5() (gas: 40)
[PASS] testResetGas6() (gas: 40)
[PASS] testResetGas7() (gas: 49)
[PASS] testResetGas8() (gas: [..])
[PASS] testResetGas9() (gas: 40)
[PASS] testResetNegativeGas() (gas: 0)
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

    let config = Config {
        fuzz: { FuzzConfig { runs: 100, seed: Some(U256::from(100)), ..Default::default() } },
        ..Default::default()
    };
    prj.write_config(config);

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
            <failure message="revert: Revert"/>
            <system-out>[FAIL: revert: Revert] test_junit_revert_fail() ([GAS])</system-out>
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

forgetest_init!(deprecated_regex_arg, |prj, cmd| {
    cmd.args(["test", "--decode-internal", "test_Increment"]).assert_success().stderr_eq(str![[r#"
Warning: specifying argument for --decode-internal is deprecated and will be removed in the future, use --match-test instead

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
  [152142] MetadataTraceTest::test_proxy_trace()
    ├─ [49499] → new Counter@0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f
    │   └─ ← [Return] 247 bytes of code
    ├─ [37978] → new Proxy@0x2e234DAe75C793f67A35089C9d99245E1C58470b
    │   └─ ← [Return] 63 bytes of code
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
  [130521] MetadataTraceTest::test_proxy_trace()
    ├─ [38693] → new Counter@0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f
    │   └─ ← [Return] 193 bytes of code
    ├─ [27175] → new Proxy@0x2e234DAe75C793f67A35089C9d99245E1C58470b
    │   └─ ← [Return] 9 bytes of code
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

    cmd.args(["test", "--debug", "testDummy", "--dump", dump_path.to_str().unwrap()]);
    cmd.assert_success();

    assert!(dump_path.exists());
});

forgetest_init!(assume_no_revert_tests, |prj, cmd| {
    prj.wipe_contracts();
    let config = Config {
        fuzz: { FuzzConfig { runs: 256, seed: Some(U256::from(100)), ..Default::default() } },
        ..Default::default()
    };
    prj.write_config(config);
    prj.insert_ds_test();
    prj.insert_vm();
    prj.clear();

    prj.add_source(
        "AssumeNoRevertTest.t.sol",
        r#"

    import {Test} from "forge-std/Test.sol";
    import {Vm} from "./Vm.sol";
    
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
    }
    
    contract ReverterTest is Test {
        Reverter reverter;
        Vm _vm = Vm(VM_ADDRESS);
    
        function setUp() public {
            reverter = new Reverter();
        }
    
        /// @dev Test that `assumeNoRevert` anticipates and correctly rejects a specific error selector
        function testAssumeSelector(uint256 x) public view {
            _vm.assumeNoRevert(Reverter.MyRevert.selector);
            reverter.revertIf2(x);
        }
    
        /// @dev Test that `assumeNoRevert` anticipates and correctly rejects a specific error selector and data
        function testAssumeWithDataSingle(uint256 x) public view {
            _vm.assumeNoRevert(abi.encodeWithSelector(Reverter.RevertWithData.selector, 2));
            reverter.revertWithDataIf2(x);
        }
    
        /// @dev Test that `assumeNoPartialRevert` anticipates and correctly rejects a specific error selector with any extra data
        function testAssumeWithDataPartial(uint256 x) public view {
            _vm.assumeNoPartialRevert(Reverter.RevertWithData.selector);
            reverter.revertWithDataIf2(x);
        }
        
        /// @dev Test that `assumeNoRevert` assumptions are not cleared after a cheatcode call
        function testAssumeNotClearedAfterCheatcodeCall(uint256 x) public {
            _vm.assumeNoRevert(Reverter.MyRevert.selector);
            _vm.warp(block.timestamp + 1000);
            reverter.revertIf2(x);
        }
        
        /// @dev Test that `assumeNoRevert` correctly rejects two different error selectors
        function testMultipleAssumesPasses(uint256 x) public view {
            _vm.assumeNoRevert(abi.encodeWithSelector(Reverter.MyRevert.selector), address(reverter));
            _vm.assumeNoRevert(abi.encodeWithSelector(Reverter.RevertWithData.selector, 3), address(reverter));
            reverter.twoPossibleReverts(x);
        }
      
        /// @dev Test that `assumeNoPartialRevert` correctly interacts with `assumeNoRevert`
        function testMultipleAssumes_Partial(uint256 x) public view {
            _vm.assumeNoPartialRevert(Reverter.RevertWithData.selector);
            _vm.assumeNoRevert(Reverter.MyRevert.selector);
            reverter.twoPossibleReverts(x);
        } 
        
        /// @dev Test that calling `assumeNoRevert` after `expectRevert` results in an error
        function testExpectThenAssumeFails() public {
            _vm._expectCheatcodeRevert();
            _vm.assumeNoRevert();
            reverter.revertIf2(1);
        }
    
        /// @dev Test that `assumeNoRevert` does not reject an unanticipated error selector
        function testAssume_wrongSelector_fails(uint256 x) public view {
            _vm.assumeNoRevert(Reverter.UnusedError.selector);
            reverter.revertIf2(x);
        }
    
        /// @dev Test that `assumeNoRevert` does not reject an unanticipated error with extra data
        function testAssume_wrongData_fails(uint256 x) public view {
            _vm.assumeNoRevert(abi.encodeWithSelector(Reverter.RevertWithData.selector, 3));
            reverter.revertWithDataIf2(x);
        }
    
        /// @dev Test that `assumeNoRevert` correctly rejects an error selector from a different contract
        function testAssumeWithReverter_fails(uint256 x) public view {
            ReverterB subReverter = (reverter.subReverter());
            _vm.assumeNoRevert(abi.encodeWithSelector(Reverter.MyRevert.selector), address(reverter));
            subReverter.revertIf2(x);
        }
 
        /// @dev Test that `assumeNoRevert` correctly rejects one of two different error selectors when supplying a specific reverter
        function testMultipleAssumes_OneWrong_fails(uint256 x) public view {
            _vm.assumeNoRevert(abi.encodeWithSelector(Reverter.MyRevert.selector), address(reverter));
            _vm.assumeNoRevert(abi.encodeWithSelector(Reverter.RevertWithData.selector, 4), address(reverter));
            reverter.twoPossibleReverts(x);
        }

    
        /// @dev Test that `assumeNoRevert` assumptions are cleared after the first non-cheatcode external call
        function testMultipleAssumesClearAfterCall_fails(uint256 x) public view {
            _vm.assumeNoRevert(Reverter.MyRevert.selector);
            _vm.assumeNoPartialRevert(Reverter.RevertWithData.selector, address(reverter));
            reverter.twoPossibleReverts(x);
    
            reverter.twoPossibleReverts(2);
        }
 
        // /// @dev Test that `assumeNoRevert` correctly rejects any error selector when no selector is provided
        // function testMultipleAssumes_ThrowOnGenericNoRevert_fails(bytes4 selector) public view {
        //     _vm.assumeNoRevert();
        //     _vm.assumeNoRevert(selector);
        //     reverter.twoPossibleReverts(2);
        // }
    
        /// @dev Test that `assumeNoRevert` correctly rejects a generic assumeNoRevert call after any specific reason is provided
        function testMultipleAssumes_ThrowOnGenericNoRevert_AfterSpecific_fails(bytes4 selector) public view {
            _vm.assumeNoRevert(selector);
            _vm.assumeNoRevert();
            reverter.twoPossibleReverts(2);
        }
    
        /// @dev Test that calling `expectRevert` after `assumeNoRevert` results in an error
        function testAssumeThenExpect_fails(uint256) public {
            _vm.assumeNoRevert(Reverter.MyRevert.selector);
            _vm.expectRevert();
            reverter.revertIf2(1);
        }
}
    
"#,
    )
    .unwrap();

    fn match_test<'a>(cmd: &'a mut TestCommand, name: &str) -> &'a mut TestCommand {
        cmd.forge_fuse().args(["test", "--mt", name])
    }

    fn assert_failure_contains(cmd: &mut TestCommand, test_name: &str, expected_message: &str) {
        let output = String::from_utf8(
            match_test(cmd, test_name).assert_failure().get_output().stdout.clone(),
        )
        .unwrap();
        assert!(
            output.contains(expected_message),
            "expected stdout for {test_name} to contain '{expected_message}'; got '{output}'",
        );
    }

    match_test(&mut cmd, "testAssumeSelector").assert_success();
    match_test(&mut cmd, "testAssumeWithDataSingle").assert_success();
    match_test(&mut cmd, "testAssumeWithDataPartial").assert_success();
    match_test(&mut cmd, "testMultipleAssumesPasses").assert_success();
    match_test(&mut cmd, "testMultipleAssumes_Partial").assert_success();
    match_test(&mut cmd, "testAssumeNotClearedAfterCheatcodeCall").assert_success();
    match_test(&mut cmd, "testExpectThenAssumeFails");
    assert_failure_contains(
        &mut cmd,
        "testAssume_wrongSelector_fails",
        "FAIL: MyRevert(); counterexample:",
    );
    assert_failure_contains(
        &mut cmd,
        "testAssume_wrongData_fails",
        "FAIL: RevertWithData(2); counterexample:",
    );
    assert_failure_contains(
        &mut cmd,
        "testAssumeWithReverter_fails",
        "FAIL: MyRevert(); counterexample:",
    );
    assert_failure_contains(
        &mut cmd,
        "testMultipleAssumes_OneWrong_fails",
        "FAIL: RevertWithData(3); counterexample:",
    );
    assert_failure_contains(
        &mut cmd,
        "testMultipleAssumesClearAfterCall_fails",
        "FAIL: MyRevert(); counterexample:",
    );
    // need a better way to handle cheatcodes reverting; currently their messages get abi-encoded
    // and treated like normal evm data, which makes them hard (and inefficient) to match in the
    // handler
    // assert_failure_contains(
    //     &mut cmd,
    //     "testMultipleAssumes_ThrowOnGenericNoRevert_fails",
    //     "FAIL: vm.assumeNoRevert: cannot combine a generic assumeNoRevert with specific
    // assumeNoRevert reasons;", );
    assert_failure_contains(
        &mut cmd,
        "testMultipleAssumes_ThrowOnGenericNoRevert_AfterSpecific_fails",
        "FAIL: vm.assumeNoRevert: cannot combine a generic assumeNoRevert with specific assumeNoRevert reasons;",
    );
    assert_failure_contains(
        &mut cmd,
        "testAssumeThenExpect_fails",
        "FAIL: vm.expectRevert: cannot expect a revert when using assumeNoRevert;",
    );
});
