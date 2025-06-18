//! Foundry version information.

/// The SemVer compatible version information for Foundry.
pub const SEMVER_VERSION: &str = env!("FOUNDRY_SEMVER_VERSION");

/// The short version message information for the Foundry CLI.
pub const SHORT_VERSION: &str = env!("FOUNDRY_SHORT_VERSION");

/// The long version message information for the Foundry CLI.
pub const LONG_VERSION: &str = concat!(
    env!("FOUNDRY_LONG_VERSION_0"),
    "\n",
    env!("FOUNDRY_LONG_VERSION_1"),
    "\n",
    env!("FOUNDRY_LONG_VERSION_2"),
    "\n",
    env!("FOUNDRY_LONG_VERSION_3"),
);

/// Whether the version is a nightly build.
pub const IS_NIGHTLY_VERSION: bool = option_env!("FOUNDRY_IS_NIGHTLY_VERSION").is_some();

/// The warning message for nightly versions.
pub const NIGHTLY_VERSION_WARNING_MESSAGE: &str = "This is a nightly build of Foundry. It is recommended to use the latest stable version. \
    To mute this warning set `FOUNDRY_DISABLE_NIGHTLY_WARNING` in your environment. \n";
