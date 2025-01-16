// The version of the Foundry CLI.
pub const VERSION_MESSAGE: &str = env!("FOUNDRY_SHORT_VERSION");

// Whether the version is a nightly build.
pub const IS_NIGHTLY_VERSION: &str = env!("FOUNDRY_IS_NIGHTLY_VERSION");

// The warning message for nightly versions.
pub const NIGHTLY_VERSION_WARNING_MESSAGE: &str =
    "This is a nightly build of Foundry. It is recommended to use the latest stable version. \
    Visit https://book.getfoundry.sh/announcements for more information. \n\
    To mute this warning set `FOUNDRY_DISABLE_NIGHTLY_WARNING` in your environment. \n";
