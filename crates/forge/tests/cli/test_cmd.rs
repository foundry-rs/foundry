//! Contains various tests for `forge test`.

use alloy_primitives::U256;
use foundry_config::{Config, FuzzConfig};
use foundry_test_utils::{
    rpc,
    util::{OutputExt, OTHER_SOLC_VERSION, SOLC_VERSION},
};
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
    cmd.unchecked_output().stdout_matches_path(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/warn_no_tests.stdout"),
    );
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
    cmd.unchecked_output().stdout_matches_path(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/warn_no_tests_match.stdout"),
    );
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
    cmd.unchecked_output().stdout_matches_path(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/suggest_when_no_tests_match.stdout"),
    );
});

// tests that direct import paths are handled correctly
forgetest!(can_fuzz_array_params, |prj, cmd| {
    prj.insert_ds_test();

    prj.add_source(
        "ATest.t.sol",
        r#"
import "./test.sol";
contract ATest is DSTest {
    function testArray(uint64[2] calldata values) external {
        assertTrue(true);
    }
}
   "#,
    )
    .unwrap();

    cmd.arg("test");
    cmd.stdout_lossy().contains("[PASS]");
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
    function testArray(uint64[2] calldata values) external {
        assertTrue(true);
    }
}
   "#,
    )
    .unwrap();

    cmd.arg("test");
    cmd.stdout_lossy().contains("[PASS]");
});

// tests that using the --match-path option only runs files matching the path
forgetest!(can_test_with_match_path, |prj, cmd| {
    prj.insert_ds_test();

    prj.add_source(
        "ATest.t.sol",
        r#"
import "./test.sol";
contract ATest is DSTest {
    function testArray(uint64[2] calldata values) external {
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

    cmd.args(["test", "--match-path", "*src/ATest.t.sol"]);
    assert!(cmd.stdout_lossy().contains("[PASS]") && !cmd.stdout_lossy().contains("[FAIL]"));
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

    cmd.arg("test");
    cmd.unchecked_output().stdout_matches_path(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/can_run_test_in_custom_test_folder.stdout"),
    );
});

// checks that forge test repeatedly produces the same output
#[cfg(not(feature = "isolate-by-default"))]
forgetest_init!(can_test_repeatedly, |_prj, cmd| {
    cmd.arg("test");
    cmd.assert_non_empty_stdout();

    for _ in 0..5 {
        cmd.unchecked_output().stdout_matches_path(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/can_test_repeatedly.stdout"),
        );
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

    cmd.arg("test");
    cmd.unchecked_output().stdout_matches_path(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/runs_tests_exactly_once_with_changed_versions.1.stdout"),
    );

    // pin version
    let config = Config { solc: Some(OTHER_SOLC_VERSION.into()), ..Default::default() };
    prj.write_config(config);

    cmd.unchecked_output().stdout_matches_path(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/runs_tests_exactly_once_with_changed_versions.2.stdout"),
    );
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

    cmd.arg("test");
    cmd.unchecked_output().stdout_matches_path(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/can_use_libs_in_multi_fork.stdout"),
    );
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
    cmd.assert_err();
});

forgetest_init!(exit_code_error_on_fail_fast_with_json, |prj, cmd| {
    prj.wipe_contracts();

    prj.add_source("failing_test", FAILING_TEST).unwrap();
    // set up command
    cmd.args(["test", "--fail-fast", "--json"]);

    // run command and assert error exit code
    cmd.assert_err();
});

// <https://github.com/foundry-rs/foundry/issues/6531>
forgetest_init!(repro_6531, |prj, cmd| {
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

    let expected = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/repro_6531.stdout"),
    )
    .unwrap()
    .replace("<url>", &endpoint);

    cmd.args(["test", "-vvvv"]).unchecked_output().stdout_matches_content(&expected);
});

// <https://github.com/foundry-rs/foundry/issues/6579>
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

    cmd.args(["test", "-vvvv"]).unchecked_output().stdout_matches_path(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/include_custom_types_in_traces.stdout"),
    );
});

forgetest_init!(can_test_selfdestruct_with_isolation, |prj, cmd| {
    prj.wipe_contracts();

    prj.add_test(
        "Contract.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract Destructing {
    function destruct() public {
        selfdestruct(payable(address(0)));
    }
}

contract SelfDestructTest is Test {
    function test() public {
        Destructing d = new Destructing();
        vm.store(address(d), bytes32(0), bytes32(uint256(1)));
        d.destruct();
        assertEq(address(d).code.length, 0);
        assertEq(vm.load(address(d), bytes32(0)), bytes32(0));
    }
}
   "#,
    )
    .unwrap();

    cmd.args(["test", "-vvvv", "--isolate"]).assert_success();
});

forgetest_init!(can_test_transient_storage_with_isolation, |prj, cmd| {
    prj.wipe_contracts();

    prj.add_test(
        "Contract.t.sol",
        r#"pragma solidity 0.8.24;
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
        &r#"pragma solidity 0.8.24;
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
    cmd.assert_success()
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
        r#"pragma solidity 0.8.24;
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

    cmd.args(["test"]);
    let (stderr, _) = cmd.unchecked_output_lossy();
    // make sure there are only 61 runs (with proptest shrinking same test results in 298 runs)
    assert_eq!(extract_number_of_runs(stderr), 61);
});

forgetest_init!(should_exit_early_on_invariant_failure, |prj, cmd| {
    prj.wipe_contracts();
    prj.add_test(
        "CounterInvariant.t.sol",
        r#"pragma solidity 0.8.24;
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

    cmd.args(["test"]);
    let (stderr, _) = cmd.unchecked_output_lossy();
    // make sure invariant test exit early with 0 runs
    assert_eq!(extract_number_of_runs(stderr), 0);
});

fn extract_number_of_runs(stderr: String) -> usize {
    let runs = stderr.find("runs:").and_then(|start_runs| {
        let runs_split = &stderr[start_runs + 6..];
        runs_split.find(',').map(|end_runs| &runs_split[..end_runs])
    });
    runs.unwrap().parse::<usize>().unwrap()
}

forgetest_init!(should_replay_failures_only, |prj, cmd| {
    prj.wipe_contracts();
    prj.add_test(
        "ReplayFailures.t.sol",
        r#"pragma solidity 0.8.24;
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

    cmd.args(["test"]);
    cmd.assert_err();
    // Test failure filter should be persisted.
    assert!(prj.root().join("cache/test-failures").exists());

    // Perform only the 2 failing tests from last run.
    cmd.forge_fuse();
    cmd.args(["test", "--rerun"]).unchecked_output().stdout_matches_path(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/replay_last_run_failures.stdout"),
    );
});

// <https://github.com/foundry-rs/foundry/issues/7530>
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

    let output = cmd.args(["test", "-vvvv"]).stdout_lossy();
    assert!(output.contains("VM: [0x7109709ECfa91a80626fF3989D68f67F5b1DD12D]"));
    assert!(output.contains("console: [0x000000000000000000636F6e736F6c652e6c6f67]"));
    assert!(output.contains("Create2Deployer: [0x4e59b44847b379578588920cA78FbF26c0B4956C]"));
    assert!(output.contains("DefaultSender: [0x1804c8AB1F12E6bbf3894d4083f33e07309d1f38]"));
    assert!(output.contains("DefaultTestContract: [0xb4c79daB8f259C7Aee6E5b2Aa729821864227e84]"));
    assert!(output.contains("ECRecover: [0x0000000000000000000000000000000000000001]"));
    assert!(output.contains("SHA-256: [0x0000000000000000000000000000000000000002]"));
    assert!(output.contains("RIPEMD-160: [0x0000000000000000000000000000000000000003]"));
    assert!(output.contains("Identity: [0x0000000000000000000000000000000000000004]"));
    assert!(output.contains("ModExp: [0x0000000000000000000000000000000000000005]"));
    assert!(output.contains("ECAdd: [0x0000000000000000000000000000000000000006]"));
    assert!(output.contains("ECMul: [0x0000000000000000000000000000000000000007]"));
    assert!(output.contains("ECPairing: [0x0000000000000000000000000000000000000008]"));
    assert!(output.contains("Blake2F: [0x0000000000000000000000000000000000000009]"));
    assert!(output.contains("PointEvaluation: [0x000000000000000000000000000000000000000A]"));
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
        r#"pragma solidity 0.8.24;
        import {Test, console2} from "forge-std/Test.sol";
    contract ContractFuzz is Test {
      function testFuzzConsoleLog(uint256 x) public pure {
        console2.log("inside fuzz test, x is:", x);
      }
    }
     "#,
    )
    .unwrap();
    cmd.args(["test", "-vv"]);
    let stdout = cmd.stdout_lossy();
    assert!(stdout.contains("inside fuzz test, x is:"), "\n{stdout}");
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
        r#"pragma solidity 0.8.24;
        import {Test, console2} from "forge-std/Test.sol";
    contract ContractFuzz is Test {

      /// forge-config: default.fuzz.show-logs = true
      function testFuzzConsoleLog(uint256 x) public pure {
        console2.log("inside fuzz test, x is:", x);
      }
    }
     "#,
    )
    .unwrap();
    cmd.args(["test", "-vv"]);
    let stdout = cmd.stdout_lossy();
    assert!(stdout.contains("inside fuzz test, x is:"), "\n{stdout}");
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
        r#"pragma solidity 0.8.24;
        import {Test, console2} from "forge-std/Test.sol";
    contract ContractFuzz is Test {

      function testFuzzConsoleLog(uint256 x) public pure {
        console2.log("inside fuzz test, x is:", x);
      }
    }
     "#,
    )
    .unwrap();
    cmd.args(["test", "-vv"]);
    let stdout = cmd.stdout_lossy();
    assert!(!stdout.contains("inside fuzz test, x is:"), "\n{stdout}");
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
        r#"pragma solidity 0.8.24;
        import {Test, console2} from "forge-std/Test.sol";
    contract ContractFuzz is Test {

      /// forge-config: default.fuzz.show-logs = false
      function testFuzzConsoleLog(uint256 x) public pure {
        console2.log("inside fuzz test, x is:", x);
      }
    }
     "#,
    )
    .unwrap();
    cmd.args(["test", "-vv"]);
    let stdout = cmd.stdout_lossy();
    assert!(!stdout.contains("inside fuzz test, x is:"), "\n{stdout}");
});
