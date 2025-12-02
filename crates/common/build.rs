#![expect(clippy::disallowed_macros)]

use chrono::DateTime;
use std::{error::Error, path::PathBuf};
use vergen::EmitBuilder;

fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo:rerun-if-changed=build.rs");

    EmitBuilder::builder()
        .build_date()
        .build_timestamp()
        .git_describe(false, true, None)
        .git_sha(false)
        .emit_and_set()?;

    let sha = env_var("VERGEN_GIT_SHA");
    let sha_short = &sha[..10];

    let tag_name = try_env_var("TAG_NAME").unwrap_or_else(|| String::from("dev"));
    let is_nightly = tag_name.contains("nightly");
    let version_suffix = if is_nightly { "nightly" } else { &tag_name };

    if is_nightly {
        println!("cargo:rustc-env=FOUNDRY_IS_NIGHTLY_VERSION=true");
    }

    let pkg_version = env_var("CARGO_PKG_VERSION");
    let version = format!("{pkg_version}-{version_suffix}");

    // `PROFILE` captures only release or debug. Get the actual name from the out directory.
    let out_dir = PathBuf::from(env_var("OUT_DIR"));
    let profile = out_dir.components().rev().nth(3).unwrap().as_os_str().to_str().unwrap();

    let build_timestamp = env_var("VERGEN_BUILD_TIMESTAMP");
    let build_timestamp_unix = DateTime::parse_from_rfc3339(&build_timestamp)?.timestamp();

    // The SemVer compatible version information for Foundry.
    // - The latest version from Cargo.toml.
    // - The short SHA of the latest commit.
    // - The UNIX formatted build timestamp.
    // - The build profile.
    // Example: forge 0.3.0-nightly+3cb96bde9b.1737036656.debug
    println!(
        "cargo:rustc-env=FOUNDRY_SEMVER_VERSION={version}+{sha_short}.{build_timestamp_unix}.{profile}"
    );

    // The short version information for the Foundry CLI.
    // - The latest version from Cargo.toml
    // - The short SHA of the latest commit.
    // Example: 0.3.0-dev (3cb96bde9b)
    println!("cargo:rustc-env=FOUNDRY_SHORT_VERSION={version} ({sha_short} {build_timestamp})");

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
    let long_version = format!(
        "\
Version: {version}
Commit SHA: {sha}
Build Timestamp: {build_timestamp} ({build_timestamp_unix})
Build Profile: {profile}"
    );
    assert_eq!(long_version.lines().count(), 4);
    for (i, line) in long_version.lines().enumerate() {
        println!("cargo:rustc-env=FOUNDRY_LONG_VERSION_{i}={line}");
    }

    Ok(())
}

fn env_var(name: &str) -> String {
    try_env_var(name).unwrap()
}

fn try_env_var(name: &str) -> Option<String> {
    println!("cargo:rerun-if-env-changed={name}");
    std::env::var(name).ok()
}
