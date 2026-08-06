use foundry_test_utils::{forgetest, forgetest_init, str};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git").current_dir(root).args(args).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

fn add_local_submodule(root: &Path, path: &str) -> String {
    let source = root.join("lib/forge-std");
    let output = Command::new("git")
        .current_dir(root)
        .args(["-c", "protocol.file.allow=always", "submodule", "add", "--"])
        .arg(source)
        .arg(path)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    git(&root.join(path), &["rev-parse", "HEAD"])
}

forgetest_init!(lock_check_succeeds_when_dependencies_match, |_prj, cmd| {
    cmd.args(["lock", "--check"]).assert_success().stdout_eq("").stderr_eq("");
});

forgetest!(lock_check_succeeds_without_lockfile_or_dependencies, |prj, cmd| {
    cmd.git_init();
    assert!(!prj.root().join("foundry.lock").exists());

    cmd.args(["lock", "--check"]).assert_success().stdout_eq("").stderr_eq("");
});

forgetest_init!(lock_check_reports_revision_mismatch_without_modifying_lockfile, |prj, cmd| {
    let foundry_lock = prj.root().join("foundry.lock");
    let lockfile = r#"{
  "lib/forge-std": {
    "rev": "0000000000000000000000000000000000000000"
  }
}"#;
    fs::write(&foundry_lock, lockfile).unwrap();

    cmd.args(["lock", "--check"]).assert_failure().stdout_eq("").stderr_eq(str![[r#"
Error: foundry.lock does not match installed dependencies:
  lib/forge-std: expected 0000000000000000000000000000000000000000, found [..]

"#]]);
    assert_eq!(fs::read_to_string(foundry_lock).unwrap(), lockfile);
});

forgetest_init!(lock_check_reports_dependencies_missing_from_lockfile, |prj, cmd| {
    fs::remove_file(prj.root().join("foundry.lock")).unwrap();

    cmd.args(["lock", "--check"]).assert_failure().stdout_eq("").stderr_eq(str![[r#"
Error: foundry.lock does not match installed dependencies:
  lib/forge-std: missing from foundry.lock (found [..])

"#]]);
});

forgetest_init!(lock_check_rejects_malformed_lockfile, |prj, cmd| {
    fs::write(prj.root().join("foundry.lock"), "not json").unwrap();

    cmd.args(["lock", "--check"]).assert_failure().stdout_eq("").stderr_eq(str![[r#"
Error: Failed to read foundry.lock

Context:
- expected ident at line 1 column 2

"#]]);
});

forgetest_init!(lock_check_reports_stale_lockfile_entries, |prj, cmd| {
    let foundry_lock = prj.root().join("foundry.lock");
    let mut lockfile: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&foundry_lock).unwrap()).unwrap();
    lockfile["lib/stale"] = serde_json::json!({
        "rev": "0000000000000000000000000000000000000000"
    });
    fs::write(foundry_lock, serde_json::to_vec_pretty(&lockfile).unwrap()).unwrap();

    cmd.args(["lock", "--check"]).assert_failure().stdout_eq("").stderr_eq(str![[r#"
Error: foundry.lock does not match installed dependencies:
  lib/stale: dependency submodule is missing (expected 0000000000000000000000000000000000000000)

"#]]);
});

forgetest_init!(lock_check_reports_uninitialized_dependencies, |prj, cmd| {
    let foundry_lock = fs::read(prj.root().join("foundry.lock")).unwrap();
    assert!(
        Command::new("git")
            .current_dir(prj.root())
            .args(["submodule", "deinit", "-f", "lib/forge-std"])
            .status()
            .unwrap()
            .success()
    );
    let index = fs::read(prj.root().join(".git/index")).unwrap();
    let git_config = fs::read(prj.root().join(".git/config")).unwrap();

    cmd.args(["lock", "--check"]).assert_failure().stdout_eq("").stderr_eq(str![[r#"
Error: foundry.lock does not match installed dependencies:
  lib/forge-std: dependency submodule is not initialized (expected [..])

"#]]);
    assert_eq!(fs::read(prj.root().join("foundry.lock")).unwrap(), foundry_lock);
    assert_eq!(fs::read(prj.root().join(".git/index")).unwrap(), index);
    assert_eq!(fs::read(prj.root().join(".git/config")).unwrap(), git_config);
    assert!(!prj.root().join("lib/forge-std/.git").exists());
});

forgetest!(lock_check_supports_projects_nested_in_parent_repository, |prj, cmd| {
    cmd.git_init();
    cmd.args(["init", "nested", "--use-parent-git"]).assert_success();

    let root = prj.root();
    let output = Command::new("git")
        .current_dir(root)
        .args(["-c", "protocol.file.allow=always", "submodule", "add", "--"])
        .arg(root.join("nested/lib/forge-std"))
        .arg("sibling")
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));

    cmd.forge_fuse().args(["install", "--root", "nested"]).assert_success();
    let foundry_lock = root.join("nested/foundry.lock");
    let mut lockfile: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&foundry_lock).unwrap()).unwrap();
    assert!(lockfile.get("../sibling").is_some());
    for (path, dependency) in lockfile.as_object_mut().unwrap() {
        dependency["rev"] =
            serde_json::Value::String(git(&root.join("nested").join(path), &["rev-parse", "HEAD"]));
    }
    fs::write(foundry_lock, serde_json::to_vec_pretty(&lockfile).unwrap()).unwrap();

    cmd.forge_fuse()
        .args(["lock", "--check", "--root", "nested"])
        .assert_success()
        .stdout_eq("")
        .stderr_eq("");
});

forgetest_init!(lock_check_supports_dependency_paths_with_spaces, |prj, cmd| {
    let root = prj.root();
    git(root, &["mv", "lib/forge-std", "lib/forge std"]);

    let foundry_lock = root.join("foundry.lock");
    let mut lockfile: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&foundry_lock).unwrap()).unwrap();
    let forge_std = lockfile.as_object_mut().unwrap().remove("lib/forge-std").unwrap();
    lockfile["lib/forge std"] = forge_std;
    fs::write(foundry_lock, serde_json::to_vec_pretty(&lockfile).unwrap()).unwrap();

    cmd.args(["lock", "--check"]).assert_success().stdout_eq("").stderr_eq("");
});

forgetest_init!(lock_check_accepts_modified_submodule_when_head_matches_lock, |prj, cmd| {
    let root = prj.root();
    let submodule = root.join("lib/forge-std");
    let previous = git(&submodule, &["rev-parse", "HEAD^"]);
    git(root, &["update-index", "--cacheinfo", "160000", &previous, "lib/forge-std"]);

    cmd.args(["lock", "--check"]).assert_success().stdout_eq("").stderr_eq("");
});

forgetest_init!(lock_check_reports_conflicted_submodule, |prj, cmd| {
    let root = prj.root();
    let submodule = root.join("lib/forge-std");
    let current = git(&submodule, &["rev-parse", "HEAD"]);
    let previous = git(&submodule, &["rev-parse", "HEAD^"]);
    git(root, &["update-index", "--force-remove", "lib/forge-std"]);
    let mut child = Command::new("git")
        .current_dir(root)
        .args(["update-index", "--index-info"])
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    write!(
        child.stdin.take().unwrap(),
        "160000 {previous} 1\tlib/forge-std\n160000 {current} 2\tlib/forge-std\n160000 {previous} 3\tlib/forge-std\n"
    )
    .unwrap();
    assert!(child.wait().unwrap().success());

    cmd.args(["lock", "--check"]).assert_failure().stdout_eq("").stderr_eq(str![[r#"
Error: foundry.lock does not match installed dependencies:
  lib/forge-std: dependency submodule has merge conflicts

"#]]);
});

forgetest_init!(lock_check_reports_all_mismatches_in_path_order, |prj, cmd| {
    let root = prj.root();
    add_local_submodule(root, "lib/second");
    let foundry_lock = root.join("foundry.lock");
    let mut lockfile: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&foundry_lock).unwrap()).unwrap();
    lockfile["lib/forge-std"]["rev"] =
        serde_json::Value::String("0000000000000000000000000000000000000000".to_string());
    lockfile["lib/stale"] = serde_json::json!({
        "rev": "1111111111111111111111111111111111111111"
    });
    fs::write(foundry_lock, serde_json::to_vec_pretty(&lockfile).unwrap()).unwrap();

    cmd.args(["lock", "--check"]).assert_failure().stdout_eq("").stderr_eq(str![[r#"
Error: foundry.lock does not match installed dependencies:
  lib/forge-std: expected 0000000000000000000000000000000000000000, found [..]
  lib/second: missing from foundry.lock (found [..])
  lib/stale: dependency submodule is missing (expected 1111111111111111111111111111111111111111)

"#]]);
});

forgetest_init!(lock_check_supports_custom_dependency_directory, |prj, cmd| {
    let root = prj.root();
    fs::create_dir(root.join("dependencies")).unwrap();
    git(root, &["mv", "lib/forge-std", "dependencies/forge-std"]);
    prj.update_config(|config| config.libs = vec![PathBuf::from("dependencies")]);

    let foundry_lock = root.join("foundry.lock");
    let mut lockfile: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&foundry_lock).unwrap()).unwrap();
    let forge_std = lockfile.as_object_mut().unwrap().remove("lib/forge-std").unwrap();
    lockfile["dependencies/forge-std"] = forge_std;
    fs::write(foundry_lock, serde_json::to_vec_pretty(&lockfile).unwrap()).unwrap();

    cmd.args(["lock", "--check"]).assert_success().stdout_eq("").stderr_eq("");
});

forgetest_init!(lock_check_supports_project_root_as_dependency_directory, |prj, cmd| {
    prj.update_config(|config| config.libs = vec![PathBuf::from(".")]);

    cmd.args(["lock", "--check"]).assert_success().stdout_eq("").stderr_eq("");
});

forgetest_init!(lock_check_matches_synced_submodules_outside_dependency_directory, |prj, cmd| {
    let root = prj.root();
    add_local_submodule(root, "vendor/second");

    cmd.forge_fuse().args(["install"]).assert_success();

    let mut lockfile: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(root.join("foundry.lock")).unwrap()).unwrap();
    assert!(lockfile.get("vendor/second").is_some());
    lockfile["lib/forge-std"]["rev"] =
        serde_json::Value::String(git(&root.join("lib/forge-std"), &["rev-parse", "HEAD"]));
    fs::write(root.join("foundry.lock"), serde_json::to_vec_pretty(&lockfile).unwrap()).unwrap();

    cmd.forge_fuse().args(["lock", "--check"]).assert_success().stdout_eq("").stderr_eq("");
});

forgetest_init!(lock_check_aggregates_missing_submodule_mapping, |prj, cmd| {
    let root = prj.root();
    let head = git(&root.join("lib/forge-std"), &["rev-parse", "HEAD"]);
    git(root, &["update-index", "--add", "--cacheinfo", "160000", &head, "lib/unmapped"]);
    let mut gitmodules = fs::read_to_string(root.join(".gitmodules")).unwrap();
    gitmodules.push_str("\n[submodule \"lib/unmapped\"]\n\turl = ../unmapped\n");
    fs::write(root.join(".gitmodules"), gitmodules).unwrap();

    let foundry_lock = root.join("foundry.lock");
    let mut lockfile: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&foundry_lock).unwrap()).unwrap();
    lockfile["lib/stale"] = serde_json::json!({
        "rev": "1111111111111111111111111111111111111111"
    });
    fs::write(foundry_lock, serde_json::to_vec_pretty(&lockfile).unwrap()).unwrap();

    cmd.args(["lock", "--check"]).assert_failure().stdout_eq("").stderr_eq(str![[r#"
Error: foundry.lock does not match installed dependencies:
  lib/stale: dependency submodule is missing (expected 1111111111111111111111111111111111111111)
  lib/unmapped: dependency submodule is missing from .gitmodules

"#]]);
});
