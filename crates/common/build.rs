use std::{env, error::Error};

use chrono::DateTime;
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
        .git_sha(false)
        .emit_and_set()?;

    // Set the Git SHA of the latest commit.
    let sha = env::var("VERGEN_GIT_SHA")?;
    let sha_short = &sha[..10];

    // Set the version suffix and whether the version is a nightly build.
    // if not on a tag: <BIN> 0.3.0-dev+ba03de0019.1737036656.debug
    // if on a tag: <BIN> 0.3.0-stable+ba03de0019.1737036656.release
    let tag_name = env::var("TAG_NAME").unwrap_or_else(|_| String::from("dev"));
    let (is_nightly, version_suffix) = if tag_name == "nightly" {
        (true, "-nightly".to_string())
    } else {
        (false, format!("-{tag_name}"))
    };

    // Whether the version is a nightly build.
    println!("cargo:rustc-env=FOUNDRY_IS_NIGHTLY_VERSION={is_nightly}");

    // Set formatted version strings
    let pkg_version = env::var("CARGO_PKG_VERSION")?;

    // Append the YYYYMMDD build timestamp to the version string, removing the dashes to make it
    // SemVer compliant.
    let timestamp = env::var("VERGEN_BUILD_TIMESTAMP")?;
    let timestamp_unix = DateTime::parse_from_rfc3339(&timestamp)?.timestamp();

    // Append the profile to the version string, defaulting to "debug".
    let profile = env::var("PROFILE").unwrap_or_else(|_| String::from("debug"));

    // The SemVer compatible version for Foundry.
    // - The latest version from Cargo.toml.
    // - The short SHA of the latest commit.
    // - The UNIX formatted build timestamp.
    // - The build profile.
    // Example: forge 0.3.0-nightly+3cb96bde9b.1737036656.debug
    println!(
        "cargo:rustc-env=FOUNDRY_VERSION_SEMVER={pkg_version}{version_suffix}+{sha_short}.{timestamp_unix}.{profile}"
    );

    // The version message for the Foundry CLI.
    // - The latest version from Cargo.toml.
    // - The long SHA of the latest commit.
    // - The build timestamp in RFC3339 format.
    // - The build profile.
    //
    // Example:
    //
    // ```text
    // <BIN>
    // Version: 0.3.0-dev
    // Commit SHA: 5186142d3bb4d1be7bb4ade548b77c8e2270717e
    // Build Timestamp: 2025-01-16T13:52:28.926928104Z
    // Build Profile: debug
    // ```
    println!("cargo:rustc-env=FOUNDRY_VERSION_MESSAGE_0=Version: {pkg_version}{version_suffix}");
    println!("cargo:rustc-env=FOUNDRY_VERSION_MESSAGE_1=Commit SHA: {sha}");
    println!("cargo:rustc-env=FOUNDRY_VERSION_MESSAGE_2=Build Timestamp: {timestamp}");
    println!("cargo:rustc-env=FOUNDRY_VERSION_MESSAGE_3=Build Profile: {profile}");

    Ok(())
}
