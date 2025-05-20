//! Foundry version information.

/// The SemVer compatible version information for Foundry.
pub const SEMVER_VERSION: &str = env!("FOUNDRY_SEMVER_VERSION");

/// The short version message information for the Foundry CLI.
pub const VERSION: &str = env!("FOUNDRY_SHORT_VERSION");

/// Whether the version is a nightly build.
pub const IS_NIGHTLY_VERSION: bool = option_env!("FOUNDRY_IS_NIGHTLY_VERSION").is_some();

/// The warning message for nightly versions.
pub const NIGHTLY_VERSION_WARNING_MESSAGE: &str =
    "This is a nightly build of Foundry. It is recommended to use the latest stable version. \
    Visit https://book.getfoundry.sh/announcements for more information. \n\
    To mute this warning set `FOUNDRY_DISABLE_NIGHTLY_WARNING` in your environment. \n";
