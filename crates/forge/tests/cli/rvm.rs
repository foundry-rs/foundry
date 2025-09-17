//! rvm sanity checks

use lazy_static::lazy_static;
use rvm::Binary;
use semver::Version;

const LATEST_SOLC: Version = Version::new(0, 8, 29);
lazy_static! {
    static ref LATEST_RESOLC: Version = Version::parse("0.1.0-dev.13").unwrap();
}

#[test]
fn ensure_latest_resolc() {
    let releases = rvm::VersionManager::new(false)
        .unwrap()
        .list_available(Some(LATEST_SOLC))
        .unwrap_or_else(|err| panic!("Could not fetch releases: {err:?}"));
    let found = releases.iter().any(|release| matches!(release, Binary::Remote(resolc) | Binary::Local{path: _, info: resolc} if resolc.version == *LATEST_RESOLC));
    assert!(found, "Expected resolc version: {} not found in releases", *LATEST_RESOLC);
}
