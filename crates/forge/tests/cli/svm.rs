//! svm sanity checks

use semver::Version;
use svm::Platform;

/// The latest Solc release.
///
/// Solc to Foundry release process:
/// 1. new solc release
/// 2. svm updated with all build info
/// 3. svm bumped in ethers-rs
/// 4. ethers bumped in foundry + update the `LATEST_SOLC`
const LATEST_SOLC: Version = Version::new(0, 8, 23);

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
    let src = format!(
        r#"
pragma solidity ={LATEST_SOLC};

import "forge-std/Test.sol";

contract CounterTest is Test {{
    function testAssert() public {{
        assert(true);
    }}
}}
    "#
    );
    prj.add_test("Counter", &src).unwrap();
    cmd.arg("test").stdout_lossy().contains("[PASS]");
});
