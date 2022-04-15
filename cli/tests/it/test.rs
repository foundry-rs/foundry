//! Contains various tests for checking `forge test`
use foundry_cli_test_utils::{
    forgetest,
    util::{TestCommand, TestProject},
};

// import forge utils as mod
#[allow(unused)]
#[path = "../../src/utils.rs"]
mod forge_utils;

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
