use crate::foundry_test_utils::util::OutputExt;
use foundry_config::Config;
use foundry_test_utils::forgetest;

forgetest!(test_revive_inspect, |prj, cmd| {
    prj.add_source(
        "Contracts.sol",
        r#"
//SPDX-license-identifier: MIT

pragma solidity ^0.8.20;

contract ContractOne {
    int public i;

    constructor() {
        i = 0;
    }

    function foo() public{
        while(i<5){
            i++;
        }
    }
}
    "#,
    )
    .unwrap();

    prj.write_config(Config {
        gas_reports: (vec!["*".to_string()]),
        gas_reports_ignore: (vec![]),
        ..Default::default()
    });

    let solc_bytecode = cmd
        .arg("inspect")
        .arg("ContractOne")
        .arg("bytecode")
        .assert_success()
        .get_output()
        .stdout_lossy();
    cmd.forge_fuse();

    let revive_bytecode = cmd
        .arg("inspect")
        .arg("ContractOne")
        .arg("bytecode")
        .arg("--revive")
        .assert_success()
        .get_output()
        .stdout_lossy();
    cmd.forge_fuse();

    let revive_deployedbytecode = cmd
        .arg("inspect")
        .arg("ContractOne")
        .arg("deployedbytecode")
        .arg("--revive")
        .assert_success()
        .get_output()
        .stdout_lossy();
    cmd.forge_fuse();

    // The solc and revive bytecodes returned by inspect should be different
    assert_ne!(solc_bytecode, revive_bytecode);

    // The deployed bytecode in our case should be the same as the bytecode
    assert_eq!(revive_bytecode, revive_deployedbytecode);

    // The Revive bytecode starts with "PVM"
    assert!(revive_bytecode.starts_with("0x50564d"));

    // Throw an error when trying to inspect the assembly field
    cmd.arg("inspect")
        .arg("ContractOne")
        .arg("legacyAssembly")
        .arg("--revive")
        .assert_failure()
        .stderr_eq("Error: Revive version of inspect does not support this field\n");
});
