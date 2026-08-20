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
    assert!(
        output.contains("https://github.com/foundry-rs/forge-std"),
        "the table output must show a URL column, not just --json:\n{output}"
    );

    // Soldeer dependencies, read from `soldeer.lock`.
    assert!(output.contains("test-dep"), "missing test-dep entry:\n{output}");
    assert!(output.contains("git-dep"), "missing git-dep entry:\n{output}");
    assert!(output.contains("soldeer"), "missing soldeer source label:\n{output}");
    assert!(output.contains("1.0.0"), "missing test-dep version:\n{output}");
    assert!(output.contains("2.0.0"), "missing git-dep version:\n{output}");
    assert!(
        output.contains("https://example.com/test-dep-1.0.0.zip"),
        "missing test-dep URL in table output:\n{output}"
    );
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

/// The commit `path` (a submodule directory) is actually checked out at.
fn checked_out_rev(path: &std::path::Path) -> String {
    let output =
        Command::new("git").args(["rev-parse", "HEAD"]).current_dir(path).output().unwrap();
    assert!(output.status.success(), "git rev-parse HEAD failed in {}", path.display());
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

// A submodule pinned in `foundry.lock` (via a prior `forge install <dep>@<tag>`) reports the
// pinned tag/rev pair instead of the bare checked-out rev - but only when the pin still matches
// what's actually checked out; see `dependencies_falls_back_to_actual_rev_on_checkout_mismatch`
// for the drifted case.
forgetest_init!(dependencies_reports_lockfile_pinned_version, |prj, cmd| {
    let actual_rev = checked_out_rev(&prj.root().join("lib/forge-std"));
    fs::write(
        prj.root().join("foundry.lock"),
        format!(
            r#"{{
  "lib/forge-std": {{
    "tag": {{
      "name": "v1.9.4",
      "rev": "{actual_rev}"
    }}
  }}
}}
"#
        ),
    )
    .unwrap();

    let output = cmd.arg("dependencies").assert_success().get_output().stdout_lossy();
    assert!(
        output.contains(&format!("tag=v1.9.4@{actual_rev}")),
        "expected forge-std to report its foundry.lock pin, not a bare rev:\n{output}"
    );
});

// If a submodule was manually moved to a different commit than what's recorded in `foundry.lock`
// (`git submodule status`'s `+` marker - the superproject's index hasn't been updated to match),
// showing the stale locked tag would misrepresent what's actually checked out on disk. Falls back
// to the real rev instead.
forgetest_init!(dependencies_falls_back_to_actual_rev_on_checkout_mismatch, |prj, cmd| {
    let actual_rev = checked_out_rev(&prj.root().join("lib/forge-std"));
    fs::write(
        prj.root().join("foundry.lock"),
        r#"{
  "lib/forge-std": {
    "tag": {
      "name": "v1.9.4",
      "rev": "0000000000000000000000000000000000000000"
    }
  }
}
"#,
    )
    .unwrap();

    let output = cmd.arg("dependencies").assert_success().get_output().stdout_lossy();
    assert!(
        output.contains(&format!("rev={actual_rev}")),
        "expected the real checked-out rev, not the stale foundry.lock pin:\n{output}"
    );
    assert!(
        !output.contains("tag=v1.9.4"),
        "must not report the foundry.lock tag once it no longer matches the checkout:\n{output}"
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

// On macOS, `--root /tmp/project` and the canonical `/private/tmp/project` name the same
// directory but compare unequal as strings. `install_lib_dir` gets canonicalized by
// `Config::canonic_at`; `config.root` does not. A naive `strip_prefix` between the two silently
// falls back to the absolute path, which then never matches any submodule's project-relative
// path - excluding every submodule from a symlinked project root.
#[cfg(unix)]
forgetest_init!(dependencies_resolves_symlinked_project_root, |prj, cmd| {
    let symlink_parent = tempfile::tempdir().unwrap();
    let symlinked_root = symlink_parent.path().join("project");
    std::os::unix::fs::symlink(prj.root(), &symlinked_root).unwrap();

    let output = cmd
        .args(["dependencies", "--root"])
        .arg(&symlinked_root)
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert!(
        output.contains("forge-std"),
        "a symlinked project root excluded a real submodule:\n{output}"
    );
});

// `.gitmodules` doesn't require a submodule's section name to match its `path` field - `git
// submodule add --name openzeppelin <url> lib/openzeppelin` records `path = lib/openzeppelin`
// under section `openzeppelin`. `git config submodule.<key>.url` is keyed by the section name, so
// looking it up by path text alone finds nothing.
forgetest_init!(dependencies_resolves_url_by_gitmodules_section_name, |prj, cmd| {
    let remote = tempfile::tempdir().unwrap();
    let run_git = |args: &[&str], cwd: &std::path::Path| {
        let status = Command::new("git").args(args).current_dir(cwd).status().unwrap();
        assert!(status.success(), "git {args:?} failed in {}", cwd.display());
    };
    run_git(&["init", "-q", "-b", "main"], remote.path());
    run_git(&["commit", "-q", "--allow-empty", "-m", "init"], remote.path());

    run_git(
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "-q",
            "--name",
            "openzeppelin",
            remote.path().to_str().unwrap(),
            "lib/openzeppelin",
        ],
        prj.root(),
    );
    run_git(&["add", "-A"], prj.root());
    run_git(&["commit", "-q", "-m", "add submodule with a renamed section"], prj.root());

    let json = cmd.args(["dependencies", "--json"]).assert_success().get_output().stdout_lossy();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    let dep =
        parsed.as_array().unwrap().iter().find(|e| e["name"] == "openzeppelin").unwrap_or_else(
            || {
                panic!("expected an 'openzeppelin' submodule in {json}");
            },
        );
    assert_eq!(
        dep["url"].as_str().unwrap(),
        remote.path().to_str().unwrap(),
        "URL lookup must resolve via the .gitmodules section name, not the path text"
    );
});

// `git submodule status` prints `U<40 zeros> path` for a submodule with an unresolved merge
// conflict - `Submodule`'s status-line regex only recognizes ` `/`+`/`-` prefixes, not `U`, so
// parsing the *entire* `git submodule status` output fails, not just that one line. Silently
// falling back to an empty list here would print "No dependencies found" while the repo is
// mid-conflict, which is exactly the "looks empty but isn't" failure mode this command exists to
// avoid - it should fail loudly instead.
forgetest_init!(dependencies_errors_loudly_on_conflicted_submodule, |prj, cmd| {
    let remote = tempfile::tempdir().unwrap();
    let run_git = |args: &[&str], cwd: &std::path::Path| {
        let status = Command::new("git").args(args).current_dir(cwd).status().unwrap();
        assert!(status.success(), "git {args:?} failed in {}", cwd.display());
    };
    let rev_parse = |cwd: &std::path::Path| -> String {
        let output =
            Command::new("git").args(["rev-parse", "HEAD"]).current_dir(cwd).output().unwrap();
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    };
    // `forgetest_init!`'s project has an initialized `.git` but no commits yet (an unborn HEAD) -
    // give it one so there's a real commit to branch from below.
    run_git(&["add", "-A"], prj.root());
    run_git(&["commit", "-q", "-m", "test setup"], prj.root());
    let default_branch = {
        let output = Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .current_dir(prj.root())
            .output()
            .unwrap();
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    };

    // Two divergent commits in the remote, neither an ancestor of the other, so pinning the
    // submodule to each on separate branches produces a real (non-fast-forwardable) conflict.
    run_git(&["init", "-q", "-b", "main"], remote.path());
    fs::write(remote.path().join("file.txt"), "base").unwrap();
    run_git(&["add", "-A"], remote.path());
    run_git(&["commit", "-q", "-m", "base"], remote.path());
    let rev_base = rev_parse(remote.path());
    run_git(&["checkout", "-q", "-b", "branch-a"], remote.path());
    fs::write(remote.path().join("file.txt"), "a").unwrap();
    run_git(&["commit", "-q", "-am", "a"], remote.path());
    let rev_a = rev_parse(remote.path());
    run_git(&["checkout", "-q", "main"], remote.path());
    run_git(&["checkout", "-q", "-b", "branch-b"], remote.path());
    fs::write(remote.path().join("file.txt"), "b").unwrap();
    run_git(&["commit", "-q", "-am", "b"], remote.path());
    let rev_b = rev_parse(remote.path());

    run_git(
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "-q",
            remote.path().to_str().unwrap(),
            "lib/dep",
        ],
        prj.root(),
    );
    let submodule_path = prj.root().join("lib/dep");
    // `submodule add` checks out whatever the remote's HEAD happened to point at (`branch-b`,
    // the last branch checked out above) - reset to the shared base first so `feature` and
    // `other` below actually diverge from a common ancestor, instead of one of them being a
    // no-op relative to what got cloned (which merges cleanly instead of conflicting).
    run_git(&["checkout", "-q", &rev_base], &submodule_path);
    run_git(&["add", "-A"], prj.root());
    run_git(&["commit", "-q", "-m", "add submodule"], prj.root());

    run_git(&["checkout", "-q", "-b", "feature"], prj.root());
    run_git(&["checkout", "-q", &rev_a], &submodule_path);
    run_git(&["commit", "-q", "-am", "pin branch-a"], prj.root());

    run_git(&["checkout", "-q", &default_branch], prj.root());
    run_git(&["checkout", "-q", "-b", "other"], prj.root());
    run_git(&["checkout", "-q", &rev_b], &submodule_path);
    run_git(&["commit", "-q", "-am", "pin branch-b"], prj.root());

    // This merge is expected to conflict - don't assert success. `-c merge.ff=false` pins the
    // merge strategy so this doesn't depend on the running machine's ambient git config: with
    // `merge.ff=only` set globally (a real policy some teams/CI templates enforce), `git merge`
    // refuses outright instead of attempting the merge, and `lib/dep` never becomes conflicted -
    // the assertion below does catch that (it fails loudly rather than passing vacuously), but
    // there's no reason to depend on ambient config for a merge this test controls entirely.
    let _ = Command::new("git")
        .args(["-c", "merge.ff=false", "merge", "feature"])
        .current_dir(prj.root())
        .output()
        .unwrap();
    assert!(
        Command::new("git")
            .args(["status", "--short"])
            .current_dir(prj.root())
            .output()
            .unwrap()
            .stdout
            .starts_with(b"UU"),
        "expected the merge to leave lib/dep conflicted - test setup didn't reproduce a real conflict"
    );

    cmd.arg("dependencies").assert_failure().stderr_eq(str![[r#"
Error: failed to parse `git submodule status` - a merge-conflicted submodule can cause this

Context:
- Invalid submodule status format

"#]]);
});

// `git submodule add` populates both `.gitmodules` and the local `.git/config` with matching
// `submodule.<name>.url` entries, but they can drift apart in the wild - a submodule registered
// in `.gitmodules` whose local config entry was never populated (`git submodule init` never run)
// or got removed still has a real URL recorded in `.gitmodules` itself. The URL lookup must not
// depend on the local config copy existing.
forgetest_init!(dependencies_resolves_url_when_never_locally_initialized, |prj, cmd| {
    let remote = tempfile::tempdir().unwrap();
    let run_git = |args: &[&str], cwd: &std::path::Path| {
        let status = Command::new("git").args(args).current_dir(cwd).status().unwrap();
        assert!(status.success(), "git {args:?} failed in {}", cwd.display());
    };
    run_git(&["init", "-q", "-b", "main"], remote.path());
    run_git(&["commit", "-q", "--allow-empty", "-m", "init"], remote.path());

    run_git(
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "-q",
            remote.path().to_str().unwrap(),
            "lib/dep",
        ],
        prj.root(),
    );
    run_git(&["add", "-A"], prj.root());
    run_git(&["commit", "-q", "-m", "add submodule"], prj.root());

    // Simulate the drift: strip the local config copy, leaving only `.gitmodules`'s.
    run_git(&["config", "--unset", "submodule.lib/dep.url"], prj.root());
    assert!(
        Command::new("git")
            .args(["config", "--get", "submodule.lib/dep.url"])
            .current_dir(prj.root())
            .status()
            .unwrap()
            .code()
            != Some(0),
        "test setup didn't actually remove the local config entry"
    );

    let json = cmd.args(["dependencies", "--json"]).assert_success().get_output().stdout_lossy();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    let dep = parsed.as_array().unwrap().iter().find(|e| e["name"] == "dep").unwrap_or_else(|| {
        panic!("expected a 'dep' submodule in {json}");
    });
    assert_eq!(
        dep["url"].as_str().unwrap(),
        remote.path().to_str().unwrap(),
        "URL should still resolve from .gitmodules even with no local config entry"
    );
});

// `git submodule status` always reports the full 40-char checkout SHA, but a `foundry.lock`
// `Rev`-variant entry (from `forge install dep@<rev>`) stores whatever the user typed verbatim -
// an abbreviated hash is common and still accurate. A bare string-equality check between the two
// would reject a genuinely matching pin just because it's shorter, degrading the display to a
// bare `rev=` instead of the pinned lockfile entry.
forgetest_init!(dependencies_matches_abbreviated_rev_lockfile_entry, |prj, cmd| {
    let actual_rev = checked_out_rev(&prj.root().join("lib/forge-std"));
    let abbreviated = &actual_rev[..10];
    fs::write(
        prj.root().join("foundry.lock"),
        format!(
            r#"{{
  "lib/forge-std": {{
    "rev": "{abbreviated}"
  }}
}}
"#
        ),
    )
    .unwrap();

    let output = cmd.arg("dependencies").assert_success().get_output().stdout_lossy();
    assert!(
        output.contains(&format!("rev={abbreviated}")),
        "expected the pinned abbreviated rev, not a fallback to the full checkout hash:\n{output}"
    );
    assert!(
        !output.contains(&format!("rev={actual_rev}")),
        "should show the lockfile's abbreviated form, not the full hash:\n{output}"
    );
});
