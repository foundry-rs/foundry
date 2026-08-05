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
