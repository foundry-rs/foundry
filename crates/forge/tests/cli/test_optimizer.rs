//! Tests for commands using the preprocessed cache.

use foundry_compilers::artifacts::{EvmVersion, remappings::Remapping};
use foundry_config::{CompilationRestrictions, SettingsOverrides};

// <https://github.com/foundry-rs/foundry/issues/16682>
forgetest!(preprocess_remapped_bytecode_dependencies, |prj, cmd| {
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
        config.remappings = vec!["@p/=src/".parse::<Remapping>().unwrap().into()];
    });
    let source = r#"
contract Impl {
    constructor(uint256) {}
    function v() external pure returns (uint256) { return 111; }
}
"#;
    prj.add_source("Impl.sol", source);
    prj.add_test(
        "Impl.t.sol",
        r#"
import {Impl} from "@p/Impl.sol";
contract ImplTest {
    function test_new() public {
        require(new Impl(1).v() == 111, "stale implementation");
    }
    function test_creationCode() public {
        bytes memory code = abi.encodePacked(type(Impl).creationCode, abi.encode(uint256(1)));
        address deployed;
        assembly { deployed := create(0, add(code, 32), mload(code)) }
        require(Impl(deployed).v() == 111, "stale implementation");
    }
}
"#,
    );
    cmd.env("RUST_LOG", "error");
    cmd.args(["test"]).assert_success().stderr_eq("").stdout_eq(str![[r#"
...
Ran 2 tests for test/Impl.t.sol:ImplTest
[PASS] test_creationCode() ([GAS])
[PASS] test_new() ([GAS])
Suite result: ok. 2 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 2 tests passed, 0 failed, 0 skipped (2 total tests)

"#]]);

    // A body-only edit must reach both dynamically linked bytecode references.
    prj.add_source("Impl.sol", &source.replace("return 111", "return 222"));
    prj.forge_command().arg("build").with_no_redact().assert_success().stdout_eq(str![[r#"
Compiling 1 files with [..]
[..]
Compiler run successful!

"#]]);
    cmd.forge_fuse().arg("test").assert_failure().stdout_eq(str![[r#"
No files changed, compilation skipped

Ran 2 tests for test/Impl.t.sol:ImplTest
[FAIL: stale implementation] test_creationCode() ([GAS])
[FAIL: stale implementation] test_new() ([GAS])
Suite result: FAILED. 0 passed; 2 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 2 failed, 0 skipped (2 total tests)

Failing tests:
Encountered 2 failing tests in test/Impl.t.sol:ImplTest
[FAIL: stale implementation] test_creationCode() ([GAS])
[FAIL: stale implementation] test_new() ([GAS])

Encountered a total of 2 failing tests, 0 tests succeeded
...
"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/16682>
forgetest!(preprocess_remapping_identity, |prj, cmd| {
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
        config.remappings = ["@p/=src/../src/", "src/=lib/alternate/"]
            .into_iter()
            .map(|remapping| remapping.parse::<Remapping>().unwrap().into())
            .collect();
    });
    let source = r#"
contract Impl {
    struct Args { uint256 value; }
    constructor(Args memory args) {}
    function v() external pure returns (uint256) { return 111; }
}
"#;
    prj.add_source("Impl.sol", source);
    prj.create_file(
        "lib/alternate/Impl.sol",
        r#"
contract Impl {
    struct Args { address value; }
    constructor(Args memory args) {}
    function v() external pure returns (uint256) { return 999; }
}
"#,
    );
    prj.add_test(
        "Impl.t.sol",
        r#"
import {Impl as Implementation} from "@p/Impl.sol";
contract ImplTest {
    function test_new() public {
        require(
            new Implementation(Implementation.Args({value: 1})).v() == 111,
            "stale implementation"
        );
    }
}
contract EmptyTest {}
"#,
    );
    cmd.args(["test"]).assert_success();

    // Ambiguous source-unit references stay native and are invalidated after a body-only edit.
    prj.add_source("Impl.sol", &source.replace("return 111", "return 222"));
    cmd.forge_fuse().arg("test").with_no_redact().assert_failure().stdout_eq(str![[r#"
Compiling 3 files with [..]
[..]
Compiler run successful!

Ran 1 test for test/Impl.t.sol:ImplTest
[FAIL: stale implementation] test_new() (gas: [..])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; finished in [..] ([..] CPU time)

Ran 1 test suite in [..] ([..] CPU time): 0 tests passed, 1 failed, 0 skipped (1 total tests)

Failing tests:
Encountered 1 failing test in test/Impl.t.sol:ImplTest
[FAIL: stale implementation] test_new() (gas: [..])

Encountered a total of 1 failing tests, 0 tests succeeded
...
"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/16682>
forgetest!(preprocess_remapping_context_uses_running_test, |prj, cmd| {
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
        config.remappings =
            vec!["test/suite/:src/=lib/alternate/".parse::<Remapping>().unwrap().into()];
    });
    let source = r#"
contract Impl {
    function v() external pure returns (uint256) { return 111; }
}
"#;
    prj.add_source("Impl.sol", source);
    prj.create_file("lib/alternate/Impl.sol", &source.replace("return 111", "return 999"));
    prj.add_test(
        "support/Helper.sol",
        r#"
import {Impl} from "src/Impl.sol";
contract Helper {
    function create() public returns (Impl) { return new Impl(); }
}
"#,
    );
    prj.add_test(
        "suite/Impl.t.sol",
        r#"
import {Helper} from "../support/Helper.sol";
contract ImplTest is Helper {
    function test_new() public {
        require(create().v() == 111, "stale implementation");
    }
}
"#,
    );
    cmd.args(["test"]).assert_success();

    prj.add_source("Impl.sol", &source.replace("return 111", "return 222"));
    cmd.forge_fuse().arg("test").assert_failure().stdout_eq(str![[r#"
...
[FAIL: stale implementation] test_new() ([GAS])
...
"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/16682>
forgetest!(preprocess_remapped_helper_source, |prj, cmd| {
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
        config.remappings = ["@p/=src/", "test/:foundry-pp/=lib/alternate/"]
            .into_iter()
            .map(|remapping| remapping.parse::<Remapping>().unwrap().into())
            .collect();
    });
    let source = r#"
contract Impl {
    constructor(uint256) {}
    function v() external pure returns (uint256) { return 111; }
}
"#;
    prj.add_source("Impl.sol", source);
    prj.add_test(
        "Impl.t.sol",
        r#"
import {Impl} from "@p/Impl.sol";
contract ImplTest {
    function test_new() public {
        require(new Impl(1).v() == 111, "stale implementation");
    }
}
"#,
    );
    cmd.args(["test"]).assert_success();

    prj.add_source("Impl.sol", &source.replace("return 111", "return 222"));
    cmd.forge_fuse().arg("test").assert_failure().stdout_eq(str![[r#"
...
[FAIL: stale implementation] test_new() ([GAS])
...
"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/16682>
forgetest!(preprocess_ambiguous_artifact_stays_native, |prj, cmd| {
    prj.update_config(|config| config.dynamic_test_linking = true);
    let source = r#"
contract Impl {
    function v() external pure returns (uint256) { return 111; }
}
"#;
    prj.add_source("Impl.sol", source);
    prj.create_file("vendor/pkg/src/Impl.sol", source);
    prj.add_source(
        "UsesLib.sol",
        r#"
import {Impl as LibImpl} from "vendor/pkg/src/Impl.sol";
contract UsesLib {
    function create() public returns (LibImpl) { return new LibImpl(); }
}
"#,
    );
    prj.forge_command().arg("build").assert_success();

    let test = r#"
import {Impl} from "../src/Impl.sol";
contract ImplTest {
    function test_new() public {
        require(new Impl().v() == 111, "stale implementation");
    }
}
"#;
    prj.add_test("Impl.t.sol", test);
    cmd.args(["test"]).assert_success();

    // A narrower test-only compilation must retain the native fallback classification.
    prj.add_test("Impl.t.sol", &format!("\n{test}"));
    cmd.forge_fuse().arg("test").assert_success();

    prj.add_source("Impl.sol", &source.replace("return 111", "return 222"));
    cmd.forge_fuse().arg("test").assert_failure().stdout_eq(str![[r#"
...
[FAIL: stale implementation] test_new() ([GAS])
...
"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/16682>
forgetest!(preprocess_remapped_mock_inheritance, |prj, cmd| {
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
        config.remappings = vec!["@p/=src/".parse::<Remapping>().unwrap().into()];
    });
    let source = r#"
contract Impl {
    function v() public pure returns (uint256) { return 111; }
}
"#;
    prj.add_source("Impl.sol", source);
    prj.add_test(
        "ImplMock.sol",
        r#"
import {Impl} from "@p/Impl.sol";
contract ImplMock is Impl {}
"#,
    );
    prj.add_test(
        "Impl.t.sol",
        r#"
import {ImplMock} from "./ImplMock.sol";
contract ImplTest is ImplMock {
    function test_inherited() public pure {
        require(v() == 111, "stale implementation");
    }
}
"#,
    );
    cmd.args(["test"]).assert_success().stdout_eq(str![[r#"
...
Ran 1 test for test/Impl.t.sol:ImplTest
[PASS] test_inherited() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);

    // Inherited bytecode stays native, so the test must be rebuilt as well.
    prj.add_source("Impl.sol", &source.replace("return 111", "return 222"));
    prj.forge_command().arg("build").with_no_redact().assert_success().stdout_eq(str![[r#"
Compiling 3 files with [..]
[..]
Compiler run successful!

"#]]);
    cmd.forge_fuse().arg("test").assert_failure().stdout_eq(str![[r#"
No files changed, compilation skipped

Ran 1 test for test/Impl.t.sol:ImplTest
[FAIL: stale implementation] test_inherited() ([GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 1 failed, 0 skipped (1 total tests)

Failing tests:
Encountered 1 failing test in test/Impl.t.sol:ImplTest
[FAIL: stale implementation] test_inherited() ([GAS])

Encountered a total of 1 failing tests, 0 tests succeeded
...
"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/16682>
#[cfg(unix)]
forgetest!(preprocess_remapped_symlinked_source, |prj, cmd| {
    use std::{fs, os::unix::fs::symlink};

    fs::remove_dir_all(prj.root().join("src")).unwrap();
    fs::create_dir_all(prj.root().join(".shared/src")).unwrap();
    symlink(".shared/src", prj.root().join("src")).unwrap();
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
        config.remappings = vec!["@p/=src/".parse::<Remapping>().unwrap().into()];
    });
    let source = r#"
contract Impl {
    function v() external pure returns (uint256) { return 111; }
}
"#;
    prj.add_source("Impl.sol", source);
    prj.add_test(
        "Impl.t.sol",
        r#"
import {Impl} from "@p/Impl.sol";
contract ImplTest {
    function test_new() public {
        require(new Impl().v() == 111, "stale implementation");
    }
}
"#,
    );
    cmd.args(["test"]).assert_success();

    prj.add_source("Impl.sol", &source.replace("return 111", "return 222"));
    cmd.forge_fuse().arg("test").with_no_redact().assert_failure().stdout_eq(str![[r#"
Compiling 3 files with [..]
[..]
Compiler run successful!

Ran 1 test for test/Impl.t.sol:ImplTest
[FAIL: stale implementation] test_new() (gas: [..])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; finished in [..] ([..] CPU time)

Ran 1 test suite in [..] ([..] CPU time): 0 tests passed, 1 failed, 0 skipped (1 total tests)

Failing tests:
Encountered 1 failing test in test/Impl.t.sol:ImplTest
[FAIL: stale implementation] test_new() (gas: [..])

Encountered a total of 1 failing tests, 0 tests succeeded
...
"#]]);
});

#[cfg(unix)]
forgetest_init!(abi_commands_reuse_preprocessed_cache, |prj, cmd| {
    use foundry_test_utils::util::OutputExt;
    use std::{fs, os::unix::fs::PermissionsExt};

    prj.initialize_default_contracts();
    prj.update_config(|config| config.dynamic_test_linking = true);
    cmd.arg("build").assert_success();

    let solc = prj.root().join("fake-solc");
    let invoked = prj.root().join("fake-solc.invoked");
    fs::write(
        &solc,
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
    echo "solc, the solidity compiler commandline interface"
    echo "Version: 0.8.35+commit.69074fbd"
    exit 0
fi
touch "$0.invoked"
exit 1
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(&solc).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&solc, permissions).unwrap();
    prj.update_config(|config| {
        config.solc = Some(foundry_config::SolcReq::Local(solc));
    });

    let output =
        cmd.forge_fuse().args(["test", "--match-contract", "CounterTest"]).assert_success();
    let stdout = output.get_output().stdout_lossy();
    assert!(
        stdout.contains("Ran 2 tests for test/Counter.t.sol:CounterTest"),
        "cached ABI did not select CounterTest: {stdout}"
    );
    assert!(!invoked.exists(), "filtered test compilation did not reuse the preprocessed cache");

    cmd.forge_fuse().args(["selectors", "list"]).assert_success();
    assert!(!invoked.exists(), "selector compilation did not reuse the preprocessed cache");
});

// <https://github.com/foundry-rs/foundry/issues/8842>
forgetest_init!(filtered_tests_compile_unimported_test_fixtures, |prj, cmd| {
    prj.update_config(|config| config.solc = None);
    prj.add_raw_test(
        "fixtures/Fixture.sol",
        r#"
pragma solidity 0.7.6;

contract Fixture {
    function version() external pure returns (uint256) {
        return 1;
    }
}
"#,
    );
    prj.add_test(
        "Fixture.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

interface IFixture {
    function version() external pure returns (uint256);
}

contract FixtureTest is Test {
    function testFixture() public {
        address fixture = vm.deployCode("test/fixtures/Fixture.sol:Fixture");
        assertEq(IFixture(fixture).version(), 1);
    }
}
"#,
    );

    cmd.args(["test", "--match-contract", "FixtureTest"]).assert_success().stdout_eq(str![[r#"
...
Ran 1 test for test/Fixture.t.sol:FixtureTest
[PASS] testFixture() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);

    prj.add_raw_test(
        "fixtures/Fixture.sol",
        r#"
pragma solidity 0.7.6;

contract Fixture {
    function version() external pure returns (uint256) {
        return 2;
    }
}
"#,
    );
    prj.add_test(
        "Fixture.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

interface IFixture {
    function version() external pure returns (uint256);
}

contract FixtureTest is Test {
    function testFixture() public {
        address fixture = vm.deployCode("test/fixtures/Fixture.sol:Fixture");
        assertEq(IFixture(fixture).version(), 2);
    }
}
"#,
    );

    cmd.assert_success().stdout_eq(str![[r#"
...
Ran 1 test for test/Fixture.t.sol:FixtureTest
[PASS] testFixture() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/8842>
forgetest_init!(path_filtered_tests_compile_unimported_test_fixtures, |prj, cmd| {
    prj.update_config(|config| {
        config.solc = None;
        config.dynamic_test_linking = false;
    });
    prj.add_raw_script("Broken.s.sol", "this is not valid Solidity");
    prj.add_raw_test(
        "fixtures/Fixture.sol",
        r#"
pragma solidity 0.7.6;

contract Fixture {}
"#,
    );
    prj.add_test(
        "Fixture.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract FixtureTest is Test {
    function testFixture() public {
        assertGt(vm.getCode("test/fixtures/Fixture.sol:Fixture").length, 0);
    }
}
"#,
    );

    cmd.args(["test", "--match-path", "test/Fixture.t.sol"]).assert_success().stdout_eq(str![[
        r#"
...
Ran 1 test for test/Fixture.t.sol:FixtureTest
[PASS] testFixture() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#
    ]]);
});

// <https://github.com/foundry-rs/foundry/issues/16529>
forgetest_init!(filtered_tests_preserve_compilation_restrictions, |prj, cmd| {
    prj.wipe_contracts();
    prj.add_lib(
        "dep/src/Clz.sol",
        r#"
library Clz {
    function msb(uint128 bitmap) internal pure returns (uint256 res) {
        assembly {
            res := sub(255, clz(bitmap))
        }
    }
}
"#,
    );
    prj.add_source(
        "Root.sol",
        r#"
import "../lib/dep/src/Clz.sol";

contract Root {
    function msb(uint128 bitmap) external pure returns (uint256) {
        return Clz.msb(bitmap);
    }
}
"#,
    );
    prj.add_test("RootTest.sol", "contract RootTest { function testFoo() public pure {} }");
    prj.update_config(|config| {
        config.evm_version = EvmVersion::Prague;
        config.additional_compiler_profiles = vec![SettingsOverrides {
            name: "osaka".to_string(),
            via_ir: None,
            evm_version: Some(EvmVersion::Osaka),
            optimizer: None,
            optimizer_runs: None,
            bytecode_hash: None,
        }];
        config.compilation_restrictions = vec![CompilationRestrictions {
            paths: "src/Root.sol".parse().unwrap(),
            version: None,
            via_ir: None,
            bytecode_hash: None,
            min_optimizer_runs: None,
            optimizer_runs: None,
            max_optimizer_runs: None,
            min_evm_version: None,
            evm_version: Some(EvmVersion::Osaka),
            max_evm_version: None,
        }];
    });

    cmd.args(["test", "--match-path", "test/RootTest.sol"]).assert_success();
});

forgetest_init!(filtered_tests_support_overlapping_source_roots, |prj, cmd| {
    prj.update_config(|config| config.script = ".".into());
    prj.add_source("SourceFixture.sol", "contract SourceFixture {}");
    prj.add_test("fixtures/Fixture.sol", "contract Fixture {}");
    prj.add_test(
        "Fixture.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract FixtureTest is Test {
    function testFixture() public {
        assertGt(vm.getCode("test/fixtures/Fixture.sol:Fixture").length, 0);
        assertGt(vm.getCode("src/SourceFixture.sol:SourceFixture").length, 0);
    }
}
"#,
    );

    cmd.args(["test", "--match-contract", "FixtureTest"]).assert_success().stdout_eq(str![[r#"
...
Ran 1 test for test/Fixture.t.sol:FixtureTest
[PASS] testFixture() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
});

// Test cache is invalidated when `forge build` if optimize test option toggled.
forgetest_init!(toggle_invalidate_cache_on_build, |prj, cmd| {
    prj.initialize_default_contracts();
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
    });
    // All files are built with optimized tests.
    cmd.args(["build"]).with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 23 files with [..]
...

"#]]);
    // No files are rebuilt.
    cmd.with_no_redact().assert_success().stdout_eq(str![[r#"
...
No files changed, compilation skipped
...

"#]]);

    // Toggle test optimizer off.
    prj.update_config(|config| {
        config.dynamic_test_linking = false;
    });
    // All files are rebuilt with preprocessed cache false.
    cmd.with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 23 files with [..]
...

"#]]);
});

// Test cache is invalidated when `forge test` if optimize test option toggled.
forgetest_init!(toggle_invalidate_cache_on_test, |prj, cmd| {
    prj.initialize_default_contracts();
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
    });
    // All files are built with optimized tests.
    cmd.args(["test"]).with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 21 files with [..]
...

"#]]);
    // No files are rebuilt.
    cmd.with_no_redact().assert_success().stdout_eq(str![[r#"
...
No files changed, compilation skipped
...

"#]]);

    // Toggle test optimizer off.
    prj.update_config(|config| {
        config.dynamic_test_linking = false;
    });
    // All files are rebuilt with preprocessed cache false.
    cmd.with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 21 files with [..]
...

"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/16468>
forgetest_init!(unchecked_artifacts_support_dynamic_linking, |prj, cmd| {
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
        config.unchecked_cheatcode_artifacts = true;
    });
    prj.add_source(
        "Counter.sol",
        r#"
library Math {
    function double(uint256 x) public pure returns (uint256) {
        return x * 2;
    }
}

contract Counter {
    uint256 public number;

    constructor(uint256 number_) {
        number = Math.double(number_);
    }
}
"#,
    );
    prj.add_source(
        "nested/Counter.sol",
        r#"
library Math {
    function triple(uint256 x) public pure returns (uint256) {
        return x * 3;
    }
}

contract Counter {
    uint256 public number;

    constructor(uint256 number_) {
        number = Math.triple(number_);
    }
}
"#,
    );
    prj.add_test(
        "Counter.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Counter as DoubleCounter} from "../src/Counter.sol";
import {Counter as TripleCounter} from "../src/nested/Counter.sol";

contract CounterTest is Test {
    function testNew() public {
        DoubleCounter doubleCounter = new DoubleCounter(21);
        TripleCounter tripleCounter = new TripleCounter(21);
        assertEq(doubleCounter.number(), 42);
        assertEq(tripleCounter.number(), 63);
    }
}
"#,
    );

    cmd.args(["test", "--match-test", "testNew"]).assert_success().stdout_eq(str![[r#"
...
Ran 1 test for test/Counter.t.sol:CounterTest
[PASS] testNew() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
});

// Counter contract without interface instantiated in CounterTest
//
// ├── src
// │ └── Counter.sol
// └── test
//     └── Counter.t.sol
forgetest_init!(preprocess_contract_with_no_interface, |prj, cmd| {
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
    });

    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    uint256 public number;

    function setNumber(uint256 newNumber) public {
        number = newNumber;
    }

    function increment() public {
        number++;
    }
}
    "#,
    );

    prj.add_test(
        "Counter.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Counter} from "../src/Counter.sol";

contract CounterTest is Test {
    Counter public counter;

    function setUp() public {
        counter = new Counter();
        counter.setNumber(0);
    }

    function test_Increment() public {
        counter.increment();
        assertEq(counter.number(), 1);
    }

    function test_SetNumber() public {
        counter.setNumber(1);
        assertEq(counter.number(), 1);
    }
}
    "#,
    );
    // All files are compiled on first run.
    cmd.args(["test"]).with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 21 files with [..]
...

"#]]);

    // Change Counter implementation to fail both tests.
    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    uint256 public number;

    function setNumber(uint256 newNumber) public {
        number = 12345;
    }

    function increment() public {
        number++;
        number++;
    }
}
    "#,
    );
    // Assert that only 1 file is compiled (Counter source contract) and both tests fail.
    cmd.with_no_redact().assert_failure().stdout_eq(str![[r#"
...
Compiling 1 files with [..]
...
[FAIL: assertion failed: 12347 != 1] test_Increment() (gas: [..])
[FAIL: assertion failed: 12345 != 1] test_SetNumber() (gas: [..])
...

"#]]);

    // Change Counter implementation to fail single test.
    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    uint256 public number;

    function setNumber(uint256 newNumber) public {
        number = 1;
    }

    function increment() public {
        number++;
    }
}
    "#,
    );
    // Assert that only 1 file is compiled (Counter source contract) and only one test fails.
    cmd.with_no_redact().assert_failure().stdout_eq(str![[r#"
...
Compiling 1 files with [..]
...
[FAIL: assertion failed: 2 != 1] test_Increment() (gas: [..])
[PASS] test_SetNumber() (gas: [..])
...

"#]]);
});

// Counter contract with interface instantiated in CounterTest
//
// ├── src
// │ ├── Counter.sol
// │ └── interface
// │     └── CounterIf.sol
// └── test
//     └── Counter.t.sol
forgetest_init!(preprocess_contract_with_interface, |prj, cmd| {
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
    });

    prj.add_source(
        "interface/CounterIf.sol",
        r#"
interface CounterIf {
    function number() external returns (uint256);

    function setNumber(uint256 newNumber) external;

    function increment() external;
}
    "#,
    );
    prj.add_source(
        "Counter.sol",
        r#"
import {CounterIf} from "./interface/CounterIf.sol";
contract Counter is CounterIf {
    uint256 public number;
    uint256 public anotherNumber;

    function setNumber(uint256 newNumber) public {
        number = newNumber;
    }

    function increment() public {
        number++;
    }
}
    "#,
    );

    prj.add_test(
        "Counter.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Counter} from "../src/Counter.sol";

contract CounterTest is Test {
    Counter public counter;

    function setUp() public {
        counter = Counter(address(new Counter()));
        counter.setNumber(0);
    }

    function test_Increment() public {
        counter.increment();
        assertEq(counter.number(), 1);
    }

    function test_SetNumber() public {
        counter.setNumber(1);
        assertEq(counter.number(), 1);
    }
}
    "#,
    );
    // All 21 files are compiled on first run.
    cmd.args(["test"]).with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 22 files with [..]
...

"#]]);

    // Change only CounterIf interface.
    prj.add_source(
        "interface/CounterIf.sol",
        r#"
interface CounterIf {
    function anotherNumber() external returns (uint256);

    function number() external returns (uint256);

    function setNumber(uint256 newNumber) external;

    function increment() external;
}
    "#,
    );
    // All 3 files (interface, implementation and test) are compiled.
    cmd.with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 3 files with [..]
...

"#]]);

    // Change Counter implementation to fail both tests.
    prj.add_source(
        "Counter.sol",
        r#"
import {CounterIf} from "./interface/CounterIf.sol";
contract Counter is CounterIf {
    uint256 public number;
    uint256 public anotherNumber;

    function setNumber(uint256 newNumber) public {
        number = 12345;
    }

    function increment() public {
        number++;
        number++;
    }
}
    "#,
    );
    // Assert that only 1 file is compiled (Counter source contract) and both tests fail.
    cmd.with_no_redact().assert_failure().stdout_eq(str![[r#"
...
Compiling 1 files with [..]
...
[FAIL: assertion failed: 12347 != 1] test_Increment() (gas: [..])
[FAIL: assertion failed: 12345 != 1] test_SetNumber() (gas: [..])
...

"#]]);
});

// - Counter contract instantiated in CounterMock
// - CounterMock instantiated in CounterTest
//
// ├── src
// │ └── Counter.sol
// └── test
//     ├── Counter.t.sol
//     └── mock
//         └── CounterMock.sol
forgetest_init!(preprocess_mock_without_inheritance, |prj, cmd| {
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
    });

    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    uint256 public number;

    function setNumber(uint256 newNumber) public {
        number = newNumber;
    }

    function increment() public {
        number++;
    }
}
    "#,
    );

    prj.add_test(
        "mock/CounterMock.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Counter} from "src/Counter.sol";

contract CounterMock {
    Counter counter = new Counter();

    function setNumber(uint256 newNumber) public {
        counter.setNumber(newNumber);
    }

    function increment() public {
        counter.increment();
    }

    function number() public returns (uint256) {
        return counter.number();
    }
}
    "#,
    );
    prj.add_test(
        "Counter.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {CounterMock} from "./mock/CounterMock.sol";

contract CounterTest is Test {
    CounterMock public counter;

    function setUp() public {
        counter = new CounterMock();
        counter.setNumber(0);
    }

    function test_Increment() public {
        counter.increment();
        assertEq(counter.number(), 1);
    }

    function test_SetNumber() public {
        counter.setNumber(1);
        assertEq(counter.number(), 1);
    }
}
    "#,
    );
    // 20 files plus one mock file are compiled on first run.
    cmd.args(["test"]).with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 22 files with [..]
...

"#]]);

    // Change Counter contract implementation to fail both tests.
    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    uint256 public number;

    function setNumber(uint256 newNumber) public {
        number = 12345;
    }

    function increment() public {
        number++;
        number++;
    }
}
    "#,
    );
    // Assert that only 1 file is compiled (Counter source contract) and both tests fail.
    cmd.with_no_redact().assert_failure().stdout_eq(str![[r#"
...
Compiling 1 files with [..]
...
[FAIL: assertion failed: 12347 != 1] test_Increment() (gas: [..])
[FAIL: assertion failed: 12345 != 1] test_SetNumber() (gas: [..])
...

"#]]);

    // Change CounterMock contract implementation to pass both tests.
    prj.add_test(
        "mock/CounterMock.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Counter} from "src/Counter.sol";

contract CounterMock {
    Counter counter = new Counter();

    function setNumber(uint256 newNumber) public {
    }

    function increment() public {
    }

    function number() public returns (uint256) {
        return 1;
    }
}
    "#,
    );
    // Assert that mock and test files are compiled and no test fails.
    cmd.with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 2 files with [..]
...
[PASS] test_Increment() (gas: [..])
[PASS] test_SetNumber() (gas: [..])
...

"#]]);
});

// - CounterMock contract is Counter contract
// - CounterMock instantiated in CounterTest
//
// ├── src
// │ └── Counter.sol
// └── test
//    ├── Counter.t.sol
//    └── mock
//        └── CounterMock.sol
forgetest_init!(preprocess_mock_with_inheritance, |prj, cmd| {
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
    });

    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    uint256 public number;

    function setNumber(uint256 newNumber) public {
        number = newNumber;
    }

    function increment() public {
        number++;
    }
}
    "#,
    );

    prj.add_test(
        "mock/CounterMock.sol",
        r#"
import {Counter} from "src/Counter.sol";

contract CounterMock is Counter {
}
    "#,
    );
    prj.add_test(
        "Counter.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {CounterMock} from "./mock/CounterMock.sol";

contract CounterTest is Test {
    CounterMock public counter;

    function setUp() public {
        counter = new CounterMock();
        counter.setNumber(0);
    }

    function test_Increment() public {
        counter.increment();
        assertEq(counter.number(), 1);
    }

    function test_SetNumber() public {
        counter.setNumber(1);
        assertEq(counter.number(), 1);
    }
}
    "#,
    );
    // 20 files plus one mock file are compiled on first run.
    cmd.args(["test"]).with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 22 files with [..]
...

"#]]);

    // Change Counter contract implementation to fail both tests.
    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    uint256 public number;

    function setNumber(uint256) public virtual {
        number = 12345;
    }

    function increment() public virtual {
        number++;
        number++;
    }
}
    "#,
    );
    // Assert Counter source contract and CounterTest test contract (as it imports mock) are
    // compiled and both tests fail.
    cmd.with_no_redact().assert_failure().stdout_eq(str![[r#"
...
Compiling 3 files with [..]
...
[FAIL: assertion failed: 12347 != 1] test_Increment() (gas: [..])
[FAIL: assertion failed: 12345 != 1] test_SetNumber() (gas: [..])
...

"#]]);

    // Change mock implementation to pass both tests.
    prj.add_test(
        "mock/CounterMock.sol",
        r#"
import {Counter} from "src/Counter.sol";

contract CounterMock is Counter {
    function setNumber(uint256 newNumber) public override {
        number = newNumber;
    }

    function increment() public override {
        number++;
    }
}
    "#,
    );
    // Assert that CounterMock and CounterTest files are compiled and no test fails.
    cmd.with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 2 files with [..]
...
[PASS] test_Increment() (gas: [..])
[PASS] test_SetNumber() (gas: [..])
...

"#]]);
});

// - CounterMock contract is Counter contract
// - CounterMock instantiated in CounterTest
//
// ├── src
// │ └── Counter.sol
// └── test
//    ├── Counter.t.sol
//    └── mock
//        └── CounterMock.sol
forgetest_init!(preprocess_mock_to_non_mock, |prj, cmd| {
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
    });

    let source = r#"
contract Counter {
    uint256 public number;

    function setNumber(uint256 newNumber) public {
        number = newNumber;
    }

    function increment() public {
        number++;
    }
}
    "#;
    prj.add_source("Counter.sol", source);

    prj.add_test(
        "mock/CounterMock.sol",
        r#"
import {Counter} from "src/Counter.sol";

contract CounterMock is Counter {
}
    "#,
    );
    prj.add_test(
        "Counter.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {CounterMock} from "./mock/CounterMock.sol";

contract CounterTest is Test {
    CounterMock public counter;

    function setUp() public {
        counter = new CounterMock();
        counter.setNumber(0);
    }

    function test_Increment() public {
        counter.increment();
        assertEq(counter.number(), 1);
    }

    function test_SetNumber() public {
        counter.setNumber(1);
        assertEq(counter.number(), 1);
    }
}
    "#,
    );
    // 20 files plus one mock file are compiled on first run.
    cmd.args(["test"]).with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 22 files with [..]
...

"#]]);
    cmd.with_no_redact().assert_success().stdout_eq(str![[r#"
...
No files changed, compilation skipped
...

"#]]);

    // Change mock implementation to fail tests, no inherit from Counter.
    prj.add_test(
        "mock/CounterMock.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Counter} from "src/Counter.sol";

contract CounterMock {
    uint256 public number;
    function setNumber(uint256 newNumber) public {
        number = 1234;
    }

    function increment() public {
        number = 5678;
    }
}
    "#,
    );
    // Assert that CounterMock and CounterTest files are compiled and tests fail.
    cmd.with_no_redact().assert_failure().stdout_eq(str![[r#"
...
Compiling 2 files with [..]
...
[FAIL: assertion failed: 5678 != 1] test_Increment() (gas: [..])
[FAIL: assertion failed: 1234 != 1] test_SetNumber() (gas: [..])
...

"#]]);

    // The former mock classification must not rebuild importers after a source body-only edit.
    prj.add_source("Counter.sol", &source.replace("number++", "number += 2"));
    prj.forge_command().arg("build").with_no_redact().assert_success().stdout_eq(str![[r#"
Compiling 1 files with [..]
[..]
Compiler run successful!

"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/12452>
// - CounterMock contract is Counter contract
// - CounterMock declared in CounterTest
//
// ├── src
// │ └── Counter.sol
// └── test
//    ├── Counter.t.sol
forgetest_init!(preprocess_mock_declared_in_test_contract, |prj, cmd| {
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
    });

    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    function add(uint256 x, uint256 y) public pure returns (uint256) {
        return x + y;
    }
}
    "#,
    );

    prj.add_test(
        "Counter.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Counter} from "src/Counter.sol";

contract CounterMock is Counter {}

contract CounterTest is Test {
    function test_add() public {
        CounterMock impl = new CounterMock();
        assertEq(impl.add(2, 2), 4);
    }
}
    "#,
    );
    // 20 files plus one mock file are compiled on first run.
    cmd.args(["test"]).with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 21 files with [..]
...

"#]]);
    cmd.with_no_redact().assert_success().stdout_eq(str![[r#"
...
No files changed, compilation skipped
...

"#]]);

    // Change Counter implementation to fail tests.
    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    function add(uint256 x, uint256 y) public pure returns (uint256) {
        return x + y + 1;
    }
}
    "#,
    );
    // Assert that Counter and CounterTest files are compiled and tests fail.
    cmd.with_no_redact().assert_failure().stdout_eq(str![[r#"
...
Compiling 2 files with [..]
...
[FAIL: assertion failed: 5 != 4] test_add() (gas: [..])
...

"#]]);
});

// ├── src
// │ ├── CounterA.sol
// │ ├── CounterB.sol
// │ ├── Counter.sol
// │ └── v1
// │     └── Counter.sol
// └── test
// └── Counter.t.sol
forgetest_init!(preprocess_multiple_contracts_with_constructors, |prj, cmd| {
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
    });

    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    uint256 public number;

    function setNumber(uint256 newNumber) public {
        number = newNumber;
    }

    function increment() public {
        number++;
    }
}
    "#,
    );
    prj.add_source(
        "CounterA.sol",
        r#"
contract CounterA {
    uint256 public number;
    address public owner;

    constructor(uint256 _newNumber, address _owner) {
        number = _newNumber;
        owner = _owner;
    }

    function increment() public {
        number++;
    }
}
    "#,
    );
    // Contract with constructor args without name.
    prj.add_source(
        "CounterB.sol",
        r#"
contract CounterB {
    uint256 public number;

    constructor(uint256) {
        number = 1;
    }

    function increment() public {
        number++;
    }
}
    "#,
    );
    prj.add_source(
        "v1/Counter.sol",
        r#"
contract Counter {
    uint256 public number;

    constructor(uint256 _number) {
        number = _number;
    }

    function increment() public {
        number++;
    }
}
    "#,
    );

    prj.add_test(
        "Counter.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Counter} from "src/Counter.sol";
import "src/CounterA.sol";
import "src/CounterB.sol";
import {Counter as CounterV1} from "src/v1/Counter.sol";

contract CounterTest is Test {
    function test_Increment_In_Counter() public {
        Counter counter = new Counter();
        counter.increment();
        assertEq(counter.number(), 1);
    }

    function test_Increment_In_Counter_V1() public {
        CounterV1 counter = new CounterV1(1234);
        counter.increment();
        assertEq(counter.number(), 1235);
    }

    function test_Increment_In_Counter_A() public {
        CounterA counter = new CounterA(1234, address(this));
        counter.increment();
        assertEq(counter.number(), 1235);
    }

    function test_Increment_In_Counter_A_with_named_args() public {
        CounterA counter = new CounterA({_newNumber: 1234, _owner: address(this)});
        counter.increment();
        assertEq(counter.number(), 1235);
    }

    function test_Increment_In_Counter_B() public {
        CounterB counter = new CounterB(1234);
        counter.increment();
        assertEq(counter.number(), 2);
    }
}
    "#,
    );
    // 22 files plus one mock file are compiled on first run.
    cmd.args(["test"]).with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 24 files with [..]
...
[PASS] test_Increment_In_Counter() (gas: [..])
[PASS] test_Increment_In_Counter_A() (gas: [..])
[PASS] test_Increment_In_Counter_A_with_named_args() (gas: [..])
[PASS] test_Increment_In_Counter_B() (gas: [..])
[PASS] test_Increment_In_Counter_V1() (gas: [..])
...

"#]]);

    // Change v1/Counter to fail test.
    prj.add_source(
        "v1/Counter.sol",
        r#"
contract Counter {
    uint256 public number;

    constructor(uint256 _number) {
        number = _number;
    }

    function increment() public {
        number = 12345;
    }
}
    "#,
    );
    // Only v1/Counter should be compiled and test should fail.
    cmd.with_no_redact().assert_failure().stdout_eq(str![[r#"
...
Compiling 1 files with [..]
...
[PASS] test_Increment_In_Counter() (gas: [..])
[PASS] test_Increment_In_Counter_A() (gas: [..])
[PASS] test_Increment_In_Counter_A_with_named_args() (gas: [..])
[PASS] test_Increment_In_Counter_B() (gas: [..])
[FAIL: assertion failed: 12345 != 1235] test_Increment_In_Counter_V1() (gas: [..])
...

"#]]);

    // Change CounterA to fail test.
    prj.add_source(
        "CounterA.sol",
        r#"
contract CounterA {
    uint256 public number;
    address public owner;

    constructor(uint256 _newNumber, address _owner) {
        number = _newNumber;
        owner = _owner;
    }

    function increment() public {
        number = 12345;
    }
}
    "#,
    );
    // Only CounterA should be compiled and test should fail.
    cmd.with_no_redact().assert_failure().stdout_eq(str![[r#"
...
Compiling 1 files with [..]
...
[PASS] test_Increment_In_Counter() (gas: [..])
[FAIL: assertion failed: 12345 != 1235] test_Increment_In_Counter_A() (gas: [..])
[FAIL: assertion failed: 12345 != 1235] test_Increment_In_Counter_A_with_named_args() (gas: [..])
[PASS] test_Increment_In_Counter_B() (gas: [..])
[FAIL: assertion failed: 12345 != 1235] test_Increment_In_Counter_V1() (gas: [..])
...

"#]]);

    // Change CounterB to fail test.
    prj.add_source(
        "CounterB.sol",
        r#"
contract CounterB {
    uint256 public number;

    constructor(uint256) {
        number = 100;
    }

    function increment() public {
        number++;
    }
}
    "#,
    );
    // Only CounterB should be compiled and test should fail.
    cmd.with_no_redact().assert_failure().stdout_eq(str![[r#"
...
Compiling 1 files with [..]
...
[PASS] test_Increment_In_Counter() (gas: [..])
[FAIL: assertion failed: 12345 != 1235] test_Increment_In_Counter_A() (gas: [..])
[FAIL: assertion failed: 12345 != 1235] test_Increment_In_Counter_A_with_named_args() (gas: [..])
[FAIL: assertion failed: 101 != 2] test_Increment_In_Counter_B() (gas: [..])
[FAIL: assertion failed: 12345 != 1235] test_Increment_In_Counter_V1() (gas: [..])
...

"#]]);

    // Change Counter to fail test.
    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    uint256 public number;

    function setNumber(uint256 newNumber) public {
        number = newNumber;
    }

    function increment() public {
        number = 12345;
    }
}
    "#,
    );
    // Only Counter should be compiled and test should fail.
    cmd.with_no_redact().assert_failure().stdout_eq(str![[r#"
...
Compiling 1 files with [..]
...
[FAIL: assertion failed: 12345 != 1] test_Increment_In_Counter() (gas: [..])
[FAIL: assertion failed: 12345 != 1235] test_Increment_In_Counter_A() (gas: [..])
[FAIL: assertion failed: 12345 != 1235] test_Increment_In_Counter_A_with_named_args() (gas: [..])
[FAIL: assertion failed: 101 != 2] test_Increment_In_Counter_B() (gas: [..])
[FAIL: assertion failed: 12345 != 1235] test_Increment_In_Counter_V1() (gas: [..])
...

"#]]);
});

// Test preprocessing contracts with payable constructor, value and salt named args.
forgetest_init!(flaky_preprocess_contracts_with_payable_constructor_and_salt, |prj, cmd| {
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
    });

    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    uint256 public number;

    constructor(uint256 _number) payable {
        number = msg.value;
    }

    function setNumber(uint256 newNumber) public {
        number = newNumber;
    }

    function increment() public {
        number++;
    }
}
    "#,
    );
    prj.add_source(
        "CounterWithSalt.sol",
        r#"
contract CounterWithSalt {
    uint256 public number;

    constructor(uint256 _number) payable {
        number = msg.value;
    }

    function setNumber(uint256 newNumber) public {
        number = newNumber;
    }

    function increment() public {
        number++;
    }
}
    "#,
    );

    prj.add_test(
        "Counter.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Counter} from "src/Counter.sol";
import {CounterWithSalt} from "src/CounterWithSalt.sol";

contract CounterTest is Test {
    function test_Increment_In_Counter() public {
        Counter counter = Counter(address(new Counter{value: 111}(1)));
        counter.increment();
        assertEq(counter.number(), 112);
    }

    function test_Increment_In_Counter_With_Salt() public {
        CounterWithSalt counter = new CounterWithSalt{value: 111, salt: bytes32("preprocess_counter_with_salt")}(1);
        assertGt(uint160(address(counter)), 0);
        counter.increment();
        assertEq(counter.number(), 112);
    }
}
    "#,
    );

    cmd.args(["test"]).with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 22 files with [..]
...
[PASS] test_Increment_In_Counter() (gas: [..])
[PASS] test_Increment_In_Counter_With_Salt() (gas: [..])
...

"#]]);

    // Change contract to fail test.
    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    uint256 public number;

    constructor(uint256 _number) payable {
        number = msg.value + _number;
    }

    function setNumber(uint256 newNumber) public {
        number = newNumber;
    }

    function increment() public {
        number++;
    }
}
    "#,
    );
    // Only Counter should be compiled and test should fail.
    cmd.with_no_redact().assert_failure().stdout_eq(str![[r#"
...
Compiling 1 files with [..]
...
[FAIL: assertion failed: 113 != 112] test_Increment_In_Counter() (gas: [..])
[PASS] test_Increment_In_Counter_With_Salt() (gas: [..])
...

"#]]);

    // Change contract with salt to fail test too.
    prj.add_source(
        "CounterWithSalt.sol",
        r#"
contract CounterWithSalt {
    uint256 public number;

    constructor(uint256 _number) payable {
        number = msg.value + _number;
    }

    function setNumber(uint256 newNumber) public {
        number = newNumber;
    }

    function increment() public {
        number++;
    }
}
    "#,
    );
    // Only Counter should be compiled and test should fail.
    cmd.with_no_redact().assert_failure().stdout_eq(str![[r#"
...
Compiling 1 files with [..]
...
[FAIL: assertion failed: 113 != 112] test_Increment_In_Counter() (gas: [..])
[FAIL: assertion failed: 113 != 112] test_Increment_In_Counter_With_Salt() (gas: [..])
...

"#]]);
});

// Counter contract with constructor reverts and emitted events.
forgetest_init!(preprocess_contract_with_require_and_emit, |prj, cmd| {
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
    });

    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    event CounterCreated(uint256 number);
    uint256 public number;

    constructor(uint256 no) {
        require(no != 1, "ctor revert");
        emit CounterCreated(10);
    }
}
    "#,
    );

    prj.add_test(
        "Counter.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Counter} from "../src/Counter.sol";

contract CounterTest is Test {
    function test_assert_constructor_revert() public {
        vm.expectRevert("ctor revert");
        new Counter(1);
    }

    function test_assert_constructor_emit() public {
        vm.expectEmit(true, true, true, true);
        emit Counter.CounterCreated(10);

        new Counter(11);
    }
}
    "#,
    );
    // All 20 files are compiled on first run.
    cmd.args(["test"]).with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 21 files with [..]
...

"#]]);

    // Change Counter implementation to revert with different message.
    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    event CounterCreated(uint256 number);
    uint256 public number;

    constructor(uint256 no) {
        require(no != 1, "ctor revert update");
        emit CounterCreated(10);
    }
}
    "#,
    );
    // Assert that only 1 file is compiled (Counter source contract) and revert test fails.
    cmd.with_no_redact().assert_failure().stdout_eq(str![[r#"
...
Compiling 1 files with [..]
...
[PASS] test_assert_constructor_emit() (gas: [..])
[FAIL: Error != expected error: ctor revert update != ctor revert] test_assert_constructor_revert() (gas: [..])
...

"#]]);

    // Change Counter implementation and don't revert.
    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    event CounterCreated(uint256 number);
    uint256 public number;

    constructor(uint256 no) {
        require(no != 0, "ctor revert");
        emit CounterCreated(10);
    }
}
    "#,
    );
    // Assert that only 1 file is compiled (Counter source contract) and revert test fails.
    cmd.with_no_redact().assert_failure().stdout_eq(str![[r#"
...
Compiling 1 files with [..]
...
[PASS] test_assert_constructor_emit() (gas: [..])
[FAIL: next call did not revert as expected] test_assert_constructor_revert() (gas: [..])
...

"#]]);

    // Change Counter implementation to emit different event.
    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    event CounterCreated(uint256 number);
    uint256 public number;

    constructor(uint256 no) {
        require(no != 0, "ctor revert");
        emit CounterCreated(100);
    }
}
    "#,
    );
    // Assert that only 1 file is compiled (Counter source contract) and emit test fails.
    cmd.with_no_redact().assert_failure().stdout_eq(str![[r#"
...
Compiling 1 files with [..]
...
[FAIL: expected an emit, but no logs were emitted afterwards. you might have mismatched events or not enough events were emitted] test_assert_constructor_emit() (gas: [..])
[FAIL: next call did not revert as expected] test_assert_constructor_revert() (gas: [..])
...

"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/10312>
forgetest_init!(preprocess_contract_with_constructor_args_struct, |prj, cmd| {
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
    });

    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    struct ConstructorArgs {
        uint256 _number;
    }

    constructor(uint256 no) {
    }
}
    "#,
    );

    prj.add_test(
        "Counter.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Counter} from "../src/Counter.sol";

contract CounterTest is Test {
    function test_assert_constructor_revert() public {
        Counter counter = new Counter(1);
    }
}
    "#,
    );
    // All 20 files should properly compile.
    cmd.args(["test"]).with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 21 files with [..]
...

"#]]);
});

// Test preprocessed contracts with decode internal fns.
forgetest_init!(preprocess_contract_with_decode_internal, |prj, cmd| {
    prj.initialize_default_contracts();
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
    });

    prj.add_test(
        "Counter.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Counter} from "../src/Counter.sol";

contract CounterTest is Test {
    Counter public counter;

    function setUp() public {
        create_counter(0);
    }

    function test_Increment() public {
        create_counter(0);
        counter.increment();
        assertEq(counter.number(), 1);
    }

    function create_counter(uint256 number) internal {
        counter = new Counter();
        counter.setNumber(number);
    }
}
    "#,
    );

    cmd.args(["test", "--decode-internal", "-vvvv"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/Counter.t.sol:CounterTest
[PASS] test_Increment() ([GAS])
Traces:
  [..] CounterTest::test_Increment()
    ├─ [0] VM::deployCode("src/Counter.sol:Counter")
    │   ├─ [96345] → new Counter@0xF62849F9A0B5Bf2913b396098F7c7019b51A820a
    │   │   └─ ← [Return] 481 bytes of code
    │   └─ ← [Return] Counter: [0xF62849F9A0B5Bf2913b396098F7c7019b51A820a]
    ├─ [..] Counter::setNumber(0)
    │   └─ ← [Stop]
    ├─ [..] Counter::increment()
    │   └─ ← [Stop]
    ├─ [..] Counter::number() [staticcall]
    │   └─ ← [Return] 1
    ├─ [..] StdAssertions::assertEq(uint256,uint256)(1, 1)
    │   └─ ← 
    └─ ← [Stop]

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/10492>
// Preprocess test contracts with try constructor statements.
forgetest_init!(preprocess_contract_with_try_ctor_stmt, |prj, cmd| {
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
    });

    prj.add_source(
        "CounterA.sol",
        r#"
contract CounterA {
    uint256 number;
}
    "#,
    );
    prj.add_source(
        "CounterB.sol",
        r#"
contract CounterB {
    uint256 number;
    constructor(uint256 a) payable {
        require(a > 0, "ctor failure");
        number = a;
    }
}
    "#,
    );
    prj.add_source(
        "CounterC.sol",
        r#"
contract CounterC {
    uint256 number;
    constructor(uint256 a) {
        require(a > 0, "ctor failure");
        number = a;
    }
}
    "#,
    );

    prj.add_test(
        "Counter.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {CounterA} from "../src/CounterA.sol";
import {CounterB} from "../src/CounterB.sol";
import {CounterC} from "../src/CounterC.sol";

contract CounterTest is Test {
    function test_try_counterA_creation() public {
        try new CounterA() {} catch {
            revert();
        }
    }

    function test_try_counterB_creation() public {
        try new CounterB(1) {} catch {
            revert();
        }
    }

    function test_try_counterB_creation_with_salt() public {
        try new CounterB{value: 111, salt: bytes32("preprocess_counter_with_salt")}(1) {} catch {
            revert();
        }
    }

    function test_try_counterC_creation() public {
        try new CounterC(2) {
            new CounterC(1);
        } catch {
            revert();
        }
    }
}
    "#,
    );
    // All 23 files should properly compile, tests pass.
    cmd.args(["test"]).with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 23 files with [..]
...
[PASS] test_try_counterA_creation() (gas: [..])
[PASS] test_try_counterB_creation() (gas: [..])
[PASS] test_try_counterB_creation_with_salt() (gas: [..])
[PASS] test_try_counterC_creation() (gas: [..])
...

"#]]);

    // Change CounterB to fail test.
    prj.add_source(
        "CounterB.sol",
        r#"
contract CounterB {
    uint256 number;
    constructor(uint256 a) payable {
        require(a > 11, "ctor failure");
        number = a;
    }
}
    "#,
    );
    // Only CounterB should compile.
    cmd.assert_failure().stdout_eq(str![[r#"
...
Compiling 1 files with [..]
...
[PASS] test_try_counterA_creation() (gas: [..])
[FAIL: EvmError: Revert] test_try_counterB_creation() (gas: [..])
[FAIL: EvmError: Revert] test_try_counterB_creation_with_salt() (gas: [..])
[PASS] test_try_counterC_creation() (gas: [..])
...

"#]]);

    // Change CounterC to fail test in try statement.
    prj.add_source(
        "CounterC.sol",
        r#"
contract CounterC {
    uint256 number;
    constructor(uint256 a) {
        require(a > 1, "ctor failure");
        number = a;
    }
}
    "#,
    );
    // Only CounterC should compile.
    cmd.assert_failure().stdout_eq(str![[r#"
...
Compiling 1 files with [..]
...
[PASS] test_try_counterA_creation() (gas: [..])
[FAIL: EvmError: Revert] test_try_counterB_creation() (gas: [..])
[FAIL: EvmError: Revert] test_try_counterB_creation_with_salt() (gas: [..])
[FAIL: ctor failure] test_try_counterC_creation() (gas: [..])
...

"#]]);

    // Change CounterC to fail test in try statement.
    prj.add_source(
        "CounterC.sol",
        r#"
contract CounterC {
    uint256 number;
    constructor(uint256 a) {
        require(a > 2, "ctor failure");
        number = a;
    }
}
    "#,
    );
    // Only CounterC should compile and revert.
    cmd.assert_failure().stdout_eq(str![[r#"
...
Compiling 1 files with [..]
...
[PASS] test_try_counterA_creation() (gas: [..])
[FAIL: EvmError: Revert] test_try_counterB_creation() (gas: [..])
[FAIL: EvmError: Revert] test_try_counterB_creation_with_salt() (gas: [..])
[FAIL: EvmError: Revert] test_try_counterC_creation() (gas: [..])
...

"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/11978>
// Preprocess test contracts when active prank.
forgetest_init!(preprocess_contract_with_active_prank, |prj, cmd| {
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
    });

    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    uint256 public number;
    address public deployer;
    address public origin;

    constructor() {
        deployer = msg.sender;
        origin = tx.origin;
    }
}
    "#,
    );

    prj.add_test(
        "Counter.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Counter} from "../src/Counter.sol";

contract CounterTest is Test {
    function test_deployer() public {
        address deployer = makeAddr("deployer");
        address origin = makeAddr("origin");
        vm.startPrank(deployer, origin);
        Counter first = new Counter{salt: 0}();
        Counter second = new Counter{salt: bytes32(uint256(1))}();
        assertEq(first.deployer(), deployer);
        assertEq(first.origin(), origin);
        assertEq(second.deployer(), deployer);
        assertEq(second.origin(), origin);
    }

    function test_consecutive_single_call_pranks() public {
        address firstDeployer = makeAddr("firstDeployer");
        address firstOrigin = makeAddr("firstOrigin");
        vm.prank(firstDeployer, firstOrigin);
        Counter first = new Counter();

        address secondDeployer = makeAddr("secondDeployer");
        address secondOrigin = makeAddr("secondOrigin");
        vm.prank(secondDeployer, secondOrigin);
        Counter second = new Counter();

        assertEq(first.deployer(), firstDeployer);
        assertEq(first.origin(), firstOrigin);
        assertEq(second.deployer(), secondDeployer);
        assertEq(second.origin(), secondOrigin);

        Counter unpranked = new Counter();
        assertEq(unpranked.deployer(), address(this));
        assertEq(unpranked.origin(), tx.origin);
    }
}
    "#,
    );
    // Test should pass.
    cmd.args(["test"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 2 tests for test/Counter.t.sol:CounterTest
[PASS] test_consecutive_single_call_pranks() ([GAS])
[PASS] test_deployer() ([GAS])
Suite result: ok. 2 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 2 tests passed, 0 failed, 0 skipped (2 total tests)

"#]]);
});

// Preprocess test contracts with try constructor statements that bind return type.
forgetest_init!(preprocess_contract_with_try_ctor_stmt_and_returns, |prj, cmd| {
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
    });

    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    uint256 number;
    constructor(uint256 a) payable {
        require(a > 0, "ctor failure");
        number = a;
    }
}
        "#,
    );
    prj.add_test(
        "CounterReturns.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Counter} from "../src/Counter.sol";

contract CounterReturnsTest is Test {
    function test_try_counter_creation_returns_custom_type() public {
        try new Counter(1) returns (Counter c) {
            c;
        } catch {
            revert();
        }
    }
}
        "#,
    );

    cmd.args(["test"]).with_no_redact().assert_success().stdout_eq(str![[r#"
...
Compiling 21 files with [..]
...
[PASS] test_try_counter_creation_returns_custom_type() (gas: [..])
...

"#]]);

    // Change Counter to fail test in try statement, only Counter contract should be compiled.
    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    uint256 number;
    constructor(uint256 a) payable {
        require(a == 0, "ctor failure");
        number = a;
    }
}
        "#,
    );
    cmd.assert_failure().stdout_eq(str![[r#"
...
Compiling 1 files with [..]
...
[FAIL: ctor failure] test_try_counter_creation_returns_custom_type() (gas: [..])
...

"#]]);
});

// Test that `type(Contract).creationCode` can be used in view functions.
// https://github.com/foundry-rs/foundry/issues/13086
forgetest_init!(preprocess_creation_code_in_view_function, |prj, cmd| {
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
    });

    prj.add_source(
        "Target.sol",
        r#"
contract Target {
    uint256 public immutable value;
    constructor(uint256 _value) { value = _value; }
}
        "#,
    );

    prj.add_test(
        "Target.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Target} from "../src/Target.sol";

contract TargetTest is Test {
    function computeAddress(address factory, uint256 salt, uint256 value) internal view returns (address) {
        bytes32 hash = keccak256(
            abi.encodePacked(
                bytes1(0xff),
                factory,
                salt,
                keccak256(abi.encodePacked(type(Target).creationCode, abi.encode(value)))
            )
        );
        return address(uint160(uint256(hash)));
    }

    function testComputeAddress() public view {
        computeAddress(address(this), 1, 100);
    }
}
        "#,
    );

    cmd.args(["build"]).assert_success();
});

// <https://github.com/foundry-rs/foundry/issues/16487>
forgetest_init!(preprocess_custom_layout_contract, |prj, cmd| {
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
        config.solc = Some(foundry_config::SolcReq::Version(semver::Version::new(0, 8, 35)));
    });

    prj.add_source(
        "Target.sol",
        r#"
contract Target layout at erc7201("test.Target") {
    uint256 public value;

    constructor(uint256 value_) {
        value = value_;
    }
}
        "#,
    );

    prj.add_test(
        "Target.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Target} from "../src/Target.sol";

contract TargetTest is Test {
    function testDirectNew() public {
        Target target = new Target(42);
        assertEq(target.value(), 42);
    }

    function targetCreationCode() public view returns (bytes memory) {
        return type(Target).creationCode;
    }
}
        "#,
    );

    cmd.args(["test"]).assert_success();
});

// Test that `type(Contract).creationCode` keeps native pure semantics when dynamic linking is
// enabled.
forgetest_init!(preprocess_creation_code_in_pure_function, |prj, cmd| {
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
    });

    prj.add_source(
        "Target.sol",
        r#"
contract Target {
    uint256 public immutable value;
    constructor(uint256 _value) { value = _value; }
}
        "#,
    );

    prj.add_test(
        "Target.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Target} from "../src/Target.sol";

contract TargetTest is Test {
    function computeAddress(address factory, uint256 salt, uint256 value) internal pure returns (address) {
        bytes32 hash = keccak256(
            abi.encodePacked(
                bytes1(0xff),
                factory,
                salt,
                keccak256(abi.encodePacked(type(Target).creationCode, abi.encode(value)))
            )
        );
        return address(uint160(uint256(hash)));
    }

    function testComputeAddress() public pure {
        computeAddress(address(0xBEEF), 1, 100);
    }
}
        "#,
    );

    cmd.args(["build"]).assert_success();
});

// Test that `type(Contract).creationCode` keeps native pure semantics when it is used in a
// modifier body that is applied to a pure function.
forgetest_init!(preprocess_creation_code_in_modifier_used_by_pure_function, |prj, cmd| {
    prj.update_config(|config| {
        config.dynamic_test_linking = true;
    });

    prj.add_source(
        "Target.sol",
        r#"
contract Target {}
        "#,
    );

    prj.add_test(
        "ModifierCreationCode.t.sol",
        r#"
import {Target} from "../src/Target.sol";

contract ModifierCreationCodeTest {
    modifier usesCreationCode() {
        bytes memory code = type(Target).creationCode;
        code;
        _;
    }

    function testModifierCreationCode() public pure usesCreationCode {}
}
        "#,
    );

    cmd.args(["build"]).assert_success();
});
