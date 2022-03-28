//! Contains various tests for checking `forge test`
use foundry_cli_test_utils::{
    forgetest,
    util::{TestCommand, TestProject},
};

// import forge utils as mod
#[allow(unused)]
#[path = "../src/utils.rs"]
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
