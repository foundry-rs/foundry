//! Contains various tests for checking `forge test`
use foundry_config::Config;
use foundry_test_utils::{
    forgetest, forgetest_init,
    util::{template_lock, OutputExt, TestCommand, TestProject},
};
use foundry_utils::rpc;
use std::{path::PathBuf, process::Command, str::FromStr};

// tests that test filters are handled correctly
forgetest!(can_set_filter_values, |prj: TestProject, mut cmd: TestCommand| {
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
});

// tests that warning is displayed when there are no tests in project
forgetest!(warn_no_tests, |prj: TestProject, mut cmd: TestCommand| {
    prj.inner()
        .add_source(
            "dummy",
            r"
// SPDX-License-Identifier: UNLICENSED
pragma solidity =0.8.13;

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
forgetest!(warn_no_tests_match, |prj: TestProject, mut cmd: TestCommand| {
    prj.inner()
        .add_source(
            "dummy",
            r"
// SPDX-License-Identifier: UNLICENSED
pragma solidity =0.8.13;

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
forgetest!(suggest_when_no_tests_match, |prj: TestProject, mut cmd: TestCommand| {
    // set up project
    prj.inner()
        .add_source(
            "TestE.t.sol",
            r"
// SPDX-License-Identifier: UNLICENSED
pragma solidity 0.8.10;

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
forgetest!(can_fuzz_array_params, |prj: TestProject, mut cmd: TestCommand| {
    prj.insert_ds_test();

    prj.inner()
        .add_source(
            "ATest.t.sol",
            r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity 0.8.10;
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
    cmd.stdout_lossy().contains("[PASS]")
});

// tests that `bytecode_hash` will be sanitized
forgetest!(can_test_pre_bytecode_hash, |prj: TestProject, mut cmd: TestCommand| {
    prj.insert_ds_test();

    prj.inner()
        .add_source(
            "ATest.t.sol",
            r#"
// SPDX-License-Identifier: UNLICENSED
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
    cmd.stdout_lossy().contains("[PASS]")
});

// tests that using the --match-path option only runs files matching the path
forgetest!(can_test_with_match_path, |prj: TestProject, mut cmd: TestCommand| {
    prj.insert_ds_test();

    prj.inner()
        .add_source(
            "ATest.t.sol",
            r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity 0.8.10;
import "./test.sol";
contract ATest is DSTest {
    function testArray(uint64[2] calldata values) external {
        assertTrue(true);
    }
}
   "#,
        )
        .unwrap();

    prj.inner()
        .add_source(
            "FailTest.t.sol",
            r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity 0.8.10;
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
    cmd.stdout_lossy().contains("[PASS]") && !cmd.stdout_lossy().contains("[FAIL]")
});

// tests that `forge test` will pick up tests that are stored in the `test = <path>` config value
forgetest!(can_run_test_in_custom_test_folder, |prj: TestProject, mut cmd: TestCommand| {
    prj.insert_ds_test();

    // explicitly set the test folder
    let config = Config { test: "nested/forge-tests".into(), ..Default::default() };
    prj.write_config(config);
    let config = cmd.config();
    assert_eq!(config.test, PathBuf::from("nested/forge-tests"));

    prj.inner()
        .add_source(
            "nested/forge-tests/MyTest.t.sol",
            r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity 0.8.10;
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
forgetest_init!(can_test_repeatedly, |_prj: TestProject, mut cmd: TestCommand| {
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
forgetest!(
    runs_tests_exactly_once_with_changed_versions,
    |prj: TestProject, mut cmd: TestCommand| {
        prj.insert_ds_test();

        prj.inner()
            .add_source(
                "Contract.t.sol",
                r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity >=0.8.10;
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
        let config = Config { solc: Some("0.8.10".into()), ..Default::default() };
        prj.write_config(config);

        cmd.arg("test");
        cmd.unchecked_output()
            .stdout_matches_path(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
                "tests/fixtures/runs_tests_exactly_once_with_changed_versions.0.8.10.stdout",
            ));

        // pin version
        let config = Config { solc: Some("0.8.13".into()), ..Default::default() };
        prj.write_config(config);

        cmd.unchecked_output()
            .stdout_matches_path(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
                "tests/fixtures/runs_tests_exactly_once_with_changed_versions.0.8.13.stdout",
            ));
    }
);

// checks that we can test forge std successfully
// `forgetest_init!` will install with `forge-std` under `lib/forge-std`
forgetest_init!(
    #[serial_test::serial]
    can_test_forge_std,
    |prj: TestProject, mut cmd: TestCommand| {
        let mut lock = template_lock();
        let write = lock.write().unwrap();
        let forge_std_dir = prj.root().join("lib/forge-std");
        let status = Command::new("git")
            .current_dir(&forge_std_dir)
            .args(["pull", "origin", "master"])
            .status()
            .unwrap();
        if !status.success() {
            panic!("failed to update forge-std");
        }
        drop(write);

        // execute in subdir
        cmd.cmd().current_dir(forge_std_dir);
        cmd.args(["test", "--root", "."]);
        let stdout = cmd.stdout_lossy();
        assert!(stdout.contains("[PASS]"), "No tests passed:\n{stdout}");
        assert!(!stdout.contains("[FAIL]"), "Tests failed:\n{stdout}");
    }
);

// tests that libraries are handled correctly in multiforking mode
forgetest_init!(can_use_libs_in_multi_fork, |prj: TestProject, mut cmd: TestCommand| {
    prj.wipe_contracts();
    prj.inner()
        .add_source(
            "Contract.sol",
            r"
// SPDX-License-Identifier: UNLICENSED
pragma solidity =0.8.13;

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

    prj.inner()
        .add_test(
            "Contract.t.sol",
            r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity =0.8.13;

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
// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.17;

import "forge-std/Test.sol";

contract FailingTest is Test {
    function testShouldFail() public {
        assertTrue(false);
    }
}
"#;

forgetest_init!(exit_code_error_on_fail_fast, |prj: TestProject, mut cmd: TestCommand| {
    prj.wipe_contracts();
    prj.inner().add_source("failing_test", FAILING_TEST).unwrap();

    // set up command
    cmd.args(["test", "--fail-fast"]);

    // run command and assert error exit code
    cmd.assert_err();
});

forgetest_init!(
    exit_code_error_on_fail_fast_with_json,
    |prj: TestProject, mut cmd: TestCommand| {
        prj.wipe_contracts();

        prj.inner().add_source("failing_test", FAILING_TEST).unwrap();
        // set up command
        cmd.args(["test", "--fail-fast", "--json"]);

        // run command and assert error exit code
        cmd.assert_err();
    }
);
