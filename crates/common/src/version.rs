use std::{env, error::Error};

pub const NIGHTLY_VERSION_WARNING_MESSAGE: &str =
    "This is a nightly build of Foundry. It is recommended to use the latest stable version. \
    Visit https://book.getfoundry.sh/announcements for more information. \n\
    To mute this warning set `FOUNDRY_DISABLE_NIGHTLY_WARNING` in your environment. \n";

#[allow(clippy::disallowed_macros)]
/// Set the build version information for Foundry binaries.
pub fn set_build_version() -> Result<(), Box<dyn Error>> {
    // Set the short Git SHA of the latest commit.
    let sha = env::var("VERGEN_GIT_SHA")?;

    // Set the version suffix and whether the version is a nightly build.
    // if not on a tag: <BIN> 0.3.0-dev+ba03de0019.debug
    // if on a tag: <BIN> 0.3.0-stable+ba03de0019.release
    let tag_name = option_env!("TAG_NAME");
    let (is_nightly, version_suffix) = match tag_name {
        Some(tag_name) if tag_name.eq("nightly") => (true, "-nightly".to_string()),
        Some(tag_name) => (false, format!("-{tag_name}")),
        None => (false, "-dev".to_string()),
    };

    // Whether the version is a nightly build.
    println!("cargo:rustc-env=FOUNDRY_IS_NIGHTLY_VERSION={is_nightly}");

    // Set formatted version strings
    let pkg_version = env!("CARGO_PKG_VERSION");

    // Append the profile to the version string if it exists.
    let profile_suffix = env::var("PROFILE").map_or(String::new(), |profile| format!(".{profile}"));

    // The short version information for Foundry.
    // - The latest version from Cargo.toml
    // - The short SHA of the latest commit.
    // Example: 0.3.0-dev+ba03de0019.debug
    println!(
        "cargo:rustc-env=FOUNDRY_SHORT_VERSION={pkg_version}{version_suffix}+{sha}{profile_suffix}"
    );

    Ok(())
}
