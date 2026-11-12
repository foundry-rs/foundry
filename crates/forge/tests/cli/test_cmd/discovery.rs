//! Test discovery respects the configured project directories.

use super::*;
use foundry_compilers::artifacts::remappings::Remapping;

#[cfg(unix)]
use std::os::unix::fs::symlink;

#[forgetest]
fn does_not_run_imported_dependency_tests(prj: _, cmd: _) {
    prj.update_config(|config| {
        config.libs = vec!["lib".into()];
        config.remappings =
            vec![Remapping::from_str("dependency/=node_modules/dependency/").unwrap().into()];
    });
    let dependency = prj.root().join("node_modules/dependency");
    std::fs::create_dir_all(&dependency).unwrap();
    std::fs::write(
        dependency.join("Dependency.sol"),
        r#"
// SPDX-License-Identifier: MIT
pragma solidity >=0.8.0;

contract DependencyTest {
    function testDependency() external pure {
        revert("dependency test must not run");
    }
}
"#,
    )
    .unwrap();
    prj.add_source("Import.sol", "import 'dependency/Dependency.sol'; contract Import {}");
    prj.add_test("Local.t.sol", "contract LocalTest { function testLocal() public pure {} }");

    cmd.arg("test").assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/Local.t.sol:LocalTest
[PASS] testLocal() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);

    cmd.forge_fuse().args(["test", "--list", "--json"]).assert_json_stdout(str![[r#"
{"test/Local.t.sol":{"LocalTest":["testLocal"]}}
"#]]);
    cmd.forge_fuse()
        .args(["test", "--list", "--json", "--match-path", "node_modules/**"])
        .assert_json_stdout("{}");

    cmd.forge_fuse().args(["coverage", "--report=lcov"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!
Analysing contracts...
Running tests...

Ran 1 test for test/Local.t.sol:LocalTest
[PASS] testLocal() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)
Wrote LCOV report.

"#]]);

    prj.update_config(|config| {
        config.libs.push("node_modules".into());
        config.remappings = vec![
            Remapping::from_str("dependency/=src/../node_modules/dependency/").unwrap().into(),
        ];
    });
    cmd.forge_fuse().args(["test", "--list", "--json"]).assert_json_stdout(str![[r#"
{"test/Local.t.sol":{"LocalTest":["testLocal"]}}
"#]]);
}

#[cfg(unix)]
#[forgetest]
fn discovers_tests_through_symlinked_directories(prj: _, cmd: _) {
    let shared = tempfile::tempdir().unwrap();
    fs::write(
        shared.path().join("Linked.t.sol"),
        "pragma solidity >=0.8.0; contract LinkedTest { function testLinked() public pure {} }",
    )
    .unwrap();
    fs::create_dir_all(prj.root().join("test")).unwrap();
    symlink(shared.path(), prj.root().join("test/shared")).unwrap();

    cmd.args(["test", "--list", "--json"]).assert_json_stdout(str![[r#"
{"test/shared/Linked.t.sol":{"LinkedTest":["testLinked"]}}
"#]]);
    cmd.forge_fuse().args(["test", "--match-path", "test/shared/**"]).assert_success().stdout_eq(
        str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/shared/Linked.t.sol:LinkedTest
[PASS] testLinked() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]],
    );
}
