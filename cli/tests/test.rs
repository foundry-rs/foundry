//! Contains various tests for checking `forge test`
use ansi_term::Colour;
use ethers::solc::{artifacts::Metadata, ConfigurableContractArtifact};
use forge::executor::opts::EvmOpts;
use foundry_cli_test_utils::{
    ethers_solc::{remappings::Remapping, PathStyle},
    forgetest, forgetest_ignore, forgetest_init, pretty_eq,
    util::{pretty_err, read_string, TestCommand, TestProject},
};
use foundry_config::{
    parse_with_profile, BasicConfig, Config, OptimizerDetails, SolidityErrorCode,
};
use pretty_assertions::assert_eq;
use std::{env, fs, str::FromStr};

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

    cmd.print_output();

//     assert!(cmd.stdout_lossy().ends_with(
//         "
// Compiler run successful
// "
//     ));
});