/// The SemVer compatible version for Foundry.
pub const VERSION_SEMVER: &str = env!("FOUNDRY_VERSION_SEMVER");

/// The version message for the Foundry CLI.
pub const VERSION_MESSAGE: &str = concat!(
    "\n",
    env!("FOUNDRY_VERSION_MESSAGE_0"),
    "\n",
    env!("FOUNDRY_VERSION_MESSAGE_1"),
    "\n",
    env!("FOUNDRY_VERSION_MESSAGE_2"),
    "\n",
    env!("FOUNDRY_VERSION_MESSAGE_3"),
);

/// Whether the version is a nightly build.
pub const IS_NIGHTLY_VERSION: bool = option_env!("FOUNDRY_IS_NIGHTLY_VERSION").is_some();

/// The warning message for nightly versions.
pub const NIGHTLY_VERSION_WARNING_MESSAGE: &str =
    "This is a nightly build of Foundry. It is recommended to use the latest stable version. \
    Visit https://book.getfoundry.sh/announcements for more information. \n\
    To mute this warning set `FOUNDRY_DISABLE_NIGHTLY_WARNING` in your environment. \n";
