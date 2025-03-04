use foundry_test_utils::util::ExtTester;

// Actively maintained tests

// <https://github.com/foundry-rs/forge-std>
#[test]
fn forge_std() {
    ExtTester::new("foundry-rs", "forge-std", "464587138602dd194ed0eb5aab15b4721859d422")
        // Skip fork tests.
        .args(["--nmc", "Fork"])
        .run();
}

// <https://github.com/PaulRBerg/prb-math>
#[test]
#[cfg_attr(windows, ignore = "Windows cannot find installed programs")]
fn prb_math() {
    ExtTester::new("PaulRBerg", "prb-math", "b03f814a03558ed5b62f89a57bcc8d720a393f67")
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
        ExtTester::new("sablier-labs", "v2-core", "43cf7c9d968e61a5a03e9237a71a27165b125414")
            // Skip fork tests.
            .args(["--nmc", "Fork"])
            // Increase the gas limit: https://github.com/sablier-labs/v2-core/issues/956
            .args(["--gas-limit", u64::MAX.to_string().as_str()])
            // Run tests without optimizations.
            .env("FOUNDRY_PROFILE", "lite")
            .install_command(&["bun", "install", "--prefer-offline"])
            // Try npm if bun fails / is not installed.
            .install_command(&["npm", "install", "--prefer-offline"]);

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
    ExtTester::new("Vectorized", "solady", "66162801e022c268a2a0f621ac5eb0df4986f6eb").run();
}

// <https://github.com/pcaversaccio/snekmate>
#[test]
#[cfg_attr(windows, ignore = "Windows cannot find installed programs")]
#[cfg(not(feature = "isolate-by-default"))]
fn snekmate() {
    ExtTester::new("pcaversaccio", "snekmate", "472c31780a15cb77ff5582083ad15151ac5a278b")
        .install_command(&["pnpm", "install", "--prefer-offline"])
        // Try npm if pnpm fails / is not installed.
        .install_command(&["npm", "install", "--prefer-offline"])
        .run();
}

// <https://github.com/mds1/multicall>
#[test]
fn mds1_multicall3() {
    ExtTester::new("mds1", "multicall", "f534fbc9f98386a217eaaf9b29d3d4f6f920d5ec").run();
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
