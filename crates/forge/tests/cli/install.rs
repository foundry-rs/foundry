//! forge install and update tests

use forge::{DepIdentifier, FOUNDRY_LOCK, Lockfile};
use foundry_cli::utils::{Git, Submodules};
use foundry_compilers::artifacts::Remapping;
use foundry_config::Config;
use foundry_test_utils::util::{
    ExtTester, FORGE_STD_REVISION, OutputExt, TestCommand, pretty_err, read_string,
};
use semver::Version;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    str::FromStr,
};
use url::Url;

#[cfg(unix)]
use std::os::unix::fs::symlink;

fn lockfile_get(root: &Path, dep_path: &Path) -> Option<DepIdentifier> {
    let mut l = Lockfile::new(root);
    l.read().unwrap();
    l.get(dep_path).cloned()
}

// checks missing dependencies are auto installed
#[forgetest_init]
fn can_install_missing_deps_build(prj: _, cmd: _) {
    prj.initialize_default_contracts();
    prj.clear();

    // wipe forge-std
    let forge_std_dir = prj.root().join("lib/forge-std");
    pretty_err(&forge_std_dir, fs::remove_dir_all(&forge_std_dir));

    // Build the project
    cmd.arg("build")
        .assert_success()
        .stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

"#]])
        .stderr_eq(str![[r#"
Missing dependencies found. Installing now...
[UPDATING_DEPENDENCIES]
...
"#]]);

    // assert lockfile
    let forge_std = lockfile_get(prj.root(), &PathBuf::from("lib/forge-std")).unwrap();
    assert_eq!(forge_std.rev(), FORGE_STD_REVISION);

    // Expect compilation to be skipped as no files have changed
    cmd.forge_fuse().arg("build").assert_success().stdout_eq(str![[r#"
No files changed, compilation skipped

"#]]);
}

// checks missing dependencies are auto installed
#[forgetest_init]
fn can_install_missing_deps_test(prj: _, cmd: _) {
    prj.initialize_default_contracts();
    prj.clear();

    // wipe forge-std
    let forge_std_dir = prj.root().join("lib/forge-std");
    pretty_err(&forge_std_dir, fs::remove_dir_all(&forge_std_dir));

    cmd.arg("test")
        .assert_success()
        .stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 2 tests for test/Counter.t.sol:CounterTest
[PASS] testFuzz_SetNumber(uint256) (runs: 256, [AVG_GAS])
[PASS] test_Increment() ([GAS])
Suite result: ok. 2 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 2 tests passed, 0 failed, 0 skipped (2 total tests)

"#]])
        .stderr_eq(str![[r#"
Missing dependencies found. Installing now...
[UPDATING_DEPENDENCIES]
...
"#]]);

    // assert lockfile
    let forge_std = lockfile_get(prj.root(), &PathBuf::from("lib/forge-std")).unwrap();
    assert_eq!(forge_std.rev(), FORGE_STD_REVISION);
}

// Checks missing dependencies are auto installed.
#[forgetest_init]
fn can_install_missing_deps_lint(prj: _, cmd: _) {
    prj.initialize_default_contracts();
    prj.clear();

    // Wipe forge-std.
    let forge_std_dir = prj.root().join("lib/forge-std");
    pretty_err(&forge_std_dir, fs::remove_dir_all(&forge_std_dir));

    cmd.arg("lint").assert_success().stdout_eq(str![""]).stderr_eq(str![[r#"
Missing dependencies found. Installing now...
[UPDATING_DEPENDENCIES]
...
"#]]);

    // Assert lockfile.
    let forge_std = lockfile_get(prj.root(), &PathBuf::from("lib/forge-std")).unwrap();
    assert_eq!(forge_std.rev(), FORGE_STD_REVISION);
}

// test to check that install/remove works properly
#[forgetest]
fn can_install_and_remove(prj: _, cmd: _) {
    cmd.git_init();

    let libs = prj.root().join("lib");
    let git_mod = prj.root().join(".git/modules/lib");
    let git_mod_file = prj.root().join(".gitmodules");

    let forge_std = libs.join("forge-std");
    let forge_std_mod = git_mod.join("forge-std");

    let install = |cmd: &mut TestCommand| {
        cmd.forge_fuse()
            .args(["install", "foundry-rs/forge-std"])
            .assert_success()
            .stdout_eq(str![""])
            .stderr_eq(str![[r#"
Installing forge-std in [..] (url: https://github.com/foundry-rs/forge-std, tag: None)
...
    Installed forge-std[..]

"#]]);

        assert!(forge_std.exists());
        assert!(forge_std_mod.exists());

        let submods = read_string(&git_mod_file);
        assert!(submods.contains("https://github.com/foundry-rs/forge-std"));
    };

    let remove = |cmd: &mut TestCommand, target: &str| {
        cmd.forge_fuse()
            .args(["remove", "--force", target])
            .assert_success()
            .stdout_eq(str![""])
            .stderr_eq(str![[r#"
Removing 'forge-std' in [..], (url: https://github.com/foundry-rs/forge-std, tag: None)

"#]]);

        assert!(!forge_std.exists());
        assert!(!forge_std_mod.exists());
        let submods = read_string(&git_mod_file);
        assert!(!submods.contains("https://github.com/foundry-rs/forge-std"));
    };

    install(&mut cmd);
    let forge_std = lockfile_get(prj.root(), &PathBuf::from("lib/forge-std")).unwrap();
    assert!(matches!(forge_std, DepIdentifier::Tag { .. }));
    remove(&mut cmd, "forge-std");
    let forge_std = lockfile_get(prj.root(), &PathBuf::from("lib/forge-std"));
    assert!(forge_std.is_none());

    // install again and remove via relative path
    install(&mut cmd);
    remove(&mut cmd, "lib/forge-std");
}

// https://github.com/foundry-rs/foundry/issues/6790
#[forgetest]
fn failed_install_leaves_repository_clean(prj: _, cmd: _) {
    cmd.git_init();
    let git = Git::new(prj.root());
    assert!(git.is_clean().unwrap());

    cmd.forge_fuse()
        .args(["install", "vectorized/solady@this-tag-does-not-exist"])
        .assert_failure()
        .stderr_eq(str![[r#"
Installing solady in [..] (url: https://github.com/vectorized/solady, tag: this-tag-does-not-exist)
...
Error: Tag: "this-tag-does-not-exist" not found for repo "https://github.com/vectorized/solady"!

"#]]);

    assert!(git.is_clean().unwrap());
    assert!(!prj.root().join(".gitmodules").exists());
    assert!(!prj.root().join("lib/solady").exists());
    assert!(!git.absolute_git_dir().unwrap().join("modules/lib/solady").exists());
    assert!(git.submodule_url(Path::new("lib/solady")).unwrap_or(None).is_none());
}

#[forgetest]
fn install_rejects_assume_unchanged_orphan_gitlink(prj: _, cmd: _) {
    cmd.git_init();
    fs::write(prj.root().join("README.md"), "baseline").unwrap();
    cmd.git_add();
    cmd.git_commit("baseline");
    let head = Command::new("git")
        .current_dir(prj.root())
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap()
        .stdout;
    let head = String::from_utf8(head).unwrap();
    let git = |args: &[&str]| {
        assert!(Command::new("git").current_dir(prj.root()).args(args).status().unwrap().success());
    };
    git(&["update-index", "--add", "--cacheinfo", "160000", head.trim(), "lib/solady"]);
    git(&["update-index", "--assume-unchanged", "lib/solady"]);
    let index = fs::read(prj.root().join(".git/index")).unwrap();
    let status = Command::new("git")
        .current_dir(prj.root())
        .args(["status", "--porcelain=v2"])
        .output()
        .unwrap()
        .stdout;

    cmd.forge_fuse().args(["install", "vectorized/solady"]).assert_failure();

    assert_eq!(fs::read(prj.root().join(".git/index")).unwrap(), index);
    assert_eq!(
        Command::new("git")
            .current_dir(prj.root())
            .args(["status", "--porcelain=v2"])
            .output()
            .unwrap()
            .stdout,
        status
    );
    assert!(!prj.root().join(".gitmodules").exists());
    assert!(!prj.root().join("lib/solady").exists());
}

#[forgetest]
fn failed_install_with_wildcard_alias_preserves_sibling(prj: _, cmd: _) {
    cmd.git_init();
    cmd.forge_fuse().args(["install", "foundry-rs/forge-std"]).assert_success();
    let sibling = prj.root().join("lib/forge-std");
    assert!(
        Command::new("git")
            .current_dir(&sibling)
            .args(["checkout", "HEAD^"])
            .output()
            .unwrap()
            .status
            .success()
    );
    let index = || {
        Command::new("git")
            .current_dir(prj.root())
            .args(["ls-files", "--stage", "-v", "-z"])
            .output()
            .unwrap()
            .stdout
    };
    let index_before = index();

    cmd.forge_fuse()
        .args(["install", "*=vectorized/solady@this-tag-does-not-exist"])
        .assert_failure();

    assert_eq!(index(), index_before);
    assert!(sibling.join(".git").exists());
    assert!(Git::new(prj.root()).is_gitlink(Path::new("lib/forge-std")).unwrap());
}

#[forgetest]
fn install_rejects_non_normal_alias(prj: _, cmd: _) {
    cmd.git_init();
    let git = Git::new(prj.root());

    cmd.forge_fuse()
        .args(["install", "foo/../solady=vectorized/solady@this-tag-does-not-exist"])
        .assert_failure();

    assert!(git.is_clean().unwrap());
    assert!(!prj.root().join(".gitmodules").exists());
    assert!(!prj.root().join("lib/solady").exists());
    assert!(!git.absolute_git_dir().unwrap().join("modules/lib/solady").exists());
    assert!(!git.has_submodule_config(Path::new("lib/solady")).unwrap());
}

#[forgetest]
fn failed_install_preserves_gitmodules(prj: _, cmd: _) {
    cmd.git_init();
    let gitmodules = prj.root().join(".gitmodules");
    let original = r#"[submodule "existing"]
	path = lib/existing
	url = https://github.com/example/existing
"#;
    fs::write(&gitmodules, original).unwrap();
    cmd.git_add();
    cmd.git_commit("add gitmodules");

    cmd.forge_fuse()
        .args(["install", "vectorized/solady@this-tag-does-not-exist"])
        .assert_failure();

    assert!(Git::new(prj.root()).is_clean().unwrap());
    assert_eq!(read_string(&gitmodules), original);
}

#[forgetest]
fn failed_submodule_add_leaves_repository_clean(prj: _, cmd: _) {
    cmd.git_init();
    let git = Git::new(prj.root());
    let git_dir = git.absolute_git_dir().unwrap();
    let index_lock = git_dir.join("index.lock");
    fs::write(&index_lock, []).unwrap();

    let output = cmd
        .forge_fuse()
        .args(["install", "vectorized/solady@this-tag-does-not-exist"])
        .assert_failure();
    assert!(output.get_output().stderr_lossy().contains("index.lock"));

    assert!(index_lock.exists());
    assert!(git.is_clean().unwrap());
    assert!(!prj.root().join(".gitmodules").exists());
    assert!(!prj.root().join("lib/solady").exists());
    assert!(!git_dir.join("modules/lib/solady").exists());
    assert!(!git.has_submodule_config(Path::new("lib/solady")).unwrap());
}

#[forgetest]
fn install_rejects_dirty_gitmodules(prj: _, cmd: _) {
    cmd.git_init();
    let git = Git::new(prj.root());
    let gitmodules = prj.root().join(".gitmodules");
    let original = r#"[submodule "existing"]
	path = lib/existing
	url = https://github.com/example/existing
"#;
    fs::write(&gitmodules, original).unwrap();
    cmd.git_add();
    cmd.git_commit("add gitmodules");

    let modified = format!("{original}# local edit\n");
    fs::write(&gitmodules, &modified).unwrap();
    let status = || {
        Command::new("git")
            .current_dir(prj.root())
            .args(["status", "--porcelain", "--", ".gitmodules"])
            .output()
            .unwrap()
            .stdout
    };

    let unstaged_status = status();
    let output = cmd
        .forge_fuse()
        .args(["install", "vectorized/solady@this-tag-does-not-exist"])
        .assert_failure();
    assert!(
        output.get_output().stderr_lossy().contains("target or .gitmodules has existing changes")
    );
    assert_eq!(status(), unstaged_status);
    assert_eq!(read_string(&gitmodules), modified);

    cmd.git_add();
    let staged_status = status();
    cmd.forge_fuse()
        .args(["install", "vectorized/solady@this-tag-does-not-exist"])
        .assert_failure();
    assert_eq!(status(), staged_status);
    assert_eq!(read_string(&gitmodules), modified);

    assert!(!prj.root().join("lib/solady").exists());
    assert!(!git.absolute_git_dir().unwrap().join("modules/lib/solady").exists());
    assert!(!git.has_submodule_config(Path::new("lib/solady")).unwrap());
}

#[forgetest]
fn install_rejects_hidden_gitmodules_changes(prj: _, cmd: _) {
    cmd.git_init();
    let gitmodules = prj.root().join(".gitmodules");
    fs::write(
        &gitmodules,
        "[submodule \"existing\"]\n\tpath = lib/existing\n\turl = https://example.com\n",
    )
    .unwrap();
    cmd.git_add();
    cmd.git_commit("add gitmodules");

    fs::write(&gitmodules, "preserve\n").unwrap();
    assert!(
        Command::new("git")
            .current_dir(prj.root())
            .args(["update-index", "--assume-unchanged", ".gitmodules"])
            .output()
            .unwrap()
            .status
            .success()
    );
    let index = fs::read(prj.root().join(".git/index")).unwrap();

    let output = cmd
        .forge_fuse()
        .args(["install", "vectorized/solady@this-tag-does-not-exist"])
        .assert_failure();
    assert!(
        output.get_output().stderr_lossy().contains("target or .gitmodules has existing changes")
    );
    assert_eq!(read_string(&gitmodules), "preserve\n");
    assert_eq!(fs::read(prj.root().join(".git/index")).unwrap(), index);
    assert!(!prj.root().join("lib/solady").exists());
}

#[cfg(unix)]
#[forgetest]
fn install_rejects_dangling_gitmodules_symlink(prj: _, cmd: _) {
    cmd.git_init();
    fs::write(prj.root().join(".gitignore"), ".gitmodules\n").unwrap();
    cmd.git_add();
    cmd.git_commit("ignore gitmodules");

    let gitmodules = prj.root().join(".gitmodules");
    symlink("gitmodules-target", &gitmodules).unwrap();
    let index = fs::read(prj.root().join(".git/index")).unwrap();

    cmd.forge_fuse()
        .args(["install", "vectorized/solady@this-tag-does-not-exist"])
        .assert_failure();

    assert_eq!(fs::read_link(&gitmodules).unwrap(), Path::new("gitmodules-target"));
    assert!(!prj.root().join("gitmodules-target").exists());
    assert_eq!(fs::read(prj.root().join(".git/index")).unwrap(), index);
    assert!(!prj.root().join("lib/solady").exists());
}

#[forgetest]
fn install_rejects_ignored_gitmodules(prj: _, cmd: _) {
    cmd.git_init();
    let git = Git::new(prj.root());
    fs::write(prj.root().join(".gitignore"), ".gitmodules\n").unwrap();
    cmd.git_add();
    cmd.git_commit("ignore gitmodules");

    let gitmodules = prj.root().join(".gitmodules");
    let original = r#"[submodule "existing"]
	path = lib/existing
	url = https://github.com/example/existing
"#;
    fs::write(&gitmodules, original).unwrap();
    assert!(git.is_clean().unwrap());

    let output = cmd
        .forge_fuse()
        .args(["install", "vectorized/solady@this-tag-does-not-exist"])
        .assert_failure();
    assert!(
        output.get_output().stderr_lossy().contains("target or .gitmodules has existing changes")
    );

    assert!(git.is_clean().unwrap());
    assert_eq!(read_string(&gitmodules), original);
    assert!(!prj.root().join("lib/solady").exists());
    assert!(!git.absolute_git_dir().unwrap().join("modules/lib/solady").exists());
    assert!(!git.has_submodule_config(Path::new("lib/solady")).unwrap());
}

#[forgetest]
fn install_rejects_existing_submodule_path(prj: _, cmd: _) {
    cmd.git_init();
    let git = Git::new(prj.root());
    let gitmodules = prj.root().join(".gitmodules");
    let original = r#"[submodule "different-name"]
	path = lib/solady
	url = https://github.com/example/solady
"#;
    fs::write(&gitmodules, original).unwrap();
    cmd.git_add();
    cmd.git_commit("add orphan submodule path");

    let output = cmd
        .forge_fuse()
        .args(["install", "vectorized/solady@this-tag-does-not-exist"])
        .assert_failure();
    assert!(output.get_output().stderr_lossy().contains("already contains a matching submodule"));

    assert!(git.is_clean().unwrap());
    assert_eq!(read_string(&gitmodules), original);
    assert!(!prj.root().join("lib/solady").exists());
    assert!(!git.absolute_git_dir().unwrap().join("modules/lib/solady").exists());
    assert!(!git.has_submodule_config(Path::new("lib/solady")).unwrap());
}

// test to check we can run `forge install` in an empty dir <https://github.com/foundry-rs/foundry/issues/6519>
#[forgetest]
fn can_install_empty(prj: _, cmd: _) {
    // create
    cmd.git_init();
    cmd.forge_fuse().args(["install"]);
    cmd.assert_empty_stdout();

    // create initial commit
    fs::write(prj.root().join("README.md"), "Initial commit").unwrap();

    cmd.git_add();
    cmd.git_commit("Initial commit");

    cmd.forge_fuse().args(["install"]);
    cmd.assert_empty_stdout();
}

// <https://github.com/foundry-rs/foundry/issues/7205>
#[forgetest]
fn install_from_nested_git_repo_uses_project_root(prj: _, cmd: _) {
    prj.update_config(|config| config.libs = vec![PathBuf::from("dependencies")]);
    cmd.git_init();

    let nested = prj.root().join("vendor/dependency");
    fs::create_dir_all(&nested).unwrap();
    Git::new(&nested).init().unwrap();

    cmd.forge_fuse().current_dir(&nested).args(["install", "--no-git"]).assert_success();

    assert!(prj.root().join("dependencies").is_dir());
    assert!(!nested.join("lib").exists());
}

#[forgetest]
fn install_from_nested_foundry_project_uses_nested_root(prj: _, cmd: _) {
    prj.update_config(|config| config.libs = vec![PathBuf::from("outer-dependencies")]);
    cmd.git_init();

    let nested = prj.root().join("vendor/dependency");
    fs::create_dir_all(&nested).unwrap();
    Git::new(&nested).init().unwrap();
    fs::write(
        nested.join(Config::FILE_NAME),
        "[profile.default]\nlibs = [\"nested-dependencies\"]\n",
    )
    .unwrap();

    cmd.forge_fuse().current_dir(&nested).args(["install", "--no-git"]).assert_success();

    assert!(nested.join("nested-dependencies").is_dir());
    assert!(!prj.root().join("outer-dependencies").exists());
}

// test to check that package can be reinstalled after manually removing the directory
#[forgetest]
fn can_reinstall_after_manual_remove(prj: _, cmd: _) {
    cmd.git_init();

    let libs = prj.root().join("lib");
    let git_mod = prj.root().join(".git/modules/lib");
    let git_mod_file = prj.root().join(".gitmodules");

    let forge_std = libs.join("forge-std");
    let forge_std_mod = git_mod.join("forge-std");

    let install = |cmd: &mut TestCommand| {
        cmd.forge_fuse()
            .args(["install", "foundry-rs/forge-std"])
            .assert_success()
            .stdout_eq(str![""])
            .stderr_eq(str![[r#"
Installing forge-std in [..] (url: https://github.com/foundry-rs/forge-std, tag: None)
...
    Installed forge-std tag=[..]
"#]]);

        assert!(forge_std.exists());
        assert!(forge_std_mod.exists());

        let submods = read_string(&git_mod_file);
        assert!(submods.contains("https://github.com/foundry-rs/forge-std"));
    };

    install(&mut cmd);
    let forge_std_lock = lockfile_get(prj.root(), &PathBuf::from("lib/forge-std")).unwrap();
    assert!(matches!(forge_std_lock, DepIdentifier::Tag { .. }));
    fs::remove_dir_all(forge_std.clone()).expect("Failed to remove forge-std");

    // install again with tag
    install(&mut cmd);
    let forge_std_lock = lockfile_get(prj.root(), &PathBuf::from("lib/forge-std")).unwrap();
    assert!(matches!(forge_std_lock, DepIdentifier::Tag { .. }));
}

// https://github.com/foundry-rs/foundry/issues/4353
#[forgetest]
fn can_reinit_submodules(prj: _, cmd: _) {
    cmd.git_init();

    let source = tempfile::tempdir().unwrap();
    let source_git = Git::new(source.path());
    source_git.init().unwrap();
    fs::write(source.path().join("source.txt"), "first revision\n").unwrap();
    source_git.add(["source.txt"]).unwrap();
    source_git.commit("first revision").unwrap();
    let first_rev = source_git.head().unwrap();

    fs::write(source.path().join("source.txt"), "second revision\n").unwrap();
    source_git.add(["source.txt"]).unwrap();
    source_git.commit("second revision").unwrap();
    let second_rev = source_git.head().unwrap();

    let output = Command::new("git")
        .current_dir(prj.root())
        .args(["-c", "protocol.file.allow=always", "submodule", "add", "--"])
        .arg(source.path())
        .arg("lib/dep")
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));

    let dependency = prj.root().join("lib/dep");
    let dependency_git = Git::new(&dependency);
    dependency_git.checkout(false, &first_rev).unwrap();
    cmd.git_add();
    cmd.git_commit("add dependency");

    dependency_git.checkout(false, &second_rev).unwrap();
    Git::new(prj.root()).add(["lib/dep"]).unwrap();
    cmd.git_commit("advance dependency");

    dependency_git.checkout(false, &first_rev).unwrap();
    fs::write(dependency.join("source.txt"), "local edit\n").unwrap();

    cmd.forge_fuse();
    cmd.env("GIT_ALLOW_PROTOCOL", "file");
    cmd.arg("reinit").assert_success();
    assert_eq!(dependency_git.head().unwrap(), second_rev);
    assert_eq!(
        read_string(dependency.join("source.txt")).replace("\r\n", "\n"),
        "second revision\n"
    );
}

// test that we can repeatedly install the same dependency without changes
#[forgetest]
fn can_install_repeatedly(cmd: _) {
    cmd.git_init();

    cmd.forge_fuse().args(["install", "foundry-rs/forge-std"]);
    for _ in 0..3 {
        cmd.assert_success();
    }
}

#[forgetest]
fn can_install_multiple_submodules(cmd: _) {
    cmd.git_init();
    cmd.forge_fuse()
        .args(["install", "foundry-rs/forge-std", "vectorized/solady"])
        .assert_success();
}

// test that by default we install the latest semver release tag
// <https://github.com/openzeppelin/openzeppelin-contracts>
#[forgetest]
fn can_install_latest_release_tag(prj: _, cmd: _) {
    cmd.git_init();
    cmd.forge_fuse().args(["install", "openzeppelin/openzeppelin-contracts"]);
    cmd.assert_success();

    let dep = prj.paths().libraries[0].join("openzeppelin-contracts");
    assert!(dep.exists());

    let oz_lock = lockfile_get(prj.root(), &PathBuf::from("lib/openzeppelin-contracts")).unwrap();
    assert!(matches!(oz_lock, DepIdentifier::Tag { .. }));

    // the latest release at the time this test was written
    let version: Version = "4.8.0".parse().unwrap();
    let out = Command::new("git").current_dir(&dep).args(["describe", "--tags"]).output().unwrap();
    let tag = String::from_utf8_lossy(&out.stdout);
    let current: Version = tag.as_ref().trim_start_matches('v').trim().parse().unwrap();

    assert!(current >= version);
}

#[forgetest]
fn can_update_and_retain_tag_revs(prj: _, cmd: _) {
    cmd.git_init();

    // Installs oz at release tag
    cmd.forge_fuse()
        .args(["install", "openzeppelin/openzeppelin-contracts@v5.1.0"])
        .assert_success();

    // Install solady pinned to rev i.e https://github.com/Vectorized/solady/commit/513f581675374706dbe947284d6b12d19ce35a2a
    cmd.forge_fuse().args(["install", "vectorized/solady@513f581"]).assert_success();

    let out = cmd.git_submodule_status();
    let status = String::from_utf8_lossy(&out.stdout);
    let oz_init = lockfile_get(prj.root(), &PathBuf::from("lib/openzeppelin-contracts")).unwrap();
    let solady_init = lockfile_get(prj.root(), &PathBuf::from("lib/solady")).unwrap();
    assert_eq!(oz_init.name(), "v5.1.0");
    assert_eq!(solady_init.rev(), "513f581");
    let submodules_init: Submodules = status.parse().unwrap();

    cmd.forge_fuse().arg("update").assert_success();

    let out = cmd.git_submodule_status();
    let status = String::from_utf8_lossy(&out.stdout);
    let submodules_update: Submodules = status.parse().unwrap();
    assert_eq!(submodules_init, submodules_update);

    let oz_update = lockfile_get(prj.root(), &PathBuf::from("lib/openzeppelin-contracts")).unwrap();
    let solady_update = lockfile_get(prj.root(), &PathBuf::from("lib/solady")).unwrap();
    assert_eq!(oz_init, oz_update);
    assert_eq!(solady_init, solady_update);
}

#[forgetest]
fn update_rejects_lockfile_paths_outside_submodules(prj: _, cmd: _) {
    cmd.git_init();
    let parent_git = Git::new(prj.root());
    let state = prj.root().join("parent-state.txt");
    fs::write(&state, "old\n").unwrap();
    parent_git.add(["parent-state.txt"]).unwrap();
    parent_git.commit("old").unwrap();
    fs::write(&state, "new\n").unwrap();
    parent_git.add(["parent-state.txt"]).unwrap();
    parent_git.commit("new").unwrap();
    let parent_head = parent_git.head().unwrap();

    let project = prj.root().join("project");
    fs::create_dir(&project).unwrap();
    let project_git = Git::new(&project);
    project_git.init().unwrap();
    fs::write(project.join("foundry.toml"), "[profile.default]\n").unwrap();
    fs::write(project.join("foundry.lock"), r#"{"..":{"rev":"HEAD^"}}"#).unwrap();

    let dependency = tempfile::tempdir().unwrap();
    let dependency_git = Git::new(dependency.path());
    dependency_git.init().unwrap();
    fs::write(dependency.path().join("file"), "content\n").unwrap();
    dependency_git.add(["file"]).unwrap();
    dependency_git.commit("initial").unwrap();
    let dependency_rev = dependency_git.head().unwrap();
    // Windows Git rejects newlines in paths; retain the spoofed status entry on Unix.
    let submodule_path = if cfg!(windows) {
        "lib/decoy"
    } else {
        "lib/decoy\n0123456789012345678901234567890123456789 .."
    };
    fs::write(
        project.join(".gitmodules"),
        format!(
            "[submodule \"decoy\"]\n\tpath = \"{}\"\n\turl = {}\n",
            submodule_path.replace('\n', "\\n"),
            Url::from_file_path(dependency.path()).unwrap()
        ),
    )
    .unwrap();
    project_git.add([".gitmodules"]).unwrap();
    let output = Command::new("git")
        .current_dir(&project)
        .args(["update-index", "--add", "--cacheinfo"])
        .arg(format!("160000,{dependency_rev},{submodule_path}"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));

    cmd.forge_fuse()
        .arg("update")
        .arg("--root")
        .arg(&project)
        .assert_failure()
        .stdout_eq(str![""])
        .stderr_eq(str![[r#"
Error: foundry.lock entry `..` does not match an installed Git submodule

"#]]);

    assert_eq!(parent_git.head().unwrap(), parent_head);
    assert_eq!(fs::read_to_string(state).unwrap(), "new\n");
}

#[forgetest]
fn update_rejects_uninitialized_submodule_worktrees(prj: _, cmd: _) {
    cmd.git_init();
    let git = Git::new(prj.root());

    let dependency = tempfile::tempdir().unwrap();
    let dependency_git = Git::new(dependency.path());
    dependency_git.init().unwrap();
    fs::write(dependency.path().join("file"), "content\n").unwrap();
    dependency_git.add(["file"]).unwrap();
    dependency_git.commit("initial").unwrap();
    let dependency_rev = dependency_git.head().unwrap();

    fs::write(
        prj.root().join(".gitmodules"),
        format!(
            "[submodule \"lib/skipped\"]\n\tpath = lib/skipped\n\turl = {}\n\tupdate = none\n",
            Url::from_file_path(dependency.path()).unwrap()
        ),
    )
    .unwrap();
    fs::write(prj.root().join("parent-state.txt"), "old\n").unwrap();
    git.add([".gitmodules", "parent-state.txt"]).unwrap();
    let output = Command::new("git")
        .current_dir(prj.root())
        .args(["update-index", "--add", "--cacheinfo"])
        .arg(format!("160000,{dependency_rev},lib/skipped"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    git.commit("old").unwrap();
    fs::write(prj.root().join("parent-state.txt"), "new\n").unwrap();
    git.add(["parent-state.txt"]).unwrap();
    git.commit("new").unwrap();
    let parent_head = git.head().unwrap();

    fs::create_dir_all(prj.root().join("lib/skipped")).unwrap();
    fs::write(prj.root().join("foundry.lock"), r#"{"lib/skipped":{"rev":"HEAD^"}}"#).unwrap();

    cmd.forge_fuse().arg("update").assert_failure().stdout_eq(str![""]).stderr_eq(str![[r#"
Submodule 'lib/skipped' ([..]) registered for path 'lib/skipped'
Skipping submodule 'lib/skipped'
Error: Dependency at `lib/skipped` is not an initialized Git submodule worktree

"#]]);

    assert_eq!(git.head().unwrap(), parent_head);
    assert_eq!(fs::read_to_string(prj.root().join("parent-state.txt")).unwrap(), "new\n");
}

#[forgetest]
fn update_initializes_uninitialized_submodule_worktrees(prj: _, cmd: _) {
    cmd.git_init();
    let dependency = tempfile::tempdir().unwrap();
    let dependency_git = Git::new(dependency.path());
    dependency_git.init().unwrap();
    fs::write(dependency.path().join("file"), "content\n").unwrap();
    dependency_git.add(["file"]).unwrap();
    dependency_git.commit("initial").unwrap();
    let dependency_rev = dependency_git.head().unwrap();

    let output = Command::new("git")
        .current_dir(prj.root())
        .args(["-c", "protocol.file.allow=always", "submodule", "add", "--"])
        .arg(dependency.path())
        .arg("lib/dep")
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let mut lock = Lockfile::new(prj.root());
    lock.insert(
        PathBuf::from("lib/dep"),
        DepIdentifier::Rev { rev: dependency_rev.clone(), r#override: false },
    );
    lock.write().unwrap();
    cmd.git_add();
    cmd.git_commit("add dependency");

    let output = Command::new("git")
        .current_dir(prj.root())
        .args(["submodule", "deinit", "--force", "--", "lib/dep"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));

    cmd.forge_fuse();
    cmd.env("GIT_ALLOW_PROTOCOL", "file");
    cmd.arg("update").assert_success();
    let dependency_path = prj.root().join("lib/dep");
    let dependency_git = Git::new(&dependency_path);
    assert!(dependency_git.is_repo_root().unwrap());
    assert_eq!(dependency_git.head().unwrap(), dependency_rev);
}

#[forgetest]
fn can_update_only_selected_dependencies(prj: _, cmd: _) {
    cmd.git_init();

    let source = tempfile::tempdir().unwrap();
    let source_git = Git::new(source.path());
    source_git.init().unwrap();
    fs::write(source.path().join("source.txt"), "first revision\n").unwrap();
    source_git.add(["source.txt"]).unwrap();
    source_git.commit("first revision").unwrap();
    let (first, branch) = source_git.current_rev_branch(source.path()).unwrap();
    fs::write(source.path().join("source.txt"), "second revision\n").unwrap();
    source_git.add(["source.txt"]).unwrap();
    source_git.commit("second revision").unwrap();
    let second = source_git.head().unwrap();

    let mut lock = Lockfile::new(prj.root());
    for name in ["dep-a", "dep-b", "dep-c", "dep-pin"] {
        let path = PathBuf::from(format!("lib/{name}"));
        let output = Command::new("git")
            .current_dir(prj.root())
            .args(["-c", "protocol.file.allow=always", "submodule", "add", "--"])
            .arg(source.path())
            .arg(&path)
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        Git::new(&prj.root().join(&path)).checkout(false, &first).unwrap();
        let dep = if name == "dep-pin" {
            DepIdentifier::Rev { rev: first.clone(), r#override: false }
        } else {
            Git::new(prj.root()).set_submodule_branch(&path, &branch).unwrap();
            DepIdentifier::Branch { name: branch.clone(), rev: first.clone(), r#override: false }
        };
        lock.insert(path, dep);
    }
    lock.write().unwrap();
    cmd.git_add();
    cmd.git_commit("pin dependency fixtures");

    let assert_revisions = |expected: [&str; 4]| {
        for (name, rev) in ["dep-a", "dep-b", "dep-c", "dep-pin"].into_iter().zip(expected) {
            let path = PathBuf::from(format!("lib/{name}"));
            assert_eq!(Git::new(&prj.root().join(&path)).head().unwrap(), rev, "{name}");
            assert_eq!(lockfile_get(prj.root(), &path).unwrap().rev(), rev, "{name}");
        }
    };

    // A pinned selection must not become Git's empty-path update of every dependency.
    cmd.forge_fuse();
    cmd.env("GIT_ALLOW_PROTOCOL", "file");
    cmd.args(["update", "dep-pin"]).assert_success();
    assert_revisions([&first, &first, &first, &first]);

    cmd.forge_fuse();
    cmd.env("GIT_ALLOW_PROTOCOL", "file");
    cmd.args(["update", "dep-a"]).assert_success();
    assert_revisions([&second, &first, &first, &first]);

    // Unqualified branches must still update when another selection has an explicit ref.
    cmd.forge_fuse();
    cmd.env("GIT_ALLOW_PROTOCOL", "file");
    cmd.args(["update", "dep-b", &format!("fixture/dep-pin@{second}")]).assert_success();
    assert_revisions([&second, &second, &first, &second]);

    cmd.forge_fuse();
    cmd.env("GIT_ALLOW_PROTOCOL", "file");
    cmd.arg("update").assert_success();
    assert_revisions([&second, &second, &second, &second]);
}

#[forgetest]
fn can_override_tag_in_update(prj: _, cmd: _) {
    cmd.git_init();

    // Installs oz at release tag
    cmd.forge_fuse()
        .args(["install", "openzeppelin/openzeppelin-contracts@v5.0.2"])
        .assert_success();

    cmd.forge_fuse().args(["install", "vectorized/solady@513f581"]).assert_success();

    let out = cmd.git_submodule_status();
    let status = String::from_utf8_lossy(&out.stdout);

    let submodules_init: Submodules = status.parse().unwrap();

    let oz_init_lock =
        lockfile_get(prj.root(), &PathBuf::from("lib/openzeppelin-contracts")).unwrap();
    assert_eq!(oz_init_lock.name(), "v5.0.2");
    let solady_init_lock = lockfile_get(prj.root(), &PathBuf::from("lib/solady")).unwrap();
    assert_eq!(solady_init_lock.rev(), "513f581");

    // Update oz to a different release tag
    cmd.forge_fuse()
        .args(["update", "openzeppelin/openzeppelin-contracts@v5.1.0"])
        .assert_success();

    let out = cmd.git_submodule_status();
    let status = String::from_utf8_lossy(&out.stdout);

    let submodules_update: Submodules = status.parse().unwrap();

    assert_ne!(submodules_init.0[0], submodules_update.0[0]);
    assert_eq!(submodules_init.0[1], submodules_update.0[1]);

    let oz_update_lock =
        lockfile_get(prj.root(), &PathBuf::from("lib/openzeppelin-contracts")).unwrap();
    let solady_update_lock = lockfile_get(prj.root(), &PathBuf::from("lib/solady")).unwrap();

    assert_ne!(oz_init_lock, oz_update_lock);
    assert_eq!(oz_update_lock.name(), "v5.1.0");
    assert_eq!(submodules_update.0[0].rev(), oz_update_lock.rev());
    assert_eq!(solady_init_lock, solady_update_lock);
}

// Ref: https://github.com/foundry-rs/foundry/pull/9522#pullrequestreview-2494431518
#[forgetest]
fn should_not_update_tagged_deps(prj: _, cmd: _) {
    cmd.git_init();

    // Installs oz at release tag
    cmd.forge_fuse()
        .args(["install", "openzeppelin/openzeppelin-contracts@tag=v4.9.4"])
        .assert_success();

    let out = cmd.git_submodule_status();
    let status = String::from_utf8_lossy(&out.stdout);
    let submodules_init: Submodules = status.parse().unwrap();

    let oz_init = lockfile_get(prj.root(), &PathBuf::from("lib/openzeppelin-contracts")).unwrap();

    cmd.forge_fuse().arg("update").assert_success();

    let out = cmd.git_submodule_status();
    let status = String::from_utf8_lossy(&out.stdout);
    let submodules_update: Submodules = status.parse().unwrap();

    assert_eq!(submodules_init, submodules_update);

    let oz_update = lockfile_get(prj.root(), &PathBuf::from("lib/openzeppelin-contracts")).unwrap();

    assert_eq!(oz_init, oz_update);
    // Check that halmos-cheatcodes dep is not added to oz deps
    let halmos_path = prj.paths().libraries[0].join("openzeppelin-contracts/lib/halmos-cheatcodes");

    assert!(!halmos_path.exists());
}

#[forgetest]
fn can_remove_dep_from_foundry_lock(prj: _, cmd: _) {
    cmd.git_init();

    cmd.forge_fuse()
        .args(["install", "openzeppelin/openzeppelin-contracts@tag=v4.9.4"])
        .assert_success();

    cmd.forge_fuse().args(["install", "vectorized/solady@513f581"]).assert_success();
    cmd.forge_fuse().args(["remove", "openzeppelin-contracts", "--force"]).assert_success();

    let mut lock = Lockfile::new(prj.root());

    lock.read().unwrap();

    assert!(lock.get(&PathBuf::from("lib/openzeppelin-contracts")).is_none());
}

#[forgetest]
#[cfg_attr(windows, ignore = "weird git fail")]
fn can_sync_foundry_lock(prj: _, cmd: _) {
    cmd.git_init();

    cmd.forge_fuse().args(["install", "foundry-rs/forge-std@master"]).assert_success();

    cmd.forge_fuse().args(["install", "vectorized/solady"]).assert_success();

    fs::remove_file(prj.root().join("foundry.lock")).unwrap();

    // sync submodules and write foundry.lock
    cmd.forge_fuse().arg("install").assert_success();

    let mut lock = forge::Lockfile::new(prj.root());
    lock.read().unwrap();

    assert!(matches!(
        lock.get(&PathBuf::from("lib/forge-std")).unwrap(),
        &DepIdentifier::Branch { .. }
    ));
    assert!(matches!(lock.get(&PathBuf::from("lib/solady")).unwrap(), &DepIdentifier::Rev { .. }));
}

// Tests that forge update doesn't break a working dependency by recursively updating nested
// dependencies
#[forgetest]
#[cfg_attr(windows, ignore = "weird git fail")]
fn can_update_library_with_outdated_nested_dependency(prj: _, cmd: _) {
    cmd.git_init();

    let libs = prj.root().join("lib");
    let git_mod = prj.root().join(".git/modules/lib");
    let git_mod_file = prj.root().join(".gitmodules");

    // get paths to check inside install fn
    let package = libs.join("forge-5980-test");
    let package_mod = git_mod.join("forge-5980-test");

    // install main dependency
    cmd.forge_fuse()
        .args(["install", "evalir/forge-5980-test"])
        .assert_success()
        .stdout_eq(str![""])
        .stderr_eq(str![[r#"
Installing forge-5980-test in [..] (url: https://github.com/evalir/forge-5980-test, tag: None)
...
    Installed forge-5980-test

"#]]);

    // assert paths exist
    assert!(package.exists());
    assert!(package_mod.exists());

    let submods = read_string(git_mod_file);
    assert!(submods.contains("https://github.com/evalir/forge-5980-test"));

    // try to update the top-level dependency; there should be no update for this dependency,
    // but its sub-dependency has upstream (breaking) changes; forge should not attempt to
    // update the sub-dependency
    cmd.forge_fuse().args(["update", "lib/forge-5980-test"]).assert_empty_stdout();

    // add explicit remappings for test file
    let config = Config {
        remappings: vec![
            Remapping::from_str("forge-5980-test/=lib/forge-5980-test/src/").unwrap().into(),
            // explicit remapping for sub-dependency seems necessary for some reason
            Remapping::from_str(
                "forge-5980-test-dep/=lib/forge-5980-test/lib/forge-5980-test-dep/src/",
            )
            .unwrap()
            .into(),
        ],
        ..Default::default()
    };
    prj.write_config(config);

    // create test file that uses the top-level dependency; if the sub-dependency is updated,
    // compilation will fail
    prj.add_source(
        "CounterCopy",
        r#"
import "forge-5980-test/Counter.sol";
contract CounterCopy is Counter {
}
   "#,
    );

    // build and check output
    cmd.forge_fuse().arg("build").assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

"#]]);
}

#[tokio::test]
async fn uni_v4_core_sync_foundry_lock() {
    let (prj, mut cmd) =
        ExtTester::new("Uniswap", "v4-core", "e50237c43811bd9b526eff40f26772152a42daba")
            .setup_forge_prj(true);

    assert!(!prj.root().join(FOUNDRY_LOCK).exists());

    let git = Git::new(prj.root());

    let submodules = git.submodules().unwrap();

    let submod_forge_std =
        submodules.into_iter().find(|s| s.path() == &PathBuf::from("lib/forge-std")).unwrap();
    let submod_oz = submodules
        .into_iter()
        .find(|s| s.path() == &PathBuf::from("lib/openzeppelin-contracts"))
        .unwrap();
    let submod_solmate =
        submodules.into_iter().find(|s| s.path() == &PathBuf::from("lib/solmate")).unwrap();

    cmd.arg("install").assert_success();

    let forge_std = lockfile_get(prj.root(), &PathBuf::from("lib/forge-std")).unwrap();
    assert!(matches!(forge_std, DepIdentifier::Rev { .. }));
    assert_eq!(forge_std.rev(), submod_forge_std.rev());
    let solmate = lockfile_get(prj.root(), &PathBuf::from("lib/solmate")).unwrap();
    assert!(matches!(solmate, DepIdentifier::Rev { .. }));
    assert_eq!(solmate.rev(), submod_solmate.rev());
    let oz = lockfile_get(prj.root(), &PathBuf::from("lib/openzeppelin-contracts")).unwrap();
    assert!(matches!(oz, DepIdentifier::Rev { .. }));
    assert_eq!(oz.rev(), submod_oz.rev());

    // Commit the lockfile
    git.add(&PathBuf::from(FOUNDRY_LOCK)).unwrap();
    git.commit("Foundry lock").unwrap();

    // Try update. Nothing should get updated everything is pinned tag/rev.
    cmd.forge_fuse().arg("update").assert_success();

    let forge_std = lockfile_get(prj.root(), &PathBuf::from("lib/forge-std")).unwrap();
    assert!(matches!(forge_std, DepIdentifier::Rev { .. }));
    assert_eq!(forge_std.rev(), submod_forge_std.rev());
    let solmate = lockfile_get(prj.root(), &PathBuf::from("lib/solmate")).unwrap();
    assert!(matches!(solmate, DepIdentifier::Rev { .. }));
    assert_eq!(solmate.rev(), submod_solmate.rev());
    let oz = lockfile_get(prj.root(), &PathBuf::from("lib/openzeppelin-contracts")).unwrap();
    assert!(matches!(oz, DepIdentifier::Rev { .. }));
    assert_eq!(oz.rev(), submod_oz.rev());
}

#[tokio::test]
async fn oz_contracts_sync_foundry_lock() {
    let (prj, mut cmd) = ExtTester::new(
        "OpenZeppelin",
        "openzeppelin-contracts",
        "840c974028316f3c8172c1b8e5ed67ad95e255ca",
    )
    .setup_forge_prj(true);

    assert!(!prj.root().join(FOUNDRY_LOCK).exists());

    let git = Git::new(prj.root());

    let submodules = git.submodules().unwrap();

    let submod_forge_std =
        submodules.into_iter().find(|s| s.path() == &PathBuf::from("lib/forge-std")).unwrap();
    let submod_erc4626_tests =
        submodules.into_iter().find(|s| s.path() == &PathBuf::from("lib/erc4626-tests")).unwrap();
    let submod_halmos = submodules
        .into_iter()
        .find(|s| s.path() == &PathBuf::from("lib/halmos-cheatcodes"))
        .unwrap();

    cmd.arg("install").assert_success();

    let forge_std = lockfile_get(prj.root(), &PathBuf::from("lib/forge-std")).unwrap();
    assert!(forge_std.is_branch());
    assert_eq!(forge_std.rev(), submod_forge_std.rev());
    assert_eq!(forge_std.name(), "v1");
    let erc4626_tests = lockfile_get(prj.root(), &PathBuf::from("lib/erc4626-tests")).unwrap();
    assert!(matches!(erc4626_tests, DepIdentifier::Rev { .. }));
    assert_eq!(erc4626_tests.rev(), submod_erc4626_tests.rev());
    let halmos = lockfile_get(prj.root(), &PathBuf::from("lib/halmos-cheatcodes")).unwrap();
    assert!(matches!(halmos, DepIdentifier::Rev { .. }));
    assert_eq!(halmos.rev(), submod_halmos.rev());

    // Commit the lockfile
    git.add(&PathBuf::from(FOUNDRY_LOCK)).unwrap();
    git.commit("Foundry lock").unwrap();

    // Try update. forge-std should get updated, rest should remain the same.
    cmd.forge_fuse().arg("update").assert_success();

    let forge_std = lockfile_get(prj.root(), &PathBuf::from("lib/forge-std")).unwrap();
    assert!(forge_std.is_branch());
    // assert_eq!(forge_std.rev(), submod_forge_std.rev());  // This can fail, as forge-std will get
    // updated to the latest commit on master.
    assert_eq!(forge_std.name(), "v1"); // But it stays locked on the same master
    let erc4626_tests = lockfile_get(prj.root(), &PathBuf::from("lib/erc4626-tests")).unwrap();
    assert!(matches!(erc4626_tests, DepIdentifier::Rev { .. }));
    assert_eq!(erc4626_tests.rev(), submod_erc4626_tests.rev());
    let halmos = lockfile_get(prj.root(), &PathBuf::from("lib/halmos-cheatcodes")).unwrap();
    assert!(matches!(halmos, DepIdentifier::Rev { .. }));
    assert_eq!(halmos.rev(), submod_halmos.rev());
}

#[tokio::test]
async fn correctly_sync_dep_with_multiple_version() {
    let (prj, mut cmd) = ExtTester::new(
        "yash-atreya",
        "sync-lockfile-multi-version-dep",
        "1ca47e73a168e54f8f7761862dbd0c603856c5c8",
    )
    .setup_forge_prj(true);

    assert!(!prj.root().join(FOUNDRY_LOCK).exists());

    let git = Git::new(prj.root());

    let submodules = git.submodules().unwrap();
    let submod_forge_std =
        submodules.into_iter().find(|s| s.path() == &PathBuf::from("lib/forge-std")).unwrap();
    let submod_solady =
        submodules.into_iter().find(|s| s.path() == &PathBuf::from("lib/solady")).unwrap();
    let submod_solday_v_245 =
        submodules.into_iter().find(|s| s.path() == &PathBuf::from("lib/solady-v0.0.245")).unwrap();

    cmd.arg("install").assert_success();

    let forge_std = lockfile_get(prj.root(), &PathBuf::from("lib/forge-std")).unwrap();
    assert!(matches!(forge_std, DepIdentifier::Rev { .. }));
    assert_eq!(forge_std.rev(), submod_forge_std.rev());

    let solady = lockfile_get(prj.root(), &PathBuf::from("lib/solady")).unwrap();
    assert!(matches!(solady, DepIdentifier::Rev { .. }));
    assert_eq!(solady.rev(), submod_solady.rev());

    let solday_v_245 = lockfile_get(prj.root(), &PathBuf::from("lib/solady-v0.0.245")).unwrap();
    assert!(matches!(solday_v_245, DepIdentifier::Rev { .. }));
    assert_eq!(solday_v_245.rev(), submod_solday_v_245.rev());
}

// Regression test: `forge install --no-git` should clean up nested submodule contents
// when installing a tag that does not use submodules for its dependencies.
// https://github.com/foundry-rs/foundry/issues/13688
#[forgetest]
fn flaky_install_no_git_cleans_nested_submodules(prj: _, cmd: _) {
    cmd.git_init();

    // Install openzeppelin-contracts-upgradeable at v4.7.3 with --no-git.
    // The default branch has submodules in lib/ (e.g. openzeppelin-contracts, erc4626-tests),
    // but v4.7.3 does not use submodules for dependencies.
    cmd.forge_fuse()
        .args(["install", "--no-git", "OpenZeppelin/openzeppelin-contracts-upgradeable@v4.7.3"])
        .assert_success();

    let dep_dir = prj.root().join("lib").join("openzeppelin-contracts-upgradeable");
    assert!(dep_dir.exists(), "dependency should be installed");

    // The nested lib/ directory should either not exist or be empty — v4.7.3 does not use
    // submodules so there should be no leftover submodule contents from the default branch.
    let nested_lib = dep_dir.join("lib");
    if nested_lib.exists() {
        let entries: Vec<_> = fs::read_dir(&nested_lib).unwrap().collect();
        assert!(
            entries.is_empty(),
            "nested lib/ should be empty after --no-git install at v4.7.3, found: {entries:?}"
        );
    }

    // There should be no .git file or directory anywhere under the installed dependency.
    fn assert_no_git(dir: &Path) {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            assert!(
                path.file_name() != Some(".git".as_ref()),
                "found leftover .git at {}",
                path.display()
            );
            if path.is_dir() {
                assert_no_git(&path);
            }
        }
    }
    assert_no_git(&dep_dir);
}

#[forgetest_init]
fn sync_on_forge_update(prj: _, cmd: _) {
    let git = Git::new(prj.root());

    let submodules = git.submodules().unwrap();
    assert!(submodules.0.iter().any(|s| s.rev() == FORGE_STD_REVISION));

    let mut lockfile = Lockfile::new(prj.root());
    lockfile.read().unwrap();

    let forge_std = lockfile.get(&PathBuf::from("lib/forge-std")).unwrap();
    assert!(forge_std.rev() == FORGE_STD_REVISION);

    // cd into the forge-std submodule
    let forge_std_path = prj.root().join("lib/forge-std");
    let git = Git::new(&forge_std_path);

    // Ensure we're on the release tag first (known starting point)
    git.checkout(false, forge_std.name()).unwrap();
    assert_eq!(git.head().unwrap(), forge_std.rev(), "Forge std should be at the release tag");

    // Make sure origin/master is up to date, then resolve its commit hash deterministically.
    git.fetch(false, "origin", Some("master")).unwrap();
    let origin_master_head = git.get_rev("refs/remotes/origin/master", &forge_std_path).unwrap();

    // Run update and assert the output matches the dynamically resolved hash.
    let expected_output = format!(
        "Updated dep at 'lib/forge-std', (from: tag={}@{}, to: branch=master@{})\n",
        forge_std.name(),
        forge_std.rev(),
        origin_master_head
    );

    cmd.forge_fuse()
        .args(["update", "foundry-rs/forge-std@master"])
        .assert_success()
        .stdout_eq(str![""])
        .stderr_eq(expected_output);

    let git = Git::new(&forge_std_path);
    assert_eq!(
        git.head().unwrap(),
        origin_master_head,
        "Submodule HEAD should match resolved origin/master after update"
    );

    let root_git = Git::new(prj.root());
    let submodules_after = root_git.submodules().unwrap();
    let forge_sm = submodules_after
        .0
        .iter()
        .find(|s| s.path().as_path() == Path::new("lib/forge-std"))
        .expect("forge-std submodule should exist");
    assert_eq!(
        forge_sm.rev(),
        origin_master_head,
        "Root submodule status should match resolved origin/master after update"
    );

    let mut lockfile = Lockfile::new(prj.root());
    lockfile.read().unwrap();
    let forge_std_after = lockfile.get(&PathBuf::from("lib/forge-std")).unwrap();
    assert_eq!(
        forge_std_after.rev(),
        origin_master_head,
        "Lockfile rev should match resolved origin/master after update"
    );
}

// Checks that `--no-commit` is accepted as a noop backwards-compatibility flag
#[forgetest_init]
fn can_install_with_no_commit(cmd: _) {
    cmd.args(["install", "--no-commit"]).assert_success();
}

#[forgetest]
fn install_no_git_cleans_failed_recursive_clone_and_retries(prj: _, cmd: _) {
    let repositories = tempfile::tempdir().unwrap();
    let parent = repositories.path().join("parent");
    let child = repositories.path().join("child");
    for path in [&parent, &child] {
        init_local_install_source(path);
    }
    let output = Command::new("git")
        .current_dir(&parent)
        .args(["-c", "protocol.file.allow=always", "submodule", "add", "--"])
        .arg(&child)
        .arg("lib/child")
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    Git::new(&parent).commit("add child").unwrap();

    let unavailable_child = repositories.path().join("unavailable-child");
    fs::rename(&child, &unavailable_child).unwrap();
    let sibling = prj.root().join("lib/existing/source.txt");
    fs::create_dir_all(sibling.parent().unwrap()).unwrap();
    fs::write(&sibling, "existing dependency\n").unwrap();

    configure_local_install(&mut cmd, &parent);
    cmd.args(["install", "--no-git", "fixture/parent"])
        .assert_failure()
        .stdout_eq(str![""])
        .stderr_eq(str![[r#"
Installing parent in [..] (url: https://github.com/fixture/parent, tag: None)
Cloning into '[..]'...
...
Failed to clone 'lib/child' a second time, aborting
Error: git clone exited with code 1

"#]]);

    let installed = prj.root().join("lib/parent");
    assert!(!installed.exists(), "failed installation must remove the partial dependency");
    assert_eq!(fs::read_to_string(&sibling).unwrap(), "existing dependency\n");

    fs::rename(&unavailable_child, &child).unwrap();
    cmd.assert_success();

    for path in [&installed, &installed.join("lib/child")] {
        assert_eq!(
            fs::read_to_string(path.join("source.txt")).unwrap().replace("\r\n", "\n"),
            "dependency source\n"
        );
        assert!(!path.join(".git").exists(), "successful installation must remove Git artifacts");
    }
    assert_eq!(fs::read_to_string(sibling).unwrap(), "existing dependency\n");
}

#[forgetest]
fn install_no_git_preserves_existing_targets(prj: _, cmd: _) {
    let source = tempfile::tempdir().unwrap();
    init_local_install_source(source.path());
    let lib = prj.root().join("lib");
    fs::create_dir_all(&lib).unwrap();
    fs::create_dir(lib.join("empty")).unwrap();
    fs::create_dir(lib.join("nonempty")).unwrap();
    fs::write(lib.join("nonempty/source.txt"), "existing dependency\n").unwrap();
    fs::write(lib.join("file"), "existing file\n").unwrap();

    for name in ["empty", "nonempty", "file"] {
        configure_local_install(&mut cmd, source.path());
        cmd.args(["install", "--no-git", &format!("{name}=fixture/parent")])
            .assert_failure()
            .stdout_eq(str![""])
            .stderr_eq(str![[r#"
Installing parent in [..] (url: https://github.com/fixture/parent, tag: None)
Error: failed to create dir "[..]": [..]

"#]]);
    }

    assert_eq!(fs::read_dir(lib.join("empty")).unwrap().count(), 0);
    assert_eq!(fs::read_dir(lib.join("nonempty")).unwrap().count(), 1);
    assert_eq!(
        fs::read_to_string(lib.join("nonempty/source.txt")).unwrap(),
        "existing dependency\n"
    );
    assert_eq!(fs::read_to_string(lib.join("file")).unwrap(), "existing file\n");
}

#[cfg(unix)]
#[forgetest]
fn install_no_git_preserves_existing_symlinks(prj: _, cmd: _) {
    let source = tempfile::tempdir().unwrap();
    init_local_install_source(source.path());
    let targets = tempfile::tempdir().unwrap();
    fs::create_dir(targets.path().join("empty")).unwrap();
    fs::create_dir(targets.path().join("nonempty")).unwrap();
    fs::write(targets.path().join("nonempty/source.txt"), "existing dependency\n").unwrap();
    let lib = prj.root().join("lib");
    fs::create_dir_all(&lib).unwrap();

    for name in ["empty", "nonempty", "dangling"] {
        let target = targets.path().join(name);
        let link = lib.join(name);
        symlink(&target, &link).unwrap();
        configure_local_install(&mut cmd, source.path());
        cmd.args(["install", "--no-git", &format!("{name}=fixture/parent")])
            .assert_failure()
            .stdout_eq(str![""])
            .stderr_eq(str![[r#"
Installing parent in [..] (url: https://github.com/fixture/parent, tag: None)
Error: failed to create dir "[..]": [..]

"#]]);
        assert_eq!(fs::read_link(link).unwrap(), target);
    }

    assert_eq!(fs::read_dir(targets.path().join("empty")).unwrap().count(), 0);
    assert_eq!(fs::read_dir(targets.path().join("nonempty")).unwrap().count(), 1);
    assert_eq!(
        fs::read_to_string(targets.path().join("nonempty/source.txt")).unwrap(),
        "existing dependency\n"
    );
    assert!(!targets.path().join("dangling").exists());
}

#[forgetest]
fn install_no_git_cleans_failed_checkout_with_nested_alias(prj: _, cmd: _) {
    let source = tempfile::tempdir().unwrap();
    init_local_install_source(source.path());
    configure_local_install(&mut cmd, source.path());
    cmd.args(["install", "--no-git", "nested/parent=fixture/parent@missing-tag"])
        .assert_failure()
        .stdout_eq(str![""])
        .stderr_eq(str![[r#"
Installing parent in [..] (url: https://github.com/fixture/parent, tag: missing-tag)
...
Error: Tag: "missing-tag" not found for repo "https://github.com/fixture/parent"!

"#]]);

    let installed = prj.root().join("lib/nested/parent");
    assert!(!installed.exists(), "failed checkout must remove the dependency");

    configure_local_install(&mut cmd, source.path());
    cmd.args(["install", "--no-git", "nested/parent=fixture/parent"]).assert_success();
    assert_eq!(
        fs::read_to_string(installed.join("source.txt")).unwrap().replace("\r\n", "\n"),
        "dependency source\n"
    );
    assert!(!installed.join(".git").exists());
}

#[forgetest]
fn install_no_git_cleans_failed_initial_clone(prj: _, cmd: _) {
    let source = tempfile::tempdir().unwrap();
    configure_local_install(&mut cmd, &source.path().join("missing"));
    cmd.args(["install", "--no-git", "fixture/parent"])
        .assert_failure()
        .stdout_eq(str![""])
        .stderr_eq(str![[r#"
Installing parent in [..] (url: https://github.com/fixture/parent, tag: None)
Cloning into '[..]'...
...
Error: git clone exited with code 128

"#]]);

    assert!(!prj.root().join("lib/parent").exists());
}

#[forgetest]
fn install_fails_on_nested_soldeer_failure(prj: _, cmd: _) {
    let source = tempfile::tempdir().unwrap();
    init_local_install_source(source.path());
    fs::write(
        source.path().join("foundry.toml"),
        r#"[dependencies]
bad = { version = "1.0.0", url = "http://127.0.0.1:1/bad.zip" }
"#,
    )
    .unwrap();
    fs::write(source.path().join("soldeer.lock"), "version = 2\n\ndependencies = []\n").unwrap();
    let git = Git::new(source.path());
    git.add(["foundry.toml", "soldeer.lock"]).unwrap();
    git.commit("add soldeer").unwrap();

    configure_local_install(&mut cmd, source.path());
    cmd.args(["install", "--no-git", "fixture/parent"]).assert_failure().stderr_eq(str![[r#"
...
Error: Failed to install soldeer dependencies for parent: Failed to run soldeer install: [..]
Run `forge soldeer install` in [..]/lib/parent to retry.

"#]]);

    // The git dependency itself stays installed.
    assert!(prj.root().join("lib/parent/source.txt").exists());
}

fn init_local_install_source(path: &Path) {
    fs::create_dir_all(path).unwrap();
    let git = Git::new(path);
    git.init().unwrap();
    fs::write(path.join("source.txt"), "dependency source\n").unwrap();
    git.add(["source.txt"]).unwrap();
    git.commit("initial").unwrap();
}

fn configure_local_install(cmd: &mut TestCommand, source: &Path) {
    cmd.forge_fuse();
    cmd.env("LC_ALL", "C");
    cmd.env("GIT_ALLOW_PROTOCOL", "file");
    cmd.env("GIT_CONFIG_COUNT", "1");
    let source_url = Url::from_directory_path(source).unwrap();
    cmd.env("GIT_CONFIG_KEY_0", format!("url.{source_url}.insteadOf"));
    cmd.env("GIT_CONFIG_VALUE_0", "https://github.com/fixture/parent");
}
