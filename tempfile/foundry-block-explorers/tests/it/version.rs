use foundry_block_explorers::{errors::EtherscanError, utils::lookup_compiler_version};
use semver::{BuildMetadata, Prerelease, Version};

#[tokio::test]
async fn can_lookup_compiler_version_build_metadata() {
    let v = Version::new(0, 8, 13);
    let version = lookup_compiler_version(&v).await.unwrap();
    assert_eq!(v.major, version.major);
    assert_eq!(v.minor, version.minor);
    assert_eq!(v.patch, version.patch);
    assert_ne!(version.build, BuildMetadata::EMPTY);
    assert_eq!(version.pre, Prerelease::EMPTY);
}

#[tokio::test]
async fn errors_on_invalid_solc() {
    let v = Version::new(100, 0, 0);
    let err = lookup_compiler_version(&v).await.unwrap_err();
    assert!(matches!(err, EtherscanError::MissingSolcVersion(_)));
}
