use foundry_compilers::artifacts::{BytecodeHash, EvmVersion};
use foundry_config::Config;
use foundry_test_utils::{
    forgetest_async,
    rpc::{next_etherscan_api_key, next_http_archive_rpc_endpoint},
    util::OutputExt,
    TestCommand, TestProject,
};

fn test_verify_bytecode(
    prj: TestProject,
    mut cmd: TestCommand,
    addr: &str,
    contract_name: &str,
    config: Config,
    expected_matches: (&str, &str),
) {
    let etherscan_key = next_etherscan_api_key();
    let rpc_url = next_http_archive_rpc_endpoint();

    // fetch and flatten source code
    let source_code = cmd
        .cast_fuse()
        .args(["etherscan-source", addr, "--flatten", "--etherscan-api-key", &etherscan_key])
        .assert_success()
        .get_output()
        .stdout_lossy();

    prj.add_source(contract_name, &source_code).unwrap();
    prj.write_config(config);

    let output = cmd
        .forge_fuse()
        .args([
            "verify-bytecode",
            addr,
            contract_name,
            "--etherscan-api-key",
            &etherscan_key,
            "--rpc-url",
            &rpc_url,
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(output
        .contains(format!("Creation code matched with status {}", expected_matches.0).as_str()));
    assert!(output
        .contains(format!("Runtime code matched with status {}", expected_matches.1).as_str()));
}

forgetest_async!(can_verify_bytecode_no_metadata, |prj, cmd| {
    test_verify_bytecode(
        prj,
        cmd,
        "0xba2492e52F45651B60B8B38d4Ea5E2390C64Ffb1",
        "SystemConfig",
        Config {
            evm_version: EvmVersion::London,
            optimizer_runs: 999999,
            optimizer: true,
            cbor_metadata: false,
            bytecode_hash: BytecodeHash::None,
            ..Default::default()
        },
        ("full", "full"),
    );
});

forgetest_async!(can_verify_bytecode_with_metadata, |prj, cmd| {
    test_verify_bytecode(
        prj,
        cmd,
        "0xb8901acb165ed027e32754e0ffe830802919727f",
        "L1_ETH_Bridge",
        Config {
            evm_version: EvmVersion::Paris,
            optimizer_runs: 50000,
            optimizer: true,
            ..Default::default()
        },
        ("partial", "partial"),
    );
});
