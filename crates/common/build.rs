#![expect(clippy::disallowed_macros)]

use chrono::{DateTime, SecondsFormat, Utc};
use std::{error::Error, path::PathBuf, process::Command};

fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo:rerun-if-changed=build.rs");

    let sha =
        try_env_var("VERGEN_GIT_SHA").or_else(git_sha).unwrap_or_else(|| String::from("unknown"));
    let sha_short = sha.get(..10).unwrap_or(&sha);

    let tag_name = try_env_var("TAG_NAME").unwrap_or_else(|| String::from("dev"));
    let version = release_version(&env_var("CARGO_PKG_VERSION"), &tag_name);
    let is_nightly = tag_name.starts_with("nightly");

    if is_nightly {
        println!("cargo:rustc-env=FOUNDRY_IS_NIGHTLY_VERSION=true");
    }

    // `PROFILE` captures only release or debug. Get the actual name from the out directory.
    let out_dir = PathBuf::from(env_var("OUT_DIR"));
    let profile = out_dir.components().rev().nth(3).unwrap().as_os_str().to_str().unwrap();

    let build_timestamp = build_timestamp()?;
    let build_timestamp_unix = build_timestamp.timestamp();
    let build_timestamp = build_timestamp.to_rfc3339_opts(SecondsFormat::AutoSi, true);

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

    // The long SHA of the latest commit.
    //
    // Example: 5186142d3bb4d1be7bb4ade548b77c8e2270717e
    println!("cargo:rustc-env=FOUNDRY_COMMIT_SHA={sha}");

    Ok(())
}

fn env_var(name: &str) -> String {
    try_env_var(name).unwrap()
}

/// Returns the build time, which is `SOURCE_DATE_EPOCH` when set.
fn build_timestamp() -> Result<DateTime<Utc>, Box<dyn Error>> {
    let Some(epoch) = try_env_var("SOURCE_DATE_EPOCH") else { return Ok(Utc::now()) };
    DateTime::from_timestamp(epoch.parse()?, 0).ok_or_else(|| "invalid SOURCE_DATE_EPOCH".into())
}

/// Returns the SHA of `HEAD` and reruns the script when `HEAD` moves.
///
/// Returns `None` outside a git worktree, e.g. when building from a release source tarball.
fn git_sha() -> Option<String> {
    let sha = git(&["rev-parse", "HEAD"])?;

    let head_ref = git(&["symbolic-ref", "HEAD"]);
    for name in ["HEAD"].into_iter().chain(head_ref.as_deref()) {
        // Cargo reruns on every build when a watched path does not exist, e.g. a packed ref.
        if let Some(path) = git(&["rev-parse", "--git-path", name]).map(PathBuf::from)
            && path.exists()
        {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }

    Some(sha)
}

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok().filter(|o| o.status.success())?;
    String::from_utf8(output.stdout).ok().map(|s| s.trim().to_owned())
}

fn release_version(pkg_version: &str, tag_name: &str) -> String {
    if let Some(version) = tag_name.strip_prefix('v') {
        return version.to_owned();
    }

    // Normalize `nightly-<sha>` to `nightly` so tarball and Docker nightly
    // artifacts produce the same version string. The commit identifier is
    // already included in the SemVer build metadata (after `+`).
    let normalized = if tag_name.starts_with("nightly-") { "nightly" } else { tag_name };

    format!("{pkg_version}-{normalized}")
}

fn try_env_var(name: &str) -> Option<String> {
    println!("cargo:rerun-if-env-changed={name}");
    std::env::var(name).ok()
}
