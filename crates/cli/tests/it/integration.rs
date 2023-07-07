use foundry_cli_test_utils::{forgetest_external, util::setup_forge_remote};

forgetest_external!(solmate, "transmissions11/solmate");
forgetest_external!(prb_math, "PaulRBerg/prb-math");
forgetest_external!(prb_proxy, "PaulRBerg/prb-proxy");
forgetest_external!(solady, "Vectorized/solady");
forgetest_external!(
    geb,
    "reflexer-labs/geb",
    &["--chain-id", "99", "--sender", "0x00a329c0648769A73afAc7F9381E08FB43dBEA72"]
);
forgetest_external!(stringutils, "Arachnid/solidity-stringutils");
forgetest_external!(lootloose, "gakonst/lootloose");
forgetest_external!(lil_web3, "m1guelpf/lil-web3");

/// clone + build in one step
#[test]
#[ignore]
fn can_checkout_build() {
    let (_prj, _cmd) = setup_forge_remote("transmissions11/solmate");
}

/// Forking tests
mod fork_integration {
    use foundry_cli_test_utils::forgetest_external;

    forgetest_external!(multicall, "makerdao/multicall", &["--block-number", "1"]);
    forgetest_external!(
        drai,
        "mds1/drai",
        13633752,
        &["--chain-id", "99", "--sender", "0x00a329c0648769A73afAc7F9381E08FB43dBEA72"]
    );
    forgetest_external!(gunilev, "hexonaut/guni-lev", 13633752);
    forgetest_external!(convex, "mds1/convex-shutdown-simulation", 14445961);
}
