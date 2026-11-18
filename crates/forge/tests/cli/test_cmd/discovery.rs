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

#[forgetest]
fn warns_about_imported_project_tests_outside_test_dir(prj: _, cmd: _) {
    prj.update_config(|config| config.test = "test/unit".into());
    prj.add_test(
        "shared/Shared.t.sol",
        r#"
contract SharedTest {
    function testShared() public pure {}
}

abstract contract SharedBase {
    function testInherited() public pure {}
}
"#,
    );
    prj.add_test(
        "unit/Unit.t.sol",
        r#"
import {SharedTest, SharedBase} from "../shared/Shared.t.sol";

contract UnitTest is SharedBase {
    function testUnit() public pure {}
}
"#,
    );

    cmd.arg("test").assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 2 tests for test/unit/Unit.t.sol:UnitTest
[PASS] testInherited() ([GAS])
[PASS] testUnit() ([GAS])
Suite result: ok. 2 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 2 tests passed, 0 failed, 0 skipped (2 total tests)

"#]]).stderr_eq(str![[r#"
Warning: Skipping test contracts outside the configured `src`, `test` and `script` directories:
	test/shared/Shared.t.sol:SharedTest
They are compiled because other files import them, but do not run as test suites. To run them, set `test` (currently `test/unit`) to a directory that contains them.

"#]]);
}
