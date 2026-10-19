use foundry_compilers::artifacts::output_selection::ContractOutputSelection;
use foundry_config::DenyLevel;
use foundry_test_utils::{forgetest, snapbox::IntoData, str, util::OutputExt};
use std::fs;

#[forgetest]
fn collision_cache_preserves_artifacts_and_invalidates_imports(prj: _, cmd: _) {
    prj.add_source(
        "Base.sol",
        "contract Base { function shared() public pure returns (uint256) { return 1; } }",
    );
    prj.add_source("First.sol", "import {Base} from './Base.sol'; contract First is Base {}");
    prj.add_source(
        "Second.sol",
        "contract Second { function shared() public pure returns (uint256) { return 2; } }",
    );
    let expected = str![[r#"
1 collisions found:

| Selector | First    | Second   |
|----------|----------|----------|
| 7126be5f | shared() | shared() |


"#]];
    let args = ["selectors", "collision", "First", "Second", "--md"];
    let cache = prj.cache().with_extension("json.abi");
    cmd.args(args).arg("--no-cache").assert_success().stdout_eq(expected.clone());
    assert!(!cache.exists());
    for _ in 0..2 {
        cmd.forge_fuse().args(args).assert_success().stdout_eq(expected.clone());
    }
    assert!(cache.is_dir());
    assert!(fs::read_dir(&prj.paths().artifacts).unwrap().next().is_none());
    assert!(!prj.cache().exists());

    let qualified =
        ["selectors", "collision", "src/First.sol:First", "src/Second.sol:Second", "--md"];
    cmd.forge_fuse().args(qualified).arg("--no-cache").assert_success().stdout_eq(expected.clone());
    cmd.forge_fuse().args(qualified).assert_success().stdout_eq(expected.clone());

    cmd.forge_fuse().args(["build", "--no-lint"]).assert_success();
    let built = [
        prj.paths().artifacts.join("First.sol/First.json"),
        prj.paths().artifacts.join("Second.sol/Second.json"),
        prj.cache().clone(),
    ]
    .map(|path| {
        let contents = fs::read(&path).unwrap();
        (path, contents)
    });
    cmd.forge_fuse().args(args).assert_success().stdout_eq(expected);

    prj.add_source(
        "Base.sol",
        "contract Base { function changed() public pure returns (uint256) { return 1; } }",
    );
    for args in [args, qualified] {
        cmd.forge_fuse().args(args).assert_success().stdout_eq(str![[r#"
No colliding method selectors between the two contracts.

"#]]);
    }
    for (path, contents) in built {
        assert_eq!(fs::read(path).unwrap(), contents);
    }

    cmd.forge_fuse().args(args).arg("--force").assert_success();
    assert!(!cache.exists());
    assert!(!prj.paths().artifacts.exists());
    assert!(!prj.cache().exists());
}

#[forgetest]
fn collision_cache_respects_warning_denial(prj: _, cmd: _) {
    prj.add_source(
        "Counter.sol",
        "contract Counter { function value() external returns (uint256) { return 1; } }",
    );
    let args = ["selectors", "collision", "Counter", "Counter"];
    for _ in 0..2 {
        cmd.forge_fuse().args(args).assert_success();
    }
    let expected = cmd
        .forge_fuse()
        .args(args)
        .args(["--deny", "warnings", "--no-cache"])
        .assert_failure()
        .get_output()
        .stderr_lossy();
    cmd.forge_fuse()
        .args(args)
        .args(["--deny", "warnings"])
        .assert_failure()
        .stderr_eq(expected.into_data().raw());
}

#[forgetest]
fn collision_cache_preserves_ambiguous_name_selection(prj: _, cmd: _) {
    prj.add_source("A.sol", "contract First { function a() external {} }");
    prj.add_source("Z.sol", "contract First { function b() external {} }");
    prj.add_source("Second.sol", "contract Second { function a() external {} }");
    let args = ["selectors", "collision", "First", "Second", "--md"];
    let expected = cmd.args(args).arg("--no-cache").assert_success().get_output().stdout_lossy();
    for _ in 0..2 {
        cmd.forge_fuse().args(args).assert_success().stdout_eq(&expected);
    }
    prj.add_source("Z.sol", "contract First { function c() external {} }");
    cmd.forge_fuse().args(args).assert_success().stdout_eq(&expected);
}

#[forgetest]
fn collision_cache_preserves_explicit_outputs(prj: _, cmd: _) {
    prj.add_source(
        "Counter.sol",
        "contract Counter { function value() external pure returns (uint256) { return 1; } }",
    );
    let cache = prj.cache().with_extension("json.abi");
    for flags in [
        vec!["--extra-output", "metadata"],
        vec!["--extra-output-files", "metadata"],
        vec!["--build-info"],
    ] {
        cmd.forge_fuse().arg("clean").assert_success();
        cmd.forge_fuse()
            .args(["selectors", "collision", "Counter", "Counter"])
            .args(flags)
            .assert_success();
        assert!(prj.paths().artifacts.join("Counter.sol/Counter.json").is_file());
        assert!(prj.cache().is_file());
        assert!(!cache.exists());
    }
}

#[forgetest]
fn find_cache_preserves_artifacts_and_invalidates_imports(prj: _, cmd: _) {
    prj.add_source("Base.sol", "contract Base { function shared() external pure {} }");
    prj.add_source("First.sol", "import './Base.sol'; contract First is Base {}");
    let args = ["selectors", "find", "7126be5f", "--md"];
    let expected = str![[r#"

| Type     | Signature | Selector   | Contract |
|----------|-----------|------------|----------|
| Function | shared()  | 0x7126be5f | Base     |
| Function | shared()  | 0x7126be5f | First    |


"#]];
    let cache = prj.cache().with_extension("json.abi");
    prj.update_config(|config| config.cache = false);
    cmd.args(args).assert_success().stdout_eq(expected.clone());
    assert!(!cache.exists());
    prj.update_config(|config| config.cache = true);
    for _ in 0..2 {
        cmd.forge_fuse().args(args).assert_success().stdout_eq(expected.clone());
    }
    assert!(cache.is_dir());
    assert!(!prj.cache().exists());
    assert!(fs::read_dir(&prj.paths().artifacts).unwrap().next().is_none());

    cmd.forge_fuse().args(["build", "--no-lint"]).assert_success();
    let built = [
        prj.paths().artifacts.join("Base.sol/Base.json"),
        prj.paths().artifacts.join("First.sol/First.json"),
        prj.cache().clone(),
    ]
    .map(|path| {
        let contents = fs::read(&path).unwrap();
        (path, contents)
    });
    cmd.forge_fuse().args(args).assert_success().stdout_eq(expected.clone());

    prj.add_source("Base.sol", "contract Base { function changed() external pure {} }");
    for _ in 0..2 {
        cmd.forge_fuse().args(args).assert_failure().stderr_eq(
            "Searching for selector \"7126be5f\" in the project...\n\
             Error: Selector not found in the project.\n",
        );
    }
    for (path, contents) in built {
        assert_eq!(fs::read(path).unwrap(), contents);
    }

    // Force rebuilds must discard both primary and secondary cached output.
    prj.add_source("Base.sol", "contract Base { function shared() external pure {} }");
    prj.update_config(|config| config.force = true);
    cmd.forge_fuse().args(args).assert_success().stdout_eq(expected);
    assert!(!prj.cache().exists());
    assert!(!prj.paths().artifacts.exists());
}

#[forgetest]
fn find_cache_respects_warning_denial(prj: _, cmd: _) {
    prj.add_source(
        "Counter.sol",
        "contract Counter { function shared() external returns (uint256) { return 1; } }",
    );
    let args = ["selectors", "find", "7126be5f"];
    for _ in 0..2 {
        cmd.forge_fuse().args(args).assert_success();
    }
    prj.update_config(|config| {
        config.deny = DenyLevel::Warnings;
        config.cache = false;
    });
    let expected = cmd.forge_fuse().args(args).assert_failure().get_output().stderr_lossy();
    prj.update_config(|config| config.cache = true);
    cmd.forge_fuse().args(args).assert_failure().stderr_eq(expected.into_data().raw());
}

#[forgetest]
fn find_cache_preserves_explicit_outputs(prj: _, cmd: _) {
    prj.add_source("Counter.sol", "contract Counter { function shared() external pure {} }");
    let args = ["selectors", "find", "7126be5f"];
    let cache = prj.cache().with_extension("json.abi");
    prj.update_config(|config| {
        config.extra_output_files = vec![ContractOutputSelection::Metadata];
    });
    cmd.args(args).assert_success();
    assert!(!cache.exists());
    assert!(!prj.cache().exists());
    assert!(fs::read_dir(&prj.paths().artifacts).unwrap().next().is_none());

    prj.update_config(|config| {
        config.extra_output_files.clear();
        config.build_info = true;
    });
    cmd.forge_fuse().args(args).assert_success();
    assert!(!cache.exists());
    assert!(prj.cache().is_file());
    assert!(prj.paths().artifacts.join("Counter.sol/Counter.json").is_file());
    assert!(fs::read_dir(prj.paths().artifacts.join("build-info")).unwrap().next().is_some());
}
