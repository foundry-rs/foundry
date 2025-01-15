use std::{env, error::Error};

pub const NIGHTLY_VERSION_WARNING_MESSAGE: &str =
    "This is a nightly build of Foundry. It is recommended to use the latest stable version. See: https://book.getfoundry.sh/announcements";

/// Set the build version information for Foundry binaries.
pub fn set_build_version() -> Result<(), Box<dyn Error>> {
    let sha = env::var("VERGEN_GIT_SHA")?;

    // Check if the git repository is dirty, i.e. has uncommitted changes.
    // If so, mark the version as a development version.
    let is_dirty = env::var("VERGEN_GIT_DIRTY")? == "true";

    // Set the version suffix and whether the version is a nightly build.
    // if not on a tag: <BIN> 0.3.0-dev+ba03de0019
    // if on a tag: <BIN> 0.3.0-stable+ba03de0019
    let tag_name = option_env!("TAG_NAME");
    
    let (is_nightly, version_suffix) = match tag_name {
        Some(tag_name) if tag_name.eq_ignore_ascii_case("nightly") => {
            (true, "-nightly".to_string())
        }
        Some(tag_name) => (false, format!("-{}", tag_name)),
        None => {
            if is_dirty {
                (false, "-dev".to_string())
            } else {
                (false, "".to_string())
            }
        }
    };

    println!("cargo:rustc-env=FOUNDRY_IS_NIGHTLY_VERSION={}", is_nightly);
    println!("cargo:rustc-env=FOUNDRY_VERSION_SUFFIX={}", version_suffix);

    // Set formatted version strings
    let pkg_version = env!("CARGO_PKG_VERSION");

    // The short version information for Foundry.
    // - The latest version from Cargo.toml
    // - The short SHA of the latest commit.
    // Example: 0.3.0-dev+ba03de0019
    println!("cargo:rustc-env=FOUNDRY_SHORT_VERSION={pkg_version}{version_suffix}+{sha}");

    Ok(())
}
