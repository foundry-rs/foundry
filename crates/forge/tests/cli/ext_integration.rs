use foundry_test_utils::util::ExtTester;

// Actively maintained tests
// Last updated: June 19th 2025

// <https://github.com/foundry-rs/forge-std>
#[test]
fn forge_std() {
    ExtTester::new("foundry-rs", "forge-std", "b69e66b0ff79924d487d49bf7fb47c9ec326acba")
        // Skip fork tests.
        .args(["--nmc", "Fork"])
        .verbosity(2)
        .run();
}

// <https://github.com/PaulRBerg/prb-math>
#[test]
#[cfg_attr(windows, ignore = "Windows cannot find installed programs")]
fn prb_math() {
    ExtTester::new("PaulRBerg", "prb-math", "aad73cfc6cdc2c9b660199b5b1e9db391ea48640")
        .install_command(&["bun", "install", "--prefer-offline"])
        // Try npm if bun fails / is not installed.
        .install_command(&["npm", "install", "--prefer-offline"])
        .run();
}

// <https://github.com/PaulRBerg/prb-proxy>
#[test]
#[cfg_attr(windows, ignore = "Windows cannot find installed programs")]
fn prb_proxy() {
    ExtTester::new("PaulRBerg", "prb-proxy", "e45f5325d4b6003227a6c4bdaefac9453f89de2e")
        .install_command(&["bun", "install", "--prefer-offline"])
        // Try npm if bun fails / is not installed.
        .install_command(&["npm", "install", "--prefer-offline"])
        .run();
}

// <https://github.com/sablier-labs/v2-core>
#[test]
#[cfg_attr(windows, ignore = "Windows cannot find installed programs")]
fn sablier_v2_core() {
    let mut tester =
        ExtTester::new("sablier-labs", "v2-core", "d85521f5615f6c19612ff250ee89c57b9afa6aa2")
            // Skip fork tests.
            .args(["--nmc", "Fork"])
            // Increase the gas limit: https://github.com/sablier-labs/v2-core/issues/956
            .args(["--gas-limit", &u64::MAX.to_string()])
            // Run tests without optimizations.
            .env("FOUNDRY_PROFILE", "lite")
            .install_command(&["bun", "install", "--prefer-offline"])
            // Try npm if bun fails / is not installed.
            .install_command(&["npm", "install", "--prefer-offline"])
            .verbosity(2);

    // This test reverts due to memory limit without isolation. This revert is not reached with
    // isolation because memory is divided between separate EVMs created by inner calls.
    if cfg!(feature = "isolate-by-default") {
        tester = tester.args(["--nmt", "test_RevertWhen_LoopCalculationOverflowsBlockGasLimit"]);
    }

    tester.run();
}

// <https://github.com/Vectorized/solady>
#[test]
fn solady() {
    ExtTester::new("Vectorized", "solady", "cbcfe0009477aa329574f17e8db0a05703bb8bdd").run();
}

// <https://github.com/pcaversaccio/snekmate>
#[test]
#[cfg_attr(windows, ignore = "Windows cannot find installed programs")]
#[cfg(not(feature = "isolate-by-default"))]
fn snekmate() {
    ExtTester::new("pcaversaccio", "snekmate", "601031d244475b160a00f73053532528bf665cc3")
        .install_command(&["pnpm", "install", "--prefer-offline"])
        // Try npm if pnpm fails / is not installed.
        .install_command(&["npm", "install", "--prefer-offline"])
        .run();
}

// <https://github.com/mds1/multicall>
#[test]
fn mds1_multicall3() {
    ExtTester::new("mds1", "multicall", "5f90062160aedb7c807fadca469ac783a0557b57").run();
}

// Legacy tests

// <https://github.com/Arachnid/solidity-stringutils>
#[test]
fn solidity_stringutils() {
    ExtTester::new("Arachnid", "solidity-stringutils", "4b2fcc43fa0426e19ce88b1f1ec16f5903a2e461")
        .run();
}

// <https://github.com/m1guelpf/lil-web3>
#[test]
fn lil_web3() {
    ExtTester::new("m1guelpf", "lil-web3", "7346bd28c2586da3b07102d5290175a276949b15").run();
}

// <https://github.com/makerdao/multicall>
#[test]
fn makerdao_multicall() {
    ExtTester::new("makerdao", "multicall", "103a8a28e4e372d582d6539b30031bda4cd48e21").run();
}

// Legacy forking tests

// <https://github.com/hexonaut/guni-lev>
#[test]
fn gunilev() {
    ExtTester::new("hexonaut", "guni-lev", "15ee8b4c2d28e553c5cd5ba9a2a274af97563bc4")
        .fork_block(13633752)
        .run();
}

// <https://github.com/mds1/convex-shutdown-simulation>
#[test]
fn convex_shutdown_simulation() {
    ExtTester::new(
        "mds1",
        "convex-shutdown-simulation",
        "2537cdebce4396753225c5e616c8e00547d2fcea",
    )
    .fork_block(14445961)
    .run();
}
