//! Contains various tests for checking `forge test`
use foundry_cli_test_utils::{
    forgetest,
    util::{OutputExt, TestCommand, TestProject},
};
use foundry_config::Config;
use std::{path::PathBuf, str::FromStr};

// import forge utils as mod
#[allow(unused)]
#[path = "../../src/utils.rs"]
mod forge_utils;

// tests that test filters are handled correctly
forgetest!(can_set_filter_values, |prj: TestProject, mut cmd: TestCommand| {
    cmd.set_current_dir(prj.root());

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
    cmd.stdout().contains("[PASS]")
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
    cmd.stdout().contains("[PASS]")
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
    cmd.stdout().contains("[PASS]") && !cmd.stdout().contains("[FAIL]")
});

// tests that `forge test` will pick up tests that are stored in the `test = <path>` config value
forgetest!(can_run_test_in_custom_test_folder, |prj: TestProject, mut cmd: TestCommand| {
    cmd.set_current_dir(prj.root());
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
