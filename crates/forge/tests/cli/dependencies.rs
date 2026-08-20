//! Contains tests for `forge dependencies`.

use foundry_test_utils::{forgetest_init, util::OutputExt};
use soldeer_core::lock::{GitLockEntry, HttpLockEntry, LockEntry};
use std::fs;
use std::process::Command;

/// Writes a synthetic `soldeer.lock` with an HTTP-sourced and a Git-sourced dependency, using
/// Soldeer's own lockfile serializer so the fixture always matches the real on-disk schema. Also
/// creates each entry's install-path directory - `forge dependencies` only reports what's
/// actually present on disk, so a lockfile-only fixture would now be silently excluded.
fn write_soldeer_lock(root: &std::path::Path) {
    let entries = vec![
        LockEntry::from(
            HttpLockEntry::builder()
                .name("test-dep")
                .version("1.0.0")
                .url("https://example.com/test-dep-1.0.0.zip")
                .checksum("deadbeef")
                .integrity("beef")
                .build(),
        ),
        LockEntry::from(
            GitLockEntry::builder()
                .name("git-dep")
                .version("2.0.0")
                .git("https://github.com/example/git-dep.git")
                .rev("abc123def456")
                .build(),
        ),
    ];
    let deps_dir = root.join("dependencies");
    for entry in &entries {
        fs::create_dir_all(entry.install_path(&deps_dir)).unwrap();
    }
    let contents = soldeer_core::lock::generate_lockfile_contents(entries);
    fs::write(root.join("soldeer.lock"), contents).unwrap();
}

// `forge dependencies` lists both the Git submodule dependencies (forge-std, installed by
// `forgetest_init!`) and Soldeer dependencies (recorded in `soldeer.lock`) in one table.
forgetest_init!(dependencies_lists_submodules_and_soldeer, |prj, cmd| {
    write_soldeer_lock(prj.root());

    let output = cmd.arg("dependencies").assert_success().get_output().stdout_lossy();

    // Git submodule dependency, pinned via `forgetest_init!`'s own `foundry.lock`. Checking for
    // "tag=" (rather than a hardcoded tag name/rev) keeps this from breaking every time
    // forge-std cuts a new release, while still proving the lockfile pin was actually consulted
    // instead of silently falling back to a bare `rev=<hash>` - see
    // `dependencies_reports_lockfile_pinned_version` for a fixture-pinned, non-flaky version of
    // this same assertion.
    assert!(output.contains("forge-std"), "missing forge-std submodule entry:\n{output}");
    assert!(output.contains("submodule"), "missing submodule source label:\n{output}");
    assert!(output.contains("lib/forge-std"), "missing forge-std path:\n{output}");
    assert!(
        output.contains("tag=") || output.contains("branch="),
        "expected forge-std to report its foundry.lock pin, not a bare rev:\n{output}"
    );

    // Soldeer dependencies, read from `soldeer.lock`.
    assert!(output.contains("test-dep"), "missing test-dep entry:\n{output}");
    assert!(output.contains("git-dep"), "missing git-dep entry:\n{output}");
    assert!(output.contains("soldeer"), "missing soldeer source label:\n{output}");
    assert!(output.contains("1.0.0"), "missing test-dep version:\n{output}");
    assert!(output.contains("2.0.0"), "missing git-dep version:\n{output}");
});

// `--json` (the global flag shared by every `forge` subcommand) emits a machine-readable array
// with the same dependencies, source-tagged.
forgetest_init!(dependencies_json_output, |prj, cmd| {
    write_soldeer_lock(prj.root());

    let output = cmd.arg("dependencies").arg("--json").assert_success().get_output().stdout_lossy();

    let parsed: serde_json::Value = serde_json::from_str(&output)
        .unwrap_or_else(|err| panic!("expected valid JSON, got error {err}:\n{output}"));
    let entries = parsed.as_array().expect("expected a JSON array");
    assert_eq!(entries.len(), 3, "expected forge-std + 2 soldeer deps:\n{output}");

    let names: Vec<&str> = entries.iter().map(|e| e["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"forge-std"));
    assert!(names.contains(&"test-dep"));
    assert!(names.contains(&"git-dep"));

    let forge_std = entries.iter().find(|e| e["name"] == "forge-std").unwrap();
    assert_eq!(forge_std["source"], "submodule");
    assert_eq!(forge_std["path"], "lib/forge-std");

    let test_dep = entries.iter().find(|e| e["name"] == "test-dep").unwrap();
    assert_eq!(test_dep["source"], "soldeer");
    assert_eq!(test_dep["version"], "1.0.0");
    assert_eq!(test_dep["url"], "https://example.com/test-dep-1.0.0.zip");

    let git_dep = entries.iter().find(|e| e["name"] == "git-dep").unwrap();
    assert_eq!(git_dep["source"], "soldeer");
    assert_eq!(git_dep["version"], "2.0.0");
    assert_eq!(git_dep["url"], "https://github.com/example/git-dep.git");
});

// With no submodules and no `soldeer.lock`, the command reports an empty list rather than
// erroring.
forgetest_init!(dependencies_reports_none_when_absent, |prj, cmd| {
    fs::remove_dir_all(prj.root().join("lib")).unwrap();
    fs::remove_file(prj.root().join("foundry.lock")).ok();

    cmd.arg("dependencies").assert_success().stdout_eq(str![[r#"
No dependencies found

"#]]);

    cmd.forge_fuse().args(["dependencies", "--json"]).assert_success().stdout_eq(str![[r#"
[]

"#]]);
});

// A registered-but-never-checked-out submodule (e.g. after `git clone` without `--recursive`,
// or a manual `rm -rf lib/forge-std` without `git submodule deinit`) still leaves a gitlink in
// the index - `git submodule status` reports its last-known rev regardless. `forge dependencies`
// must not report it as installed just because the directory happens to exist (it's left behind
// empty in this scenario).
forgetest_init!(dependencies_excludes_uninitialized_submodule, |prj, cmd| {
    let forge_std = prj.root().join("lib/forge-std");
    fs::remove_dir_all(&forge_std).unwrap();
    fs::create_dir(&forge_std).unwrap();

    let output = cmd.arg("dependencies").assert_success().get_output().stdout_lossy();
    assert!(
        !output.contains("forge-std"),
        "uninitialized forge-std submodule should not be listed:\n{output}"
    );

    let json = cmd
        .forge_fuse()
        .args(["dependencies", "--json"])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.as_array().unwrap().len(), 0, "expected no dependencies:\n{json}");
});

// A submodule pinned in `foundry.lock` (via a prior `forge install <dep>@<tag>`) reports the
// pinned tag/rev pair instead of the bare checked-out rev.
forgetest_init!(dependencies_reports_lockfile_pinned_version, |prj, cmd| {
    fs::write(
        prj.root().join("foundry.lock"),
        r#"{
  "lib/forge-std": {
    "tag": {
      "name": "v1.9.4",
      "rev": "680ee6692649dcc7c617e05b2144932618264a83"
    }
  }
}
"#,
    )
    .unwrap();

    let output = cmd.arg("dependencies").assert_success().get_output().stdout_lossy();
    assert!(
        output.contains("tag=v1.9.4@680ee6692649dcc7c617e05b2144932618264a83"),
        "expected forge-std to report its foundry.lock pin, not a bare rev:\n{output}"
    );
});

// A `soldeer.lock` entry survives a fresh clone (Soldeer packages aren't checked into Git) or a
// package can be manually deleted from `dependencies/` without touching the lockfile. Either way,
// the entry is no longer actually installed, so it must not be reported as a dependency.
forgetest_init!(dependencies_excludes_soldeer_entry_missing_from_disk, |prj, cmd| {
    write_soldeer_lock(prj.root());
    fs::remove_dir_all(prj.root().join("dependencies/test-dep-1.0.0")).unwrap();

    let output = cmd.arg("dependencies").assert_success().get_output().stdout_lossy();
    assert!(
        !output.contains("test-dep"),
        "test-dep has no install directory and should not be listed:\n{output}"
    );
    assert!(output.contains("git-dep"), "git-dep is still on disk and should be listed:\n{output}");

    let json = cmd
        .forge_fuse()
        .args(["dependencies", "--json"])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    let names: Vec<&str> =
        parsed.as_array().unwrap().iter().map(|e| e["name"].as_str().unwrap()).collect();
    assert!(!names.contains(&"test-dep"), "test-dep leaked into --json output:\n{json}");
    assert!(names.contains(&"git-dep"), "git-dep missing from --json output:\n{json}");
});

// `Git`'s commands (including `git submodule status`) all run with the project root as their
// working directory, so `submodule.path()` is already relative to the project root regardless of
// nesting - `git submodule status` prints paths relative to cwd, not the Git repository root.
// When the project root sits below the Git root (a nested monorepo layout, e.g. a `--root
// apps/contracts` project inside a bigger repo), the path must stay project-relative (`lib/dep`)
// rather than leaking the Git root's view of it (`apps/contracts/lib/dep`), and the one place
// that genuinely needs the Git-root-relative form - the `git config submodule.<path>.url` lookup,
// since `.gitmodules` always keys on repo-root-relative paths - must still resolve correctly.
forgetest_init!(dependencies_rebases_nested_project_submodule_paths, |prj, cmd| {
    let remote = tempfile::tempdir().unwrap();
    let run_git = |args: &[&str], cwd: &std::path::Path| {
        let status = Command::new("git").args(args).current_dir(cwd).status().unwrap();
        assert!(status.success(), "git {args:?} failed in {}", cwd.display());
    };
    run_git(&["init", "-q", "-b", "main"], remote.path());
    run_git(&["commit", "-q", "--allow-empty", "-m", "init"], remote.path());

    let nested_root = prj.root().join("apps/contracts");
    fs::create_dir_all(&nested_root).unwrap();

    run_git(
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "-q",
            remote.path().to_str().unwrap(),
            "apps/contracts/lib/dep",
        ],
        prj.root(),
    );
    run_git(&["add", "-A"], prj.root());
    run_git(&["commit", "-q", "-m", "add nested submodule"], prj.root());

    let output = cmd
        .args(["dependencies", "--root"])
        .arg(&nested_root)
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert!(output.contains("lib/dep"), "expected project-relative path lib/dep:\n{output}");
    assert!(
        !output.contains("apps/contracts/lib/dep"),
        "path leaked the Git root's view instead of staying project-relative:\n{output}"
    );

    let json = cmd
        .forge_fuse()
        .args(["dependencies", "--json", "--root"])
        .arg(&nested_root)
        .assert_success()
        .get_output()
        .stdout_lossy();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    let entries = parsed.as_array().unwrap();
    let dep = entries.iter().find(|e| e["name"] == "dep").unwrap_or_else(|| {
        panic!("expected a submodule named 'dep' in {json}");
    });
    assert_eq!(dep["path"], "lib/dep");
    assert_eq!(
        dep["url"].as_str().unwrap(),
        remote.path().to_str().unwrap(),
        "submodule_url lookup must still resolve via the Git-root-relative path"
    );
});
