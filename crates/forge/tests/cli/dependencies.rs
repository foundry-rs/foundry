//! Contains tests for `forge dependencies`.

use foundry_test_utils::{forgetest_init, util::OutputExt};
use soldeer_core::lock::{GitLockEntry, HttpLockEntry, LockEntry};
use std::fs;

/// Writes a synthetic `soldeer.lock` with an HTTP-sourced and a Git-sourced dependency, using
/// Soldeer's own lockfile serializer so the fixture always matches the real on-disk schema.
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
