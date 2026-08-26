//! svm sanity checks

use semver::Version;
use svm::Platform;

/// The latest Solc release.
///
/// Solc to Foundry release process:
/// 1. new solc release
/// 2. svm updated with all build info
/// 3. svm bumped in foundry-compilers
/// 4. foundry-compilers update with any breaking changes
/// 5. upgrade the `LATEST_SOLC`
const LATEST_SOLC: Version = Version::new(0, 8, 36);

macro_rules! ensure_svm_releases {
    ($($test:ident => $platform:ident),* $(,)?) => {$(
        #[tokio::test(flavor = "multi_thread")]
        async fn $test() {
            ensure_latest_release(Platform::$platform).await
        }
    )*};
}

async fn ensure_latest_release(platform: Platform) {
    let releases = svm::all_releases(platform)
        .await
        .unwrap_or_else(|err| panic!("Could not fetch releases for {platform}: {err:?}"));
    assert!(
        releases.releases.contains_key(&LATEST_SOLC),
        "platform {platform:?} is missing solc info for v{LATEST_SOLC}"
    );
}

// ensures all platform have the latest solc release version
ensure_svm_releases!(
    test_svm_releases_linux_amd64 => LinuxAmd64,
    test_svm_releases_linux_aarch64 => LinuxAarch64,
    test_svm_releases_macos_amd64 => MacOsAmd64,
    test_svm_releases_macos_aarch64 => MacOsAarch64,
    test_svm_releases_windows_amd64 => WindowsAmd64
);

// Ensures we can always test with the latest solc build
forgetest_init!(can_test_with_latest_solc, |prj, cmd| {
    prj.initialize_default_contracts();
    prj.add_test(
        "Counter.2.t.sol",
        &format!(
            r#"
pragma solidity ={LATEST_SOLC};

import "forge-std/Test.sol";

contract CounterTest is Test {{
    function testAssert() public {{
        assert(true);
    }}
}}
    "#
        ),
    );

    // we need to remove the pinned solc version for this
    prj.update_config(|c| {
        c.solc.take();
    });

    cmd.arg("test").assert_success().stdout_eq(str![[r#"
...
Ran 1 test for test/Counter.2.t.sol:CounterTest
[PASS] testAssert() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]
...
Ran 2 tests for test/Counter.t.sol:CounterTest
[PASS] testFuzz_SetNumber(uint256) (runs: 256, [AVG_GAS])
[PASS] test_Increment() ([GAS])
Suite result: ok. 2 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 2 test suites [ELAPSED]: 3 tests passed, 0 failed, 0 skipped (3 total tests)

"#]]);
});

forgetest_init!(can_test_with_solc_0_8_36_amsterdam, |prj, cmd| {
    prj.initialize_default_contracts();
    prj.add_test(
        "StateGas.t.sol",
        r#"
pragma solidity =0.8.36;

import "forge-std/Test.sol";

interface VmGas {
    struct Gas {
        uint64 gasLimit;
        uint64 gasTotalUsed;
        uint64 gasMemoryUsed;
        int64 gasRefunded;
        uint64 gasRemaining;
        int64 gasStateUsed;
    }

    function lastFrameGas() external view returns (Gas memory gas);
}

contract StateGasTarget {
    uint256 value;

    function setValue() external {
        value = 1;
    }
}

contract StateGasTest is Test {
    VmGas constant vmGas = VmGas(address(uint160(uint256(keccak256("hevm cheat code")))));

    function testLastFrameGasReportsStateGas() public {
        StateGasTarget target = new StateGasTarget();
        target.setValue();

        VmGas.Gas memory gas = vmGas.lastFrameGas();
        assertGt(gas.gasTotalUsed, 0, "regular gas was not recorded");
        assertTrue(gas.gasStateUsed > 0, "state gas was not recorded");
    }
}
"#,
    );

    // Amsterdam is an experimental EVM version in solc 0.8.36.
    cmd.args(["test", "--use", "0.8.36", "--evm-version", "amsterdam", "--experimental"])
        .assert_success();
});
