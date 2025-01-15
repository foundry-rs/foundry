use std::{env, error::Error};

pub const NIGHTLY_VERSION_WARNING_MESSAGE: &str =
    "This is a pre-release build of Foundry. It is recommended to use the latest stable version. See: https://book.getfoundry.sh/announcements";

/// Set the build version information for Foundry binaries.
pub fn set_build_version() -> Result<(), Box<dyn Error>> {
    let sha = env::var("VERGEN_GIT_SHA")?;

    // Check if the git repository is dirty, i.e. has uncommitted changes.
    // If so, mark the version as a development version.
    let is_dirty = env::var("VERGEN_GIT_DIRTY")? == "true";

    // Set nightly version information
    // This is used to determine if the build is a nightly build.
    let is_nightly = env::var("IS_NIGHTLY").is_ok();

    println!("cargo:rustc-env=FOUNDRY_IS_NIGHTLY_VERSION={}", is_nightly);

    // > git describe --always --tags
    // if not on a tag: v0.3.0-dev-defa64b2
    // if on a tag: v0.3.0-stable-defa64b2
    let version_suffix = match env::var("TAG_NAME") {
        Ok(tag_name) => format!("-{}", tag_name),
        Err(_) => {
            if is_dirty {
                "-dev".to_string()
            } else {
                "".to_string()
            }
        }
    };

    println!("cargo:rustc-env=FOUNDRY_VERSION_SUFFIX={}", version_suffix);

    // Set formatted version strings
    let pkg_version = env!("CARGO_PKG_VERSION");

    // The short version information for Foundry.
    // - The latest version from Cargo.toml
    // - The short SHA of the latest commit.
    // Example: 0.1.0 (defa64b2)
    println!("cargo:rustc-env=FOUNDRY_SHORT_VERSION={pkg_version}{version_suffix}+{sha}");

    Ok(())
}
