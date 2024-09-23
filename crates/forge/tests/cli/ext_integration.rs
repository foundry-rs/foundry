use foundry_test_utils::util::ExtTester;

#[test]
fn forge_std() {
    ExtTester::new("foundry-rs", "forge-std", "1d0766bc5d814f117c7b1e643828f7d85024fb51")
        // Skip fork tests.
        .args(["--nmc", "Fork"])
        .run();
}

#[test]
fn solmate() {
    let mut tester =
        ExtTester::new("transmissions11", "solmate", "c892309933b25c03d32b1b0d674df7ae292ba925");

    if cfg!(feature = "isolate-by-default") {
        tester = tester.args(["--nmc", "ReentrancyGuardTest"]);
    }

    tester.run();
}

#[test]
#[cfg_attr(windows, ignore = "Windows cannot find installed programs")]
fn prb_math() {
    ExtTester::new("PaulRBerg", "prb-math", "5b6279a0cf7c1b1b6a5cc96082811f7ef620cf60")
        .install_command(&["bun", "install", "--prefer-offline"])
        // Try npm if bun fails / is not installed.
        .install_command(&["npm", "install", "--prefer-offline"])
        .run();
}

#[test]
#[cfg_attr(windows, ignore = "Windows cannot find installed programs")]
fn prb_proxy() {
    ExtTester::new("PaulRBerg", "prb-proxy", "fa13cf09fbf544a2d575b45884b8e94a79a02c06")
        .install_command(&["bun", "install", "--prefer-offline"])
        // Try npm if bun fails / is not installed.
        .install_command(&["npm", "install", "--prefer-offline"])
        .run();
}

#[test]
#[cfg_attr(windows, ignore = "Windows cannot find installed programs")]
fn sablier_v2() {
    let mut tester =
        ExtTester::new("sablier-labs", "v2-core", "84758a40077bf3ccb1c8f7bb8d00278e672fbfef")
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

#[test]
fn solady() {
    ExtTester::new("Vectorized", "solady", "54ea1543a229b88b44ccb6ec5ea570135811a7d9").run();
}

#[test]
#[cfg_attr(windows, ignore = "weird git fail")]
#[cfg(not(feature = "isolate-by-default"))]
fn geb() {
    ExtTester::new("reflexer-labs", "geb", "1a59f16a377386c49f520006ed0f7fd9d128cb09")
        .env("FOUNDRY_LEGACY_ASSERTIONS", "true")
        .args(["--chain-id", "99", "--sender", "0x00a329c0648769A73afAc7F9381E08FB43dBEA72"])
        .run();
}

#[test]
fn stringutils() {
    ExtTester::new("Arachnid", "solidity-stringutils", "4b2fcc43fa0426e19ce88b1f1ec16f5903a2e461")
        .run();
}

#[test]
fn lootloose() {
    ExtTester::new("gakonst", "lootloose", "7b639efe97836155a6a6fc626bf1018d4f8b2495")
        .install_command(&["make", "install"])
        .run();
}

#[test]
fn lil_web3() {
    ExtTester::new("m1guelpf", "lil-web3", "7346bd28c2586da3b07102d5290175a276949b15").run();
}

#[test]
#[cfg_attr(windows, ignore = "Windows cannot find installed programs")]
fn snekmate() {
    ExtTester::new("pcaversaccio", "snekmate", "df226f4a45e86c8f8c3ff1f9fa3443d260002050")
        .install_command(&["pnpm", "install", "--prefer-offline"])
        // Try npm if pnpm fails / is not installed.
        .install_command(&["npm", "install", "--prefer-offline"])
        .run();
}

#[test]
fn makerdao_multicall() {
    ExtTester::new("makerdao", "multicall", "103a8a28e4e372d582d6539b30031bda4cd48e21").run();
}

#[test]
fn mds1_multicall() {
    ExtTester::new("mds1", "multicall", "263ef67f29ab9e450142b42dde617ad69adbf211").run();
}

// Forking tests

#[test]
fn drai() {
    ExtTester::new("mds1", "drai", "f31ce4fb15bbb06c94eefea2a3a43384c75b95cf")
        .args(["--chain-id", "99", "--sender", "0x00a329c0648769A73afAc7F9381E08FB43dBEA72"])
        .env("FOUNDRY_LEGACY_ASSERTIONS", "true")
        .fork_block(13633752)
        .run();
}

#[test]
fn gunilev() {
    ExtTester::new("hexonaut", "guni-lev", "15ee8b4c2d28e553c5cd5ba9a2a274af97563bc4")
        .fork_block(13633752)
        .run();
}

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
