use std::{env, error::Error};

use chrono::DateTime;
use vergen::EmitBuilder;

#[expect(clippy::disallowed_macros)]
fn main() -> Result<(), Box<dyn Error>> {
    // Re-run the build script if the build script itself changes or if the
    // environment variables change.
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=TAG_NAME");
    println!("cargo:rerun-if-env-changed=PROFILE");

    EmitBuilder::builder()
        .build_date()
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
    let tag_name = env::var("TAG_NAME")
        .or_else(|_| env::var("CARGO_TAG_NAME"))
        .unwrap_or_else(|_| String::from("dev"));
    let (is_nightly, version_suffix) = if tag_name.contains("nightly") {
        (true, "-nightly".to_string())
    } else {
        (false, format!("-{tag_name}"))
    };

    // Whether the version is a nightly build.
    if is_nightly {
        println!("cargo:rustc-env=FOUNDRY_IS_NIGHTLY_VERSION=true");
    }

    // Set formatted version strings
    let pkg_version = env::var("CARGO_PKG_VERSION")?;

    // Append the profile to the version string
    let out_dir = env::var("OUT_DIR").unwrap();
    let profile = out_dir.rsplit(std::path::MAIN_SEPARATOR).nth(3).unwrap();

    // Set the build timestamp.
    let build_timestamp = env::var("VERGEN_BUILD_TIMESTAMP")?;
    let build_timestamp_unix = DateTime::parse_from_rfc3339(&build_timestamp)?.timestamp();

    // The SemVer compatible version information for Foundry.
    // - The latest version from Cargo.toml.
    // - The short SHA of the latest commit.
    // - The UNIX formatted build timestamp.
    // - The build profile.
    // Example: forge 0.3.0-nightly+3cb96bde9b.1737036656.debug
    println!(
        "cargo:rustc-env=FOUNDRY_SEMVER_VERSION={pkg_version}{version_suffix}+{sha_short}.{build_timestamp_unix}.{profile}"
    );

    // The short version information for the Foundry CLI.
    // - The latest version from Cargo.toml
    // - The short SHA of the latest commit.
    // Example: 0.3.0-dev (3cb96bde9b)
    println!(
        "cargo:rustc-env=FOUNDRY_SHORT_VERSION={pkg_version}{version_suffix} ({sha_short} {build_timestamp})"
    );

    // The long version information for the Foundry CLI.
    // - The latest version from Cargo.toml.
    // - The long SHA of the latest commit.
    // - The build timestamp in RFC3339 format and UNIX format in seconds.
    // - The build profile.
    //
    // Example:
    //
    // ```text
    // <BIN>
    // Version: 0.3.0-dev
    // Commit SHA: 5186142d3bb4d1be7bb4ade548b77c8e2270717e
    // Build Timestamp: 2025-01-16T15:04:03.522021223Z (1737039843)
    // Build Profile: debug
    // ```
    println!("cargo:rustc-env=FOUNDRY_LONG_VERSION_0=Version: {pkg_version}{version_suffix}");
    println!("cargo:rustc-env=FOUNDRY_LONG_VERSION_1=Commit SHA: {sha}");
    println!(
        "cargo:rustc-env=FOUNDRY_LONG_VERSION_2=Build Timestamp: {build_timestamp} ({build_timestamp_unix})"
    );
    println!("cargo:rustc-env=FOUNDRY_LONG_VERSION_3=Build Profile: {profile}");

    Ok(())
}
