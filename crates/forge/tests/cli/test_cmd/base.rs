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

    cmd.args([
        "test",
        "--network",
        "base",
        "--hardfork",
        "base:Beryl",
        "--chain-id",
        "8453",
        "--match-test",
        "test_beryl",
    ])
    .assert_success();
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
