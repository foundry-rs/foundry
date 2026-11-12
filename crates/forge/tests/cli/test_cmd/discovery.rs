//! Test discovery respects the configured project directories.

use super::*;
use foundry_compilers::artifacts::remappings::Remapping;
use foundry_config::{FsPermissions, fs_permissions::PathPermission};

#[forgetest]
fn does_not_run_imported_dependency_tests(prj: _, cmd: _) {
    let outside = tempfile::tempdir_in(prj.root().parent().unwrap()).unwrap();
    let input = outside.path().join("input.txt");
    let output = outside.path().join("output.txt");
    std::fs::write(&input, "synthetic test input").unwrap();
    cmd.env("DISCOVERY_INPUT", &input);
    cmd.env("DISCOVERY_OUTPUT", &output);
    prj.update_config(|config| {
        config.libs = vec!["lib".into()];
        config.remappings =
            vec![Remapping::from_str("dependency/=node_modules/dependency/").unwrap().into()];
        config.fs_permissions = FsPermissions::new([PathPermission::read("out")]);
    });
    let dependency = prj.root().join("node_modules/dependency");
    std::fs::create_dir_all(&dependency).unwrap();
    std::fs::write(
        dependency.join("Dependency.sol"),
        r#"
// SPDX-License-Identifier: MIT
pragma solidity >=0.8.0;

interface Vm {
    function envString(string calldata) external view returns (string memory);
    function readFile(string calldata) external view returns (string memory);
    function writeFile(string calldata, string calldata) external;
}

/// forge-config: default.fs_permissions = [{ access = "read-write", path = ".." }]
contract DependencyTest {
    function testDependency() external {
        Vm vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
        vm.writeFile(vm.envString("DISCOVERY_OUTPUT"), vm.readFile(vm.envString("DISCOVERY_INPUT")));
    }
}

contract FunctionConfigDependencyTest {
    /// forge-config: default.fs_permissions = [{ access = "read-write", path = ".." }]
    function testDependency() external {
        Vm vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
        vm.writeFile(vm.envString("DISCOVERY_OUTPUT"), vm.readFile(vm.envString("DISCOVERY_INPUT")));
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
    assert!(!output.exists(), "an imported dependency ran with elevated filesystem access");

    cmd.forge_fuse().args(["test", "--list", "--json"]).assert_json_stdout(str![[r#"
{"test/Local.t.sol":{"LocalTest":["testLocal"]}}
"#]]);
    cmd.forge_fuse()
        .args(["test", "--list", "--json", "--match-path", "node_modules/**"])
        .assert_json_stdout("{}");

    cmd.forge_fuse();
    cmd.env("DISCOVERY_INPUT", &input);
    cmd.env("DISCOVERY_OUTPUT", &output);
    cmd.args(["coverage", "--report=lcov"]).assert_success().stdout_eq(str![[r#"
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
    assert!(
        !output.exists(),
        "coverage ran an imported dependency with elevated filesystem access"
    );
}

#[forgetest]
fn discovers_tests_in_configured_project_directories(prj: _, cmd: _) {
    prj.update_config(|config| {
        config.src = "contracts".into();
        config.test = "checks".into();
        config.script = "tasks".into();
        config.libs = vec!["contracts/vendor".into()];
    });
    prj.create_file(
        "contracts/Source.sol",
        r#"
pragma solidity >=0.8.0;
import "./vendor/Dependency.sol";
contract SourceTest { function testSource() public pure {} }
"#,
    );
    prj.create_file(
        "contracts/vendor/Dependency.sol",
        "pragma solidity >=0.8.0; contract DependencyTest { function testDependency() public pure {} }",
    );
    prj.create_file(
        "checks/Test.t.sol",
        "pragma solidity >=0.8.0; contract TestDirectoryTest { function testCheck() public pure {} }",
    );
    prj.create_file(
        "tasks/Script.s.sol",
        "pragma solidity >=0.8.0; contract ScriptDirectoryTest { function testScript() public pure {} }",
    );

    cmd.args(["test", "--list", "--json"]).assert_json_stdout(str![[r#"
{
  "contracts/Source.sol": { "SourceTest": ["testSource"] },
  "checks/Test.t.sol": { "TestDirectoryTest": ["testCheck"] },
  "tasks/Script.s.sol": { "ScriptDirectoryTest": ["testScript"] }
}
"#]]);
    cmd.forge_fuse()
        .args(["test", "--list", "--json", "--match-path", "checks/**"])
        .assert_json_stdout(str![[r#"
{"checks/Test.t.sol":{"TestDirectoryTest":["testCheck"]}}
"#]]);
}
