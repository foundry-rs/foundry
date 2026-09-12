use anvil::{NodeConfig, spawn};
use foundry_evm::hardforks::BaseUpgrade;
use foundry_test_utils::util::OutputExt;

forgetest!(base_azul_excludes_beryl_precompiles, |prj, cmd| {
    prj.add_test("BaseEvm.t.sol", include_str!("../../fixtures/BaseEvm.t.sol"));

    cmd.args([
        "test",
        "--network",
        "base",
        "--hardfork",
        "base:Azul",
        "--chain-id",
        "8453",
        "--match-test",
        "test_azul_excludes_beryl_precompiles",
    ])
    .assert_success();
});

forgetest!(base_defaults_to_azul, |prj, cmd| {
    prj.add_test("BaseEvm.t.sol", include_str!("../../fixtures/BaseEvm.t.sol"));

    cmd.args([
        "test",
        "--network",
        "base",
        "--chain-id",
        "8453",
        "--match-test",
        "test_azul_excludes_beryl_precompiles",
    ])
    .assert_success();
});

forgetest!(base_beryl_precompiles_and_nested_evm, |prj, cmd| {
    prj.add_test("BaseEvm.t.sol", include_str!("../../fixtures/BaseEvm.t.sol"));

    let stdout = cmd
        .args([
            "test",
            "--network",
            "base",
            "--hardfork",
            "base:Beryl",
            "--chain-id",
            "8453",
            "--match-test",
            "test_beryl",
            "-vvvv",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert!(stdout.contains("ActivationRegistry"), "{stdout}");
    assert!(stdout.contains("B20Factory"), "{stdout}");
});

forgetest!(base_list_accepts_base_network, |prj, cmd| {
    prj.add_test("BaseEvm.t.sol", include_str!("../../fixtures/BaseEvm.t.sol"));

    cmd.args([
        "test",
        "--network",
        "base",
        "--hardfork",
        "base:Beryl",
        "--chain-id",
        "8453",
        "--list",
    ])
    .assert_success();
});

// Stateful Base precompile calls must work against a forked endpoint, not just locally: read-only
// ActivationRegistry/B20 calls already passed while `activate`/`createB20` reverted.
forgetest_async!(base_fork_allows_stateful_precompile_writes, |prj, cmd| {
    let (_api, handle) =
        spawn(NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Beryl.into()))).await;

    prj.add_test("BaseForkWrites.t.sol", include_str!("../../fixtures/BaseForkWrites.t.sol"));

    cmd.args([
        "test",
        "--network",
        "base",
        "--hardfork",
        "base:Beryl",
        "--fork-url",
        &handle.http_endpoint(),
        "--match-contract",
        "BaseForkWritesTest",
        "-vvvv",
    ])
    .assert_success();
});

forgetest!(base_local_allows_stateful_precompile_writes, |prj, cmd| {
    prj.add_test("BaseForkWrites.t.sol", include_str!("../../fixtures/BaseForkWrites.t.sol"));

    cmd.args([
        "test",
        "--network",
        "base",
        "--hardfork",
        "base:Beryl",
        "--chain-id",
        "8453",
        "--match-contract",
        "BaseForkWritesTest",
        "-vvvv",
    ])
    .assert_success();
});

forgetest!(base_script_uses_native_network, |prj, cmd| {
    let script = prj.add_script(
        "BaseScript.s.sol",
        r#"
interface IActivationRegistry {
    function admin() external view returns (address);
}

contract BaseScript {
    address constant ACTIVATION_REGISTRY = 0x8453000000000000000000000000000000000001;
    address constant MAINNET_BERYL_ADMIN = 0xcE3a3bEE7E72E2A24079f3c0Cb3b97740ED425A9;

    function run() external view {
        require(
            IActivationRegistry(ACTIVATION_REGISTRY).admin() == MAINNET_BERYL_ADMIN,
            "Base EVM not selected"
        );
    }
}
   "#,
    );

    cmd.arg("script")
        .arg(script)
        .args(["--network", "base", "--hardfork", "base:Beryl", "--chain-id", "8453"])
        .assert_success();
});
