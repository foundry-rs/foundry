//! svm sanity checks

use foundry_cli_test_utils::{forgetest_init, TestCommand, TestProject};
use semver::Version;
use svm::{self, Platform};

/// The latest solc release
///
/// solc to foundry release process:
///     1. new solc release
///     2. svm updated with all build info
///     3. svm bumped in ethers-rs
///     4. ethers bumped in foundry + update the `LATEST_SOLC`
const LATEST_SOLC: Version = Version::new(0, 8, 18);

macro_rules! ensure_svm_releases {
    ($($test:ident => $platform:ident),*) => {
        $(
        #[tokio::test]
        async fn $test() {
            ensure_latest_release(Platform::$platform).await
        }
        )*
    };
}

async fn ensure_latest_release(platform: Platform) {
    let releases = svm::all_releases(platform)
        .await
        .unwrap_or_else(|err| panic!("Could not fetch releases for {platform}: {err:?}"));
    assert!(
        releases.releases.contains_key(&LATEST_SOLC),
        "platform {platform:?} is missing solc info {LATEST_SOLC}"
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
forgetest_init!(can_test_with_latest_solc, |prj: TestProject, mut cmd: TestCommand| {
    prj.inner()
        .add_test(
            "Counter",
            r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity =<VERSION>;

import "forge-std/Test.sol";

contract CounterTest is Test {

    function testAssert() public {
       assert(true);
    }
}
   "#
            .replace("<VERSION>", &LATEST_SOLC.to_string()),
        )
        .unwrap();

    cmd.args(["test"]);
    cmd.stdout().contains("[PASS]")
});
