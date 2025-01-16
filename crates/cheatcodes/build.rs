use std::{env, error::Error};

use vergen::EmitBuilder;

#[allow(clippy::disallowed_macros)]
fn main() -> Result<(), Box<dyn Error>> {
    // Re-run the build script if the build script itself changes or if the
    // environment variables change.
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=TAG_NAME");
    println!("cargo:rerun-if-env-changed=PROFILE");

    EmitBuilder::builder()
        .build_timestamp()
        .git_describe(false, true, None)
        .git_sha(true)
        .emit_and_set()?;

    // Set the short Git SHA of the latest commit.
    let sha = env::var("VERGEN_GIT_SHA")?;

    // Set the version suffix and whether the version is a nightly build.
    // if not on a tag: <BIN> 0.3.0-dev+ba03de0019.debug
    // if on a tag: <BIN> 0.3.0-stable+ba03de0019.release
    let tag_name = env::var("TAG_NAME").unwrap_or_else(|_| String::from("dev"));
    let version_suffix = format!("-{tag_name}");

    // Set formatted version strings
    let pkg_version = env::var("CARGO_PKG_VERSION")?;

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
