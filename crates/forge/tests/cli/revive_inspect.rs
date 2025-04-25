use crate::foundry_test_utils::util::OutputExt;
use foundry_config::Config;
use foundry_test_utils::forgetest;

forgetest!(test_resolc_inspect, |prj, cmd| {
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

    let resolc_bytecode = cmd
        .arg("inspect")
        .arg("ContractOne")
        .arg("bytecode")
        .arg("--resolc")
        .assert_success()
        .get_output()
        .stdout_lossy();
    cmd.forge_fuse();

    let resolc_deployedbytecode = cmd
        .arg("inspect")
        .arg("ContractOne")
        .arg("deployedbytecode")
        .arg("--resolc")
        .assert_success()
        .get_output()
        .stdout_lossy();
    cmd.forge_fuse();

    // The solc and resolc bytecodes returned by inspect should be different
    assert_ne!(solc_bytecode, resolc_bytecode);

    // The deployed bytecode in our case should be the same as the bytecode
    assert_eq!(resolc_bytecode, resolc_deployedbytecode);

    // The resolc bytecode starts with "PVM"
    assert!(resolc_bytecode.starts_with("0x50564d"));

    // Throw an error when trying to inspect the assembly field
    cmd.arg("inspect")
        .arg("ContractOne")
        .arg("legacyAssembly")
        .arg("--resolc")
        .assert_failure()
        .stderr_eq("Error: Resolc version of inspect does not support this field\n");
});
