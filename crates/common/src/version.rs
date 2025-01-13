use std::{env, error::Error};

/// Set the build version information for Foundry binaries.
pub fn set_build_version() -> Result<(), Box<dyn Error>> {
    let sha = env::var("VERGEN_GIT_SHA")?;
    let is_dirty = env::var("VERGEN_GIT_DIRTY")? == "true";

    // > git describe --always --tags
    // if not on a tag: v0.3.0-dev-defa64b2
    // if on a tag: v0.3.0-stable-defa64b2
    let version_suffix = if is_dirty {
        "-dev"
    } else if env::var("IS_NIGHTLY").is_ok() {
        "-nightly"
    } else {
        ""
    };
    println!("cargo:rustc-env=FOUNDRY_VERSION_SUFFIX={}", version_suffix);

    // Set formatted version strings
    let pkg_version = env!("CARGO_PKG_VERSION");

    // The short version information for Foundry.
    // - The latest version from Cargo.toml
    // - The short SHA of the latest commit.
    // Example: 0.1.0 (defa64b2)
    println!("cargo:rustc-env=FOUNDRY_SHORT_VERSION={pkg_version}{version_suffix}.{sha}");

    Ok(())
}
