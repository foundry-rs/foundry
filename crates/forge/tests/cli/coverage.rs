use clap::CommandFactory;
use forge::cmd::coverage::CoverageArgs;
use foundry_common::fs::{self, files_with_ext};
use foundry_compilers::cache::SOLIDITY_FILES_CACHE_FILENAME;
use foundry_config::{CompilationRestrictions, Config, SettingsOverrides};
use foundry_test_utils::{
    TestCommand, TestProject,
    snapbox::{Data, IntoData, cmd::Command},
    util::OutputExt,
};
use serde_json::Value;
use std::path::Path;

#[track_caller]
fn assert_lcov(cmd: &mut TestCommand, data: impl IntoData) {
    cmd.args(["--report=lcov", "--report-file"]).assert_file_with(
        |cmd, path| {
            cmd.arg(path).assert_success();
            assert_genhtml(cmd.cmd().get_current_dir().unwrap(), path);
        },
        data,
    );
}

/// Validate reports with a real consumer when requested (required on Linux CI).
#[track_caller]
fn assert_genhtml(root: &Path, report: &Path) {
    let Some(genhtml) = std::env::var_os("FOUNDRY_TEST_GENHTML").filter(|s| !s.is_empty()) else {
        return;
    };
    let output = tempfile::tempdir().unwrap();
    // Do not inherit user settings that could disable consistency checks or ignore errors.
    let config = tempfile::NamedTempFile::new().unwrap();
    Command::new(genhtml)
        .current_dir(root)
        .arg(report)
        .arg("--config-file")
        .arg(config.path())
        .args(["--branch-coverage", "--output-directory"])
        .arg(output.path())
        .assert()
        .success();
    assert!(output.path().join("index.html").is_file());
}

fn basic_base(prj: TestProject, mut cmd: TestCommand) {
    cmd.args(["coverage", "--report=lcov", "--report=summary"]).assert_success().stdout_eq(str![[
        r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!
Analysing contracts...
Running tests...

Ran 2 tests for test/Counter.t.sol:CounterTest
[PASS] testFuzz_SetNumber(uint256) (runs: 256, [AVG_GAS])
[PASS] test_Increment() ([GAS])
Suite result: ok. 2 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 2 tests passed, 0 failed, 0 skipped (2 total tests)
Wrote LCOV report.

╭----------------------+---------------+---------------+------------+---------------╮
| File                 | % Lines       | % Statements  | % Branches | % Funcs       |
+===================================================================================+
| script/Counter.s.sol | 0.00% (0/5)   | 0.00% (0/3)   | N/A (0/0)  | 0.00% (0/2)   |
|----------------------+---------------+---------------+------------+---------------|
| src/Counter.sol      | 100.00% (4/4) | 100.00% (2/2) | N/A (0/0)  | 100.00% (2/2) |
|----------------------+---------------+---------------+------------+---------------|
| Total                | 44.44% (4/9)  | 40.00% (2/5)  | N/A (0/0)  | 50.00% (2/4)  |
╰----------------------+---------------+---------------+------------+---------------╯

"#
    ]]);

    let lcov = prj.root().join("lcov.info");
    assert!(lcov.exists(), "lcov.info was not created");
    let default_lcov = str![[r#"
TN:
SF:script/Counter.s.sol
DA:10,0
FN:10,CounterScript.setUp
FNDA:0,CounterScript.setUp
DA:12,0
FN:12,CounterScript.run
FNDA:0,CounterScript.run
DA:13,0
DA:15,0
DA:17,0
FNF:2
FNH:0
LF:5
LH:0
BRF:0
BRH:0
end_of_record
TN:
SF:src/Counter.sol
DA:7,258
FN:7,Counter.setNumber
FNDA:258,Counter.setNumber
DA:8,258
DA:11,1
FN:11,Counter.increment
FNDA:1,Counter.increment
DA:12,1
FNF:2
FNH:2
LF:4
LH:4
BRF:0
BRH:0
end_of_record

"#]];
    assert_data_eq!(Data::read_from(&lcov, None), default_lcov.clone());
    assert_lcov(
        cmd.forge_fuse().args(["coverage", "--report=lcov", "--lcov-version=1"]),
        default_lcov,
    );

    assert_lcov(
        cmd.forge_fuse().args(["coverage", "--report=lcov", "--lcov-version=2"]),
        str![[r#"
TN:
SF:script/Counter.s.sol
DA:10,0
FN:10,10,CounterScript.setUp
FNDA:0,CounterScript.setUp
DA:12,0
FN:12,18,CounterScript.run
FNDA:0,CounterScript.run
DA:13,0
DA:15,0
DA:17,0
FNF:2
FNH:0
LF:5
LH:0
BRF:0
BRH:0
end_of_record
TN:
SF:src/Counter.sol
DA:7,258
FN:7,9,Counter.setNumber
FNDA:258,Counter.setNumber
DA:8,258
DA:11,1
FN:11,13,Counter.increment
FNDA:1,Counter.increment
DA:12,1
FNF:2
FNH:2
LF:4
LH:4
BRF:0
BRH:0
end_of_record

"#]],
    );

    assert_lcov(
        cmd.forge_fuse().args(["coverage", "--report=lcov", "--lcov-version=2.2"]),
        str![[r#"
TN:
SF:script/Counter.s.sol
DA:10,0
FNL:0,10,10
FNA:0,0,CounterScript.setUp
DA:12,0
FNL:1,12,18
FNA:1,0,CounterScript.run
DA:13,0
DA:15,0
DA:17,0
FNF:2
FNH:0
LF:5
LH:0
BRF:0
BRH:0
end_of_record
TN:
SF:src/Counter.sol
DA:7,258
FNL:2,7,9
FNA:2,258,Counter.setNumber
DA:8,258
DA:11,1
FNL:3,11,13
FNA:3,1,Counter.increment
DA:12,1
FNF:2
FNH:2
LF:4
LH:4
BRF:0
BRH:0
end_of_record

"#]],
    );
}

#[forgetest_init]
fn basic(prj: _, cmd: _) {
    prj.initialize_default_contracts();
    basic_base(prj, cmd);
}

#[forgetest_init]
fn basic_crlf(prj: _, cmd: _) {
    prj.initialize_default_contracts();
    // Manually replace `\n` with `\r\n` in the source file.
    let make_crlf = |path: &Path| {
        fs::write(path, fs::read_to_string(path).unwrap().replace('\n', "\r\n")).unwrap()
    };
    make_crlf(&prj.paths().sources.join("Counter.sol"));
    make_crlf(&prj.paths().scripts.join("Counter.s.sol"));

    // Should have identical stdout and lcov output.
    basic_base(prj, cmd);
}

#[forgetest]
fn setup(prj: _, cmd: _) {
    prj.insert_ds_test();
    prj.add_source(
        "AContract.sol",
        r#"
contract AContract {
    int public i;

    function init() public {
        i = 0;
    }

    function foo() public {
        i = 1;
    }
}
    "#,
    );

    prj.add_source(
        "AContractTest.sol",
        r#"
import "./test.sol";
import {AContract} from "./AContract.sol";

contract AContractTest is DSTest {
    AContract a;

    function setUp() public {
        a = new AContract();
        a.init();
    }

    function testFoo() public {
        a.foo();
    }
}
    "#,
    );

    // Assert 100% coverage (init function coverage called in setUp is accounted).
    cmd.arg("coverage").assert_success().stdout_eq(str![[r#"
...
╭-------------------+---------------+---------------+------------+---------------╮
| File              | % Lines       | % Statements  | % Branches | % Funcs       |
+================================================================================+
| src/AContract.sol | 100.00% (4/4) | 100.00% (2/2) | N/A (0/0)  | 100.00% (2/2) |
|-------------------+---------------+---------------+------------+---------------|
| Total             | 100.00% (4/4) | 100.00% (2/2) | N/A (0/0)  | 100.00% (2/2) |
╰-------------------+---------------+---------------+------------+---------------╯

"#]]);
}

#[forgetest]
fn setup_md(prj: _, cmd: _) {
    prj.insert_ds_test();
    prj.add_source(
        "AContract.sol",
        r#"
contract AContract {
    int public i;

    function init() public {
        i = 0;
    }

    function foo() public {
        i = 1;
    }
}
    "#,
    );

    prj.add_source(
        "AContractTest.sol",
        r#"
import "./test.sol";
import {AContract} from "./AContract.sol";

contract AContractTest is DSTest {
    AContract a;

    function setUp() public {
        a = new AContract();
        a.init();
    }

    function testFoo() public {
        a.foo();
    }
}
    "#,
    );

    // Assert 100% coverage (init function coverage called in setUp is accounted).
    cmd.arg("coverage").args(["--md"]).assert_success().stdout_eq(str![[r#"
...
| File              | % Lines       | % Statements  | % Branches | % Funcs       |
|-------------------|---------------|---------------|------------|---------------|
| src/AContract.sol | 100.00% (4/4) | 100.00% (2/2) | N/A (0/0)  | 100.00% (2/2) |
| Total             | 100.00% (4/4) | 100.00% (2/2) | N/A (0/0)  | 100.00% (2/2) |

"#]]);
}

#[forgetest]
fn no_match(prj: _, cmd: _) {
    prj.insert_ds_test();
    prj.add_source(
        "AContract.sol",
        r#"
contract AContract {
    int public i;

    function init() public {
        i = 0;
    }

    function foo() public {
        i = 1;
    }
}
    "#,
    );

    prj.add_source(
        "AContractTest.sol",
        r#"
import "./test.sol";
import {AContract} from "./AContract.sol";

contract AContractTest is DSTest {
    AContract a;

    function setUp() public {
        a = new AContract();
        a.init();
    }

    function testFoo() public {
        a.foo();
    }
}
    "#,
    );

    prj.add_source(
        "BContract.sol",
        r#"
contract BContract {
    int public i;

    function init() public {
        i = 0;
    }

    function foo() public {
        i = 1;
    }
}
    "#,
    );

    prj.add_source(
        "BContractTest.sol",
        r#"
import "./test.sol";
import {BContract} from "./BContract.sol";

contract BContractTest is DSTest {
    BContract a;

    function setUp() public {
        a = new BContract();
        a.init();
    }

    function testFoo() public {
        a.foo();
    }
}
    "#,
    );

    // Assert AContract is not included in report.
    cmd.arg("coverage").arg("--no-match-coverage=AContract").assert_success().stdout_eq(str![[
        r#"
...
╭-------------------+---------------+---------------+------------+---------------╮
| File              | % Lines       | % Statements  | % Branches | % Funcs       |
+================================================================================+
| src/BContract.sol | 100.00% (4/4) | 100.00% (2/2) | N/A (0/0)  | 100.00% (2/2) |
|-------------------+---------------+---------------+------------+---------------|
| Total             | 100.00% (4/4) | 100.00% (2/2) | N/A (0/0)  | 100.00% (2/2) |
╰-------------------+---------------+---------------+------------+---------------╯

"#
    ]]);
}

// `[profile.default.coverage] skip_files` should exclude matching sources from
// the coverage report just like `--no-match-coverage`, but using glob patterns
// from `foundry.toml`.
#[forgetest]
fn skip_files_via_config(prj: _, cmd: _) {
    prj.insert_ds_test();
    prj.add_source(
        "AContract.sol",
        r#"
contract AContract {
    int public i;

    function init() public {
        i = 0;
    }

    function foo() public {
        i = 1;
    }
}
    "#,
    );

    prj.add_source(
        "AContractTest.sol",
        r#"
import "./test.sol";
import {AContract} from "./AContract.sol";

contract AContractTest is DSTest {
    AContract a;

    function setUp() public {
        a = new AContract();
        a.init();
    }

    function testFoo() public {
        a.foo();
    }
}
    "#,
    );

    prj.add_source(
        "BContract.sol",
        r#"
contract BContract {
    int public i;

    function init() public {
        i = 0;
    }

    function foo() public {
        i = 1;
    }
}
    "#,
    );

    prj.add_source(
        "BContractTest.sol",
        r#"
import "./test.sol";
import {BContract} from "./BContract.sol";

contract BContractTest is DSTest {
    BContract a;

    function setUp() public {
        a = new BContract();
        a.init();
    }

    function testFoo() public {
        a.foo();
    }
}
    "#,
    );

    prj.update_config(|config| {
        config.coverage.skip_files =
            vec!["src/AContract.sol".into(), "src/AContractTest.sol".into()];
    });

    // Assert AContract.sol is excluded via the config glob; BContract.sol still
    // appears in the report.
    cmd.arg("coverage").assert_success().stdout_eq(str![[r#"
...
╭-------------------+---------------+---------------+------------+---------------╮
| File              | % Lines       | % Statements  | % Branches | % Funcs       |
+================================================================================+
| src/BContract.sol | 100.00% (4/4) | 100.00% (2/2) | N/A (0/0)  | 100.00% (2/2) |
|-------------------+---------------+---------------+------------+---------------|
| Total             | 100.00% (4/4) | 100.00% (2/2) | N/A (0/0)  | 100.00% (2/2) |
╰-------------------+---------------+---------------+------------+---------------╯

"#]]);
}

#[forgetest]
fn assert(prj: _, cmd: _) {
    prj.insert_ds_test();
    prj.add_source(
        "AContract.sol",
        r#"
contract AContract {
    function checkA(uint256 a) external pure returns (bool) {
        assert(a > 2);
        return true;
    }
}
    "#,
    );

    prj.add_source(
        "AContractTest.sol",
        r#"
import "./test.sol";
import {AContract} from "./AContract.sol";

interface Vm {
    function expectRevert() external;
}

contract AContractTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);
    AContract a = new AContract();

    function testAssertBranch() external {
        bool result = a.checkA(10);
        assertTrue(result);
    }

    function testAssertRevertBranch() external {
        vm.expectRevert();
        a.checkA(1);
    }
}
    "#,
    );

    // Assert 50% statement coverage for assert failure (assert not considered a branch).
    cmd.arg("coverage").args(["--mt", "testAssertRevertBranch"]).assert_success().stdout_eq(str![
        [r#"
...
╭-------------------+--------------+--------------+------------+---------------╮
| File              | % Lines      | % Statements | % Branches | % Funcs       |
+==============================================================================+
| src/AContract.sol | 66.67% (2/3) | 50.00% (1/2) | N/A (0/0)  | 100.00% (1/1) |
|-------------------+--------------+--------------+------------+---------------|
| Total             | 66.67% (2/3) | 50.00% (1/2) | N/A (0/0)  | 100.00% (1/1) |
╰-------------------+--------------+--------------+------------+---------------╯

"#]
    ]);

    // Assert 100% statement coverage for proper assert (assert not considered a branch).
    cmd.forge_fuse().arg("coverage").args(["--mt", "testAssertBranch"]).assert_success().stdout_eq(
        str![[r#"
...
╭-------------------+---------------+---------------+------------+---------------╮
| File              | % Lines       | % Statements  | % Branches | % Funcs       |
+================================================================================+
| src/AContract.sol | 100.00% (3/3) | 100.00% (2/2) | N/A (0/0)  | 100.00% (1/1) |
|-------------------+---------------+---------------+------------+---------------|
| Total             | 100.00% (3/3) | 100.00% (2/2) | N/A (0/0)  | 100.00% (1/1) |
╰-------------------+---------------+---------------+------------+---------------╯

"#]],
    );
}

#[forgetest]
fn require(prj: _, cmd: _) {
    prj.insert_ds_test();
    prj.add_source(
        "AContract.sol",
        r#"
contract AContract {
    function checkRequire(bool doNotRevert) public view {
        require(doNotRevert, "reverted");
    }
}
    "#,
    );

    prj.add_source(
        "AContractTest.sol",
        r#"
import "./test.sol";
import {AContract} from "./AContract.sol";

interface Vm {
    function expectRevert(bytes calldata revertData) external;
}

contract AContractTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);
    AContract a = new AContract();

    function testRequireRevert() external {
        vm.expectRevert(abi.encodePacked("reverted"));
        a.checkRequire(false);
    }

    function testRequireNoRevert() external {
        a.checkRequire(true);
    }
}
    "#,
    );

    // Assert 50% branch coverage if only revert tested.
    cmd.arg("coverage").args(["--mt", "testRequireRevert"]).assert_success().stdout_eq(str![[r#"
...
╭-------------------+---------------+---------------+--------------+---------------╮
| File              | % Lines       | % Statements  | % Branches   | % Funcs       |
+==================================================================================+
| src/AContract.sol | 100.00% (2/2) | 100.00% (1/1) | 50.00% (1/2) | 100.00% (1/1) |
|-------------------+---------------+---------------+--------------+---------------|
| Total             | 100.00% (2/2) | 100.00% (1/1) | 50.00% (1/2) | 100.00% (1/1) |
╰-------------------+---------------+---------------+--------------+---------------╯

"#]]);

    // Assert 50% branch coverage if only happy path tested.
    cmd.forge_fuse()
        .arg("coverage")
        .args(["--mt", "testRequireNoRevert"])
        .assert_success()
        .stdout_eq(str![[r#"
...
╭-------------------+---------------+---------------+--------------+---------------╮
| File              | % Lines       | % Statements  | % Branches   | % Funcs       |
+==================================================================================+
| src/AContract.sol | 100.00% (2/2) | 100.00% (1/1) | 50.00% (1/2) | 100.00% (1/1) |
|-------------------+---------------+---------------+--------------+---------------|
| Total             | 100.00% (2/2) | 100.00% (1/1) | 50.00% (1/2) | 100.00% (1/1) |
╰-------------------+---------------+---------------+--------------+---------------╯

"#]]);

    // Assert 100% branch coverage.
    cmd.forge_fuse().arg("coverage").assert_success().stdout_eq(str![[r#"
...
╭-------------------+---------------+---------------+---------------+---------------╮
| File              | % Lines       | % Statements  | % Branches    | % Funcs       |
+===================================================================================+
| src/AContract.sol | 100.00% (2/2) | 100.00% (1/1) | 100.00% (2/2) | 100.00% (1/1) |
|-------------------+---------------+---------------+---------------+---------------|
| Total             | 100.00% (2/2) | 100.00% (1/1) | 100.00% (2/2) | 100.00% (1/1) |
╰-------------------+---------------+---------------+---------------+---------------╯

"#]]);
}

// A custom-error require whose success path reaches `INVALID` keeps path 0 as the failing side.
#[forgetest]
fn require_custom_error_then_invalid_lcov(prj: _, cmd: _) {
    prj.add_source(
        "Guard.sol",
        r#"
contract Guard {
    error TooLarge(uint256 x);

    function check(uint256 x) external pure {
        require(x < 100, TooLarge(1));
        assembly { invalid() }
    }
}
"#,
    );
    prj.add_test(
        "Guard.t.sol",
        r#"
import "../src/Guard.sol";
contract GuardTest {
    function testPaths() public {
        Guard guard = new Guard();
        address(guard).call(abi.encodeCall(guard.check, (1)));
        address(guard).call(abi.encodeCall(guard.check, (2)));
        address(guard).call(abi.encodeCall(guard.check, (100)));
    }
}
"#,
    );
    assert_lcov(
        cmd.arg("coverage"),
        str![[r#"
TN:
SF:src/Guard.sol
DA:7,3
FN:7,Guard.check
FNDA:3,Guard.check
DA:8,3
BRDA:8,0,0,1
BRDA:8,0,1,2
DA:9,2
FNF:1
FNH:1
LF:3
LH:3
BRF:2
BRH:2
end_of_record

"#]],
    );
}

// The failing side of a require is path 0 with solc and with solar, whose layout differs.
#[forgetest]
fn require_paths_solc_and_solar_lcov(prj: _, cmd: _) {
    prj.add_source(
        "Guard.sol",
        r#"
contract Guard {
    function req(uint256 a) external pure returns (uint256) {
        require(a < 100);
        return a + 1;
    }
}
"#,
    );
    prj.add_test(
        "Guard.t.sol",
        r#"
import "../src/Guard.sol";
contract GuardTest {
    function testPaths() public {
        Guard guard = new Guard();
        guard.req(1);
        guard.req(2);
        guard.req(3);
    }
}
"#,
    );
    let solar = prj.ensure_foundry_bin("solar");
    let report = str![[r#"
TN:
SF:src/Guard.sol
DA:5,3
FN:5,Guard.req
FNDA:3,Guard.req
DA:6,3
BRDA:6,0,0,0
BRDA:6,0,1,3
DA:7,3
FNF:1
FNH:1
LF:3
LH:3
BRF:2
BRH:1
end_of_record

"#]];
    assert_lcov(cmd.arg("coverage"), report.clone());
    assert_lcov(cmd.forge_fuse().arg("coverage").arg("--use").arg(solar), report);
}

#[forgetest]
fn line_hit_not_doubled(prj: _, cmd: _) {
    prj.insert_ds_test();
    prj.add_source(
        "AContract.sol",
        r#"
contract AContract {
    int public i;

    function foo() public {
        i = 1;
    }
}
    "#,
    );

    prj.add_source(
        "AContractTest.sol",
        r#"
import "./test.sol";
import {AContract} from "./AContract.sol";

contract AContractTest is DSTest {
    AContract a = new AContract();

    function testFoo() public {
        a.foo();
    }
}
    "#,
    );

    // We want to make sure DA:8,1 is added only once so line hit is not doubled.
    assert_lcov(
        cmd.arg("coverage"),
        str![[r#"
TN:
SF:src/AContract.sol
DA:7,1
FN:7,AContract.foo
FNDA:1,AContract.foo
DA:8,1
FNF:1
FNH:1
LF:2
LH:2
BRF:0
BRH:0
end_of_record

"#]],
    );
}

#[forgetest]
fn branch(prj: _, cmd: _) {
    prj.insert_ds_test();
    prj.add_source(
        "Foo.sol",
        r#"
contract Foo {
    error Gte1(uint256 number, uint256 firstElement);

    enum Status {
        NULL,
        OPEN,
        CLOSED
    }

    struct Item {
        Status status;
        uint256 value;
    }

    mapping(uint256 => Item) internal items;
    uint256 public nextId = 1;

    function getItem(uint256 id) public view returns (Item memory item) {
        item = items[id];
    }

    function addItem(uint256 value) public returns (uint256 id) {
        id = nextId;
        items[id] = Item(Status.OPEN, value);
        nextId++;
    }

    function closeIfEqValue(uint256 id, uint256 value) public {
        if (items[id].value == value) {
            items[id].status = Status.CLOSED;
        }
    }

    function incrementIfEqValue(uint256 id, uint256 value) public {
        if (items[id].value == value) {
            items[id].value = value + 1;
        }
    }

    function foo(uint256 a) external pure {
        if (a < 10) {
            if (a < 3) {
                assert(a == 1);
            } else {
                assert(a == 5);
            }
        } else {
            assert(a == 60);
        }
    }

    function countOdd(uint256[] memory arr) external pure returns (uint256 count) {
        uint256 length = arr.length;
        for (uint256 i = 0; i < length; ++i) {
            if (arr[i] % 2 == 1) {
                count++;
                arr[0];
            }
        }
    }

    function checkLt(uint256 number, uint256[] memory arr) external pure returns (bool) {
        if (number >= arr[0]) {
            revert Gte1(number, arr[0]);
        }
        return true;
    }

    function checkEmptyStatements(uint256 number, uint256[] memory arr) external pure returns (bool) {
        // Check that empty statements are covered.
        if (number >= arr[0]) {
            // Do nothing
        } else {
            // Do nothing.
        }
        if (number >= arr[0]) {}

        return true;
    }

    function singlePathCoverage(uint256 number) external pure {
        if (number < 10) {
            if (number < 5) {
                number++;
            }
            number++;
        }
    }
}
    "#,
    );

    prj.add_source(
        "FooTest.sol",
        r#"
import "./test.sol";
import {Foo} from "./Foo.sol";

interface Vm {
    function expectRevert(bytes calldata revertData) external;
    function expectRevert() external;
}

contract FooTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);
    Foo internal foo = new Foo();

    function test_issue_7784() external {
        foo.foo(1);
        vm.expectRevert();
        foo.foo(2);
        vm.expectRevert();
        foo.foo(4);
        foo.foo(5);
        foo.foo(60);
        vm.expectRevert();
        foo.foo(70);
    }

    function test_issue_4310() external {
        uint256[] memory arr = new uint256[](3);
        arr[0] = 78;
        arr[1] = 493;
        arr[2] = 700;
        uint256 count = foo.countOdd(arr);
        assertEq(count, 1);

        arr = new uint256[](4);
        arr[0] = 78;
        arr[1] = 493;
        arr[2] = 700;
        arr[3] = 1729;
        count = foo.countOdd(arr);
        assertEq(count, 2);
    }

    function test_issue_4315() external {
        uint256 value = 42;
        uint256 id = foo.addItem(value);
        assertEq(id, 1);
        assertEq(foo.nextId(), 2);
        Foo.Item memory item = foo.getItem(id);
        assertEq(uint8(item.status), uint8(Foo.Status.OPEN));
        assertEq(item.value, value);

        foo = new Foo();
        id = foo.addItem(value);
        foo.closeIfEqValue(id, 903);
        item = foo.getItem(id);
        assertEq(uint8(item.status), uint8(Foo.Status.OPEN));

        foo = new Foo();
        foo.addItem(value);
        foo.closeIfEqValue(id, 42);
        item = foo.getItem(id);
        assertEq(uint8(item.status), uint8(Foo.Status.CLOSED));

        foo = new Foo();
        id = foo.addItem(value);
        foo.incrementIfEqValue(id, 903);
        item = foo.getItem(id);
        assertEq(item.value, 42);

        foo = new Foo();
        id = foo.addItem(value);
        foo.incrementIfEqValue(id, 42);
        item = foo.getItem(id);
        assertEq(item.value, 43);
    }

    function test_issue_4309() external {
        uint256[] memory arr = new uint256[](1);
        arr[0] = 1;
        uint256 number = 2;
        vm.expectRevert(abi.encodeWithSelector(Foo.Gte1.selector, number, arr[0]));
        foo.checkLt(number, arr);

        number = 1;
        vm.expectRevert(abi.encodeWithSelector(Foo.Gte1.selector, number, arr[0]));
        foo.checkLt(number, arr);

        number = 0;
        bool result = foo.checkLt(number, arr);
        assertTrue(result);
    }

    function test_issue_4314() external {
        uint256[] memory arr = new uint256[](1);
        arr[0] = 1;
        foo.checkEmptyStatements(0, arr);
    }

    function test_single_path_child_branch() external {
        foo.singlePathCoverage(1);
    }

    function test_single_path_parent_branch() external {
        foo.singlePathCoverage(9);
    }

    function test_single_path_branch() external {
        foo.singlePathCoverage(15);
    }
}
    "#,
    );

    // Assert no coverage for single path branch. 2 branches (parent and child) not covered.
    cmd.arg("coverage")
        .args(["--nmt", "test_single_path_child_branch|test_single_path_parent_branch"])
        .assert_success()
        .stdout_eq(str![[r#"
...
╭-------------+----------------+----------------+---------------+---------------╮
| File        | % Lines        | % Statements   | % Branches    | % Funcs       |
+===============================================================================+
| src/Foo.sol | 91.67% (33/36) | 90.00% (27/30) | 80.00% (8/10) | 100.00% (9/9) |
|-------------+----------------+----------------+---------------+---------------|
| Total       | 91.67% (33/36) | 90.00% (27/30) | 80.00% (8/10) | 100.00% (9/9) |
╰-------------+----------------+----------------+---------------+---------------╯

"#]]);

    // Assert no coverage for single path child branch. 1 branch (child) not covered.
    cmd.forge_fuse()
        .arg("coverage")
        .args(["--nmt", "test_single_path_child_branch"])
        .assert_success()
        .stdout_eq(str![[r#"
...
╭-------------+----------------+----------------+---------------+---------------╮
| File        | % Lines        | % Statements   | % Branches    | % Funcs       |
+===============================================================================+
| src/Foo.sol | 97.22% (35/36) | 96.67% (29/30) | 90.00% (9/10) | 100.00% (9/9) |
|-------------+----------------+----------------+---------------+---------------|
| Total       | 97.22% (35/36) | 96.67% (29/30) | 90.00% (9/10) | 100.00% (9/9) |
╰-------------+----------------+----------------+---------------+---------------╯

"#]]);

    // Assert 100% coverage.
    cmd.forge_fuse().arg("coverage").assert_success().stdout_eq(str![[r#"
...
╭-------------+-----------------+-----------------+-----------------+---------------╮
| File        | % Lines         | % Statements    | % Branches      | % Funcs       |
+===================================================================================+
| src/Foo.sol | 100.00% (36/36) | 100.00% (30/30) | 100.00% (10/10) | 100.00% (9/9) |
|-------------+-----------------+-----------------+-----------------+---------------|
| Total       | 100.00% (36/36) | 100.00% (30/30) | 100.00% (10/10) | 100.00% (9/9) |
╰-------------+-----------------+-----------------+-----------------+---------------╯

"#]]);
}

#[forgetest]
fn function_call(prj: _, cmd: _) {
    prj.insert_ds_test();
    prj.add_source(
        "AContract.sol",
        r#"
contract AContract {
    struct Custom {
        bool a;
        uint256 b;
    }

    function coverMe() external returns (bool) {
        // Next lines should not be counted in coverage.
        string("");
        uint256(1);
        address(this);
        bool(false);
        Custom(true, 10);
        // Next lines should be counted in coverage.
        uint256 a = uint256(1);
        Custom memory cust = Custom(false, 100);
        privateWithNoBody();
        privateWithBody();
        publicWithNoBody();
        publicWithBody();
        return true;
    }

    function privateWithNoBody() private {}

    function privateWithBody() private returns (bool) {
        return true;
    }

    function publicWithNoBody() private {}

    function publicWithBody() private returns (bool) {
        return true;
    }
}
    "#,
    );

    prj.add_source(
        "AContractTest.sol",
        r#"
import "./test.sol";
import {AContract} from "./AContract.sol";

contract AContractTest is DSTest {
    AContract a = new AContract();

    function testTypeConversionCoverage() external {
        a.coverMe();
    }
}
    "#,
    );

    // Assert 100% coverage.
    cmd.arg("coverage").assert_success().stdout_eq(str![[r#"
...
╭-------------------+-----------------+---------------+------------+---------------╮
| File              | % Lines         | % Statements  | % Branches | % Funcs       |
+==================================================================================+
| src/AContract.sol | 100.00% (14/14) | 100.00% (9/9) | N/A (0/0)  | 100.00% (5/5) |
|-------------------+-----------------+---------------+------------+---------------|
| Total             | 100.00% (14/14) | 100.00% (9/9) | N/A (0/0)  | 100.00% (5/5) |
╰-------------------+-----------------+---------------+------------+---------------╯

"#]]);
}

#[forgetest]
fn try_catch(prj: _, cmd: _) {
    prj.insert_ds_test();
    prj.add_source(
        "Foo.sol",
        r#"
contract Foo {
    address public owner;

    constructor(address _owner) {
        require(_owner != address(0), "invalid address");
        assert(_owner != 0x0000000000000000000000000000000000000001);
        owner = _owner;
    }

    function myFunc(uint256 x) public pure returns (string memory) {
        require(x != 0, "require failed");
        return "my func was called";
    }
}

contract Bar {
    event Log(string message);
    event LogBytes(bytes data);

    Foo public foo;

    constructor() {
        foo = new Foo(msg.sender);
    }

    function tryCatchExternalCall(uint256 _i) public {
        try foo.myFunc(_i) returns (string memory result) {
            emit Log(result);
        } catch {
            emit Log("external call failed");
        }
    }

    function tryCatchNewContract(address _owner) public {
        try new Foo(_owner) returns (Foo foo_) {
            emit Log("Foo created");
        } catch Error(string memory reason) {
            emit Log(reason);
        } catch (bytes memory reason) {}
    }

    function tryCatchAllNewContract(address _owner) public {
        try new Foo(_owner) returns (Foo foo_) {} catch {}
    }
}
    "#,
    );

    prj.add_source(
        "FooTest.sol",
        r#"
import "./test.sol";
import {Bar, Foo} from "./Foo.sol";

interface Vm {
    function expectRevert() external;
}

contract FooTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function test_happy_foo_coverage() external {
        vm.expectRevert();
        Foo foo = new Foo(address(0));
        vm.expectRevert();
        foo = new Foo(address(1));
        foo = new Foo(address(2));
    }

    function test_happy_path_coverage() external {
        Bar bar = new Bar();
        bar.tryCatchNewContract(0x0000000000000000000000000000000000000002);
        bar.tryCatchAllNewContract(0x0000000000000000000000000000000000000002);
        bar.tryCatchExternalCall(1);
    }

    function test_coverage() external {
        Bar bar = new Bar();
        bar.tryCatchNewContract(0x0000000000000000000000000000000000000000);
        bar.tryCatchNewContract(0x0000000000000000000000000000000000000001);
        bar.tryCatchAllNewContract(0x0000000000000000000000000000000000000001);
        bar.tryCatchExternalCall(0);
    }
}
    "#,
    );

    // Assert coverage not 100% for happy paths only.
    cmd.arg("coverage").args(["--mt", "happy"]).assert_success().stdout_eq(str![[r#"
...
╭-------------+----------------+----------------+--------------+---------------╮
| File        | % Lines        | % Statements   | % Branches   | % Funcs       |
+==============================================================================+
| src/Foo.sol | 77.27% (17/22) | 78.57% (11/14) | 66.67% (6/9) | 100.00% (6/6) |
|-------------+----------------+----------------+--------------+---------------|
| Total       | 77.27% (17/22) | 78.57% (11/14) | 66.67% (6/9) | 100.00% (6/6) |
╰-------------+----------------+----------------+--------------+---------------╯

"#]]);

    // Assert 100% branch coverage (including clauses without body).
    cmd.forge_fuse().arg("coverage").assert_success().stdout_eq(str![[r#"
...
╭-------------+-----------------+-----------------+---------------+---------------╮
| File        | % Lines         | % Statements    | % Branches    | % Funcs       |
+=================================================================================+
| src/Foo.sol | 100.00% (22/22) | 100.00% (14/14) | 100.00% (9/9) | 100.00% (6/6) |
|-------------+-----------------+-----------------+---------------+---------------|
| Total       | 100.00% (22/22) | 100.00% (14/14) | 100.00% (9/9) | 100.00% (6/6) |
╰-------------+-----------------+-----------------+---------------+---------------╯

"#]]);
}

#[forgetest]
fn yul(prj: _, cmd: _) {
    prj.insert_ds_test();
    prj.add_source(
        "Foo.sol",
        r#"
contract Foo {
    uint256[] dynamicArray;

    function readDynamicArrayLength() public view returns (uint256 length) {
        assembly {
            length := sload(dynamicArray.slot)
        }
    }

    function switchAndIfStatements(uint256 n) public pure {
        uint256 y;
        assembly {
            switch n
            case 0 { y := 0 }
            case 1 { y := 1 }
            default { y := n }

            if y { y := 2 }
        }
    }

    function yulForLoop(uint256 n) public {
        uint256 y;
        assembly {
            for { let i := 0 } lt(i, n) { i := add(i, 1) } { y := add(y, 1) }

            let j := 0
            for {} lt(j, n) { j := add(j, 1) } { j := add(j, 2) }
        }
    }

    function hello() public pure returns (bool, uint256, bytes32) {
        bool x;
        uint256 y;
        bytes32 z;

        assembly {
            x := 1
            y := 0xa
            z := "Hello World!"
        }

        return (x, y, z);
    }

    function inlineFunction() public returns (uint256) {
        uint256 result;
        assembly {
            function sum(a, b) -> c {
                c := add(a, b)
            }

            function multiply(a, b) -> c {
                for { let i := 0 } lt(i, b) { i := add(i, 1) } { c := add(c, a) }
            }

            result := sum(2, 3)
            result := multiply(result, 5)
        }
        return result;
    }
}
    "#,
    );

    prj.add_source(
        "FooTest.sol",
        r#"
import "./test.sol";
import {Foo} from "./Foo.sol";

contract FooTest is DSTest {
    function test_foo_coverage() external {
        Foo foo = new Foo();
        foo.switchAndIfStatements(0);
        foo.switchAndIfStatements(1);
        foo.switchAndIfStatements(2);
        foo.yulForLoop(2);
        foo.hello();
        foo.readDynamicArrayLength();
        foo.inlineFunction();
    }
}
    "#,
    );

    cmd.forge_fuse().arg("coverage").assert_success().stdout_eq(str![[r#"
...
╭-------------+-----------------+-----------------+---------------+---------------╮
| File        | % Lines         | % Statements    | % Branches    | % Funcs       |
+=================================================================================+
| src/Foo.sol | 100.00% (30/30) | 100.00% (40/40) | 100.00% (2/2) | 100.00% (7/7) |
|-------------+-----------------+-----------------+---------------+---------------|
| Total       | 100.00% (30/30) | 100.00% (40/40) | 100.00% (2/2) | 100.00% (7/7) |
╰-------------+-----------------+-----------------+---------------+---------------╯

"#]]);
}

#[forgetest]
fn misc(prj: _, cmd: _) {
    prj.insert_ds_test();
    prj.add_source(
        "Foo.sol",
        r#"
struct Custom {
    int256 f1;
}

contract A {
    function f(Custom memory custom) public returns (int256) {
        return custom.f1;
    }
}

contract B {
    uint256 public x;

    constructor(uint256 a) payable {
        x = a;
    }
}

contract C {
    function create() public {
        B b = new B{value: 1}(2);
        b = new B{value: 1}(2);
        b = (new B){value: 1}(2);
    }
}

contract D {
    uint256 index;

    function g() public {
        (uint256 x,, uint256 y) = (7, true, 2);
        (x, y) = (y, x);
        (index,,) = (7, true, 2);
    }
}
    "#,
    );

    prj.add_source(
        "FooTest.sol",
        r#"
import "./test.sol";
import "./Foo.sol";

interface Vm {
    function deal(address account, uint256 newBalance) external;
}

contract FooTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function test_member_access_coverage() external {
        A a = new A();
        Custom memory cust = Custom(1);
        a.f(cust);
    }

    function test_new_expression_coverage() external {
        B b = new B(1);
        b.x();
        C c = new C();
        vm.deal(address(c), 100 ether);
        c.create();
    }

    function test_tuple_coverage() external {
        D d = new D();
        d.g();
    }
}
    "#,
    );

    cmd.forge_fuse().arg("coverage").assert_success().stdout_eq(str![[r#"
...
╭-------------+-----------------+---------------+------------+---------------╮
| File        | % Lines         | % Statements  | % Branches | % Funcs       |
+============================================================================+
| src/Foo.sol | 100.00% (12/12) | 100.00% (9/9) | N/A (0/0)  | 100.00% (4/4) |
|-------------+-----------------+---------------+------------+---------------|
| Total       | 100.00% (12/12) | 100.00% (9/9) | N/A (0/0)  | 100.00% (4/4) |
╰-------------+-----------------+---------------+------------+---------------╯

"#]]);
}

// https://github.com/foundry-rs/foundry/issues/8605
#[forgetest]
fn single_statement(prj: _, cmd: _) {
    prj.insert_ds_test();
    prj.add_source(
        "AContract.sol",
        r#"
contract AContract {
    event IsTrue(bool isTrue);
    event IsFalse(bool isFalse);

    function ifElseStatementIgnored(bool flag) external returns (bool) {
        if (flag) emit IsTrue(true);
        else emit IsFalse(false);

        bool flag2;
        if (flag) flag2 = true;
        else flag2 = false;

        if (flag2) return true;
        else return false;
    }
}
    "#,
    );

    prj.add_source(
        "AContractTest.sol",
        r#"
import "./test.sol";
import {AContract} from "./AContract.sol";

contract AContractTest is DSTest {
    AContract a = new AContract();

    function testTrueCoverage() external {
        a.ifElseStatementIgnored(true);
    }

    function testFalseCoverage() external {
        a.ifElseStatementIgnored(false);
    }
}
    "#,
    );

    // Assert 50% coverage for true branches.
    cmd.arg("coverage").args(["--mt", "testTrueCoverage"]).assert_success().stdout_eq(str![[r#"
...
╭-------------------+--------------+--------------+--------------+---------------╮
| File              | % Lines      | % Statements | % Branches   | % Funcs       |
+================================================================================+
| src/AContract.sol | 62.50% (5/8) | 57.14% (4/7) | 50.00% (3/6) | 100.00% (1/1) |
|-------------------+--------------+--------------+--------------+---------------|
| Total             | 62.50% (5/8) | 57.14% (4/7) | 50.00% (3/6) | 100.00% (1/1) |
╰-------------------+--------------+--------------+--------------+---------------╯

"#]]);

    // Assert 50% coverage for false branches.
    cmd.forge_fuse()
        .arg("coverage")
        .args(["--mt", "testFalseCoverage"])
        .assert_success()
        .stdout_eq(str![[r#"
...
╭-------------------+---------------+--------------+--------------+---------------╮
| File              | % Lines       | % Statements | % Branches   | % Funcs       |
+=================================================================================+
| src/AContract.sol | 100.00% (8/8) | 57.14% (4/7) | 50.00% (3/6) | 100.00% (1/1) |
|-------------------+---------------+--------------+--------------+---------------|
| Total             | 100.00% (8/8) | 57.14% (4/7) | 50.00% (3/6) | 100.00% (1/1) |
╰-------------------+---------------+--------------+--------------+---------------╯

"#]]);

    // Assert 100% coverage (true/false branches properly covered).
    cmd.forge_fuse().arg("coverage").assert_success().stdout_eq(str![[r#"
...
╭-------------------+---------------+---------------+---------------+---------------╮
| File              | % Lines       | % Statements  | % Branches    | % Funcs       |
+===================================================================================+
| src/AContract.sol | 100.00% (8/8) | 100.00% (7/7) | 100.00% (6/6) | 100.00% (1/1) |
|-------------------+---------------+---------------+---------------+---------------|
| Total             | 100.00% (8/8) | 100.00% (7/7) | 100.00% (6/6) | 100.00% (1/1) |
╰-------------------+---------------+---------------+---------------+---------------╯

"#]]);
}

#[forgetest]
fn single_statement_loop(prj: _, cmd: _) {
    prj.insert_ds_test();
    prj.add_source(
        "AContract.sol",
        r#"
contract AContract {
    function ifBreakContinueIgnored(bool flag) external returns (uint256 sum) {
        for (uint256 i = 0; i < 5; i++) {
            if (flag) continue;
            sum += i;
        }

        for (uint256 i = 0; i < 5; i++) {
            if (flag) break;
            sum += i;
        }
    }
}
    "#,
    );

    prj.add_source(
        "AContractTest.sol",
        r#"
import "./test.sol";
import {AContract} from "./AContract.sol";

contract AContractTest is DSTest {
    AContract a = new AContract();

    function testTrueCoverage() external {
        a.ifBreakContinueIgnored(true);
    }

    function testFalseCoverage() external {
        a.ifBreakContinueIgnored(false);
    }
}
    "#,
    );

    // Both arms are taken.
    cmd.arg("coverage").args(["--mt", "testTrueCoverage"]).assert_success().stdout_eq(str![[r#"
...
╭-------------------+--------------+---------------+---------------+---------------╮
| File              | % Lines      | % Statements  | % Branches    | % Funcs       |
+==================================================================================+
| src/AContract.sol | 71.43% (5/7) | 70.00% (7/10) | 100.00% (2/2) | 100.00% (1/1) |
|-------------------+--------------+---------------+---------------+---------------|
| Total             | 71.43% (5/7) | 70.00% (7/10) | 100.00% (2/2) | 100.00% (1/1) |
╰-------------------+--------------+---------------+---------------+---------------╯

"#]]);

    // Neither arm is taken.
    cmd.forge_fuse()
        .arg("coverage")
        .args(["--mt", "testFalseCoverage"])
        .assert_success()
        .stdout_eq(str![[r#"
...
╭-------------------+---------------+-----------------+-------------+---------------╮
| File              | % Lines       | % Statements    | % Branches  | % Funcs       |
+===================================================================================+
| src/AContract.sol | 100.00% (7/7) | 100.00% (10/10) | 0.00% (0/2) | 100.00% (1/1) |
|-------------------+---------------+-----------------+-------------+---------------|
| Total             | 100.00% (7/7) | 100.00% (10/10) | 0.00% (0/2) | 100.00% (1/1) |
╰-------------------+---------------+-----------------+-------------+---------------╯

"#]]);

    // Assert 100% coverage (true/false branches properly covered).
    cmd.forge_fuse().arg("coverage").assert_success().stdout_eq(str![[r#"
...
╭-------------------+---------------+-----------------+---------------+---------------╮
| File              | % Lines       | % Statements    | % Branches    | % Funcs       |
+=====================================================================================+
| src/AContract.sol | 100.00% (7/7) | 100.00% (10/10) | 100.00% (2/2) | 100.00% (1/1) |
|-------------------+---------------+-----------------+---------------+---------------|
| Total             | 100.00% (7/7) | 100.00% (10/10) | 100.00% (2/2) | 100.00% (1/1) |
╰-------------------+---------------+-----------------+---------------+---------------╯

"#]]);
}

// https://github.com/foundry-rs/foundry/issues/18629
#[forgetest]
fn codeless_if_arm_with_checked_condition_lcov(prj: _, cmd: _) {
    prj.add_source(
        "Checks.sol",
        r#"
contract Checks {
    enum Status { NONE, OPEN }

    uint256 public r;

    function round() external view returns (uint256) {
        return r;
    }

    function index(uint256[] memory a) external pure returns (uint256 sum) {
        for (uint256 i = 0; i < a.length; i++) {
            if (a[i] == 0) continue;
            sum += a[i];
        }
    }

    function enumCheck(Status[] memory s) external pure returns (uint256 open) {
        if (s[0] != Status.OPEN) return open;
        open = 1;
    }

    function externalCall() external {
        if (Checks(address(this)).round() == 0) return;
        r = 1;
    }

    function indexThenRevert(uint256[] memory a) external pure {
        if (a[0] == 0) return;
        revert();
    }

    function breakThenRevert(uint256[] memory a, uint256 key) external pure returns (uint256) {
        for (uint256 i; i < a.length; ++i) {
            if (a[i] == 0) break;
            if (a[i] == key) return i;
        }
        revert();
    }

    function breakInIfThenRevert(uint256[] memory a) external {
        if (a.length == 3) {
            for (uint256 i; i < a.length; ++i) {
                if (a[i] == 0) break;
                r += a[i];
            }
            revert();
        }
        r = this.round();
    }
}
"#,
    );
    prj.add_test(
        "Checks.t.sol",
        r#"
import "../src/Checks.sol";
contract ChecksTest {
    function testArms() public {
        Checks checks = new Checks();
        uint256[] memory a = new uint256[](3);
        a[1] = 1;
        checks.index(a);
        Checks.Status[] memory s = new Checks.Status[](1);
        checks.enumCheck(s);
        s[0] = Checks.Status.OPEN;
        checks.enumCheck(s);
        checks.externalCall();
        checks.indexThenRevert(a);
        try checks.breakThenRevert(a, 1) {} catch {}
        try checks.breakInIfThenRevert(a) {} catch {}
    }
}
"#,
    );
    // The bounds, enum, and call checks in each condition jump around their failure with solc
    // and to a shared panic block with solar, so the codeless arms count from the edge of the
    // `if`'s own jump.
    let solar = prj.ensure_foundry_bin("solar");
    let report = str![[r#"
TN:
SF:src/Checks.sol
DA:9,1
FN:9,Checks.round
FNDA:1,Checks.round
DA:10,1
DA:13,1
FN:13,Checks.index
FNDA:1,Checks.index
DA:14,4
DA:15,3
BRDA:15,0,0,2
DA:16,1
DA:20,2
FN:20,Checks.enumCheck
FNDA:2,Checks.enumCheck
DA:21,2
BRDA:21,1,0,1
DA:22,1
DA:25,1
FN:25,Checks.externalCall
FNDA:1,Checks.externalCall
DA:26,1
BRDA:26,2,0,1
DA:27,0
DA:30,1
FN:30,Checks.indexThenRevert
FNDA:1,Checks.indexThenRevert
DA:31,1
BRDA:31,3,0,1
DA:32,0
DA:35,1
FN:35,Checks.breakThenRevert
FNDA:1,Checks.breakThenRevert
DA:36,1
DA:37,1
BRDA:37,4,0,1
DA:38,0
BRDA:38,5,0,-
DA:40,1
DA:43,1
FN:43,Checks.breakInIfThenRevert
FNDA:1,Checks.breakInIfThenRevert
DA:44,1
BRDA:44,6,0,1
DA:45,1
DA:46,1
BRDA:46,7,0,1
DA:47,0
DA:49,1
DA:51,0
FNF:7
FNH:7
LF:27
LH:22
BRF:8
BRH:7
end_of_record

"#]];
    assert_lcov(cmd.arg("coverage"), report);
    assert_lcov(
        cmd.forge_fuse().arg("coverage").arg("--use").arg(solar),
        str![[r#"
TN:
SF:src/Checks.sol
DA:9,1
FN:9,Checks.round
FNDA:1,Checks.round
DA:10,1
DA:13,1
FN:13,Checks.index
FNDA:1,Checks.index
DA:14,4
DA:15,3
BRDA:15,0,0,2
DA:16,1
DA:20,2
FN:20,Checks.enumCheck
FNDA:2,Checks.enumCheck
DA:21,2
BRDA:21,1,0,1
DA:22,0
DA:25,1
FN:25,Checks.externalCall
FNDA:1,Checks.externalCall
DA:26,1
BRDA:26,2,0,1
DA:27,0
DA:30,1
FN:30,Checks.indexThenRevert
FNDA:1,Checks.indexThenRevert
DA:31,1
BRDA:31,3,0,1
DA:32,0
DA:35,1
FN:35,Checks.breakThenRevert
FNDA:1,Checks.breakThenRevert
DA:36,1
DA:37,1
BRDA:37,4,0,1
DA:38,0
BRDA:38,5,0,-
DA:40,1
DA:43,1
FN:43,Checks.breakInIfThenRevert
FNDA:1,Checks.breakInIfThenRevert
DA:44,1
BRDA:44,6,0,1
DA:45,1
DA:46,1
BRDA:46,7,0,1
DA:47,0
DA:49,0
DA:51,0
FNF:7
FNH:7
LF:27
LH:17
BRF:8
BRH:7
end_of_record

"#]],
    );
}

// https://github.com/foundry-rs/foundry/issues/8604
#[forgetest]
fn branch_with_calldata_reads(prj: _, cmd: _) {
    prj.insert_ds_test();
    prj.add_source(
        "AContract.sol",
        r#"
contract AContract {
    event IsTrue(bool isTrue);
    event IsFalse(bool isFalse);

    function execute(bool[] calldata isTrue) external {
        for (uint256 i = 0; i < isTrue.length; i++) {
            if (isTrue[i]) {
                emit IsTrue(isTrue[i]);
            } else {
                emit IsFalse(!isTrue[i]);
            }
        }
    }
}
    "#,
    );

    prj.add_source(
        "AContractTest.sol",
        r#"
import "./test.sol";
import {AContract} from "./AContract.sol";

contract AContractTest is DSTest {
    AContract a = new AContract();

    function testTrueCoverage() external {
        bool[] memory isTrue = new bool[](1);
        isTrue[0] = true;
        a.execute(isTrue);
    }

    function testFalseCoverage() external {
        bool[] memory isFalse = new bool[](1);
        isFalse[0] = false;
        a.execute(isFalse);
    }
}
    "#,
    );

    // Assert 50% coverage for true branches.
    cmd.arg("coverage").args(["--mt", "testTrueCoverage"]).assert_success().stdout_eq(str![[r#"
...
╭-------------------+--------------+--------------+--------------+---------------╮
| File              | % Lines      | % Statements | % Branches   | % Funcs       |
+================================================================================+
| src/AContract.sol | 80.00% (4/5) | 80.00% (4/5) | 50.00% (1/2) | 100.00% (1/1) |
|-------------------+--------------+--------------+--------------+---------------|
| Total             | 80.00% (4/5) | 80.00% (4/5) | 50.00% (1/2) | 100.00% (1/1) |
╰-------------------+--------------+--------------+--------------+---------------╯

"#]]);

    // Assert 50% coverage for false branches.
    cmd.forge_fuse()
        .arg("coverage")
        .args(["--mt", "testFalseCoverage"])
        .assert_success()
        .stdout_eq(str![[r#"
...
╭-------------------+--------------+--------------+--------------+---------------╮
| File              | % Lines      | % Statements | % Branches   | % Funcs       |
+================================================================================+
| src/AContract.sol | 80.00% (4/5) | 80.00% (4/5) | 50.00% (1/2) | 100.00% (1/1) |
|-------------------+--------------+--------------+--------------+---------------|
| Total             | 80.00% (4/5) | 80.00% (4/5) | 50.00% (1/2) | 100.00% (1/1) |
╰-------------------+--------------+--------------+--------------+---------------╯

"#]]);

    // Assert 100% coverage (true/false branches properly covered).
    cmd.forge_fuse().arg("coverage").assert_success().stdout_eq(str![[r#"
...
╭-------------------+---------------+---------------+---------------+---------------╮
| File              | % Lines       | % Statements  | % Branches    | % Funcs       |
+===================================================================================+
| src/AContract.sol | 100.00% (5/5) | 100.00% (5/5) | 100.00% (2/2) | 100.00% (1/1) |
|-------------------+---------------+---------------+---------------+---------------|
| Total             | 100.00% (5/5) | 100.00% (5/5) | 100.00% (2/2) | 100.00% (1/1) |
╰-------------------+---------------+---------------+---------------+---------------╯

"#]]);
}

// https://github.com/foundry-rs/foundry/issues/10792
#[forgetest]
fn branch_with_storage_bytes_reads(prj: _, cmd: _) {
    prj.insert_ds_test();
    prj.add_source(
        "AContract.sol",
        r#"
contract AContract {
    bytes public optA = bytes("optA");
    bytes public optB = bytes("optB");

    function execute(uint16 value) external view {
        bytes memory element;
        if (value == 4) {
            element = optA;
        } else {
            element = optB;
        }
    }
}
    "#,
    );

    prj.add_source(
        "AContractTest.sol",
        r#"
import "./test.sol";
import {AContract} from "./AContract.sol";

contract AContractTest is DSTest {
    AContract a = new AContract();

    function testTrueCoverage() external view {
        a.execute(4);
    }

    function testFalseCoverage() external view {
        a.execute(5);
    }
}
    "#,
    );

    cmd.arg("coverage").args(["--mt", "testTrueCoverage"]).assert_success().stdout_eq(str![[r#"
...
╭-------------------+--------------+--------------+--------------+---------------╮
| File              | % Lines      | % Statements | % Branches   | % Funcs       |
+================================================================================+
| src/AContract.sol | 80.00% (4/5) | 75.00% (3/4) | 50.00% (1/2) | 100.00% (1/1) |
|-------------------+--------------+--------------+--------------+---------------|
| Total             | 80.00% (4/5) | 75.00% (3/4) | 50.00% (1/2) | 100.00% (1/1) |
╰-------------------+--------------+--------------+--------------+---------------╯

"#]]);

    cmd.forge_fuse()
        .arg("coverage")
        .args(["--mt", "testFalseCoverage"])
        .assert_success()
        .stdout_eq(str![[r#"
...
╭-------------------+--------------+--------------+--------------+---------------╮
| File              | % Lines      | % Statements | % Branches   | % Funcs       |
+================================================================================+
| src/AContract.sol | 80.00% (4/5) | 75.00% (3/4) | 50.00% (1/2) | 100.00% (1/1) |
|-------------------+--------------+--------------+--------------+---------------|
| Total             | 80.00% (4/5) | 75.00% (3/4) | 50.00% (1/2) | 100.00% (1/1) |
╰-------------------+--------------+--------------+--------------+---------------╯

"#]]);

    cmd.forge_fuse().arg("coverage").assert_success().stdout_eq(str![[r#"
...
╭-------------------+---------------+---------------+---------------+---------------╮
| File              | % Lines       | % Statements  | % Branches    | % Funcs       |
+===================================================================================+
| src/AContract.sol | 100.00% (5/5) | 100.00% (4/4) | 100.00% (2/2) | 100.00% (1/1) |
|-------------------+---------------+---------------+---------------+---------------|
| Total             | 100.00% (5/5) | 100.00% (4/4) | 100.00% (2/2) | 100.00% (1/1) |
╰-------------------+---------------+---------------+---------------+---------------╯

"#]]);
}

#[forgetest]
fn branch_with_code_free_else(prj: _, cmd: _) {
    prj.insert_ds_test();
    prj.add_source(
        "AContract.sol",
        r#"
contract AContract {
    uint256 public value;

    function execute(bool condition) external {
        if (condition) {
            value = 1;
        } else {
            uint256 unused;
        }
    }
}
    "#,
    );

    prj.add_source(
        "AContractTest.sol",
        r#"
import "./test.sol";
import {AContract} from "./AContract.sol";

contract AContractTest is DSTest {
    AContract a = new AContract();

    function testCoverage() external {
        a.execute(true);
        a.execute(false);
    }
}
    "#,
    );

    cmd.arg("coverage").assert_success().stdout_eq(str![[r#"
...
╭-------------------+--------------+--------------+---------------+---------------╮
| File              | % Lines      | % Statements | % Branches    | % Funcs       |
+=================================================================================+
| src/AContract.sol | 75.00% (3/4) | 50.00% (1/2) | 100.00% (2/2) | 100.00% (1/1) |
|-------------------+--------------+--------------+---------------+---------------|
| Total             | 75.00% (3/4) | 50.00% (1/2) | 100.00% (2/2) | 100.00% (1/1) |
╰-------------------+--------------+--------------+---------------+---------------╯

"#]]);
}

#[forgetest]
fn ternary_return(prj: _, cmd: _) {
    prj.insert_ds_test();
    prj.add_source(
        "AContract.sol",
        r#"
contract AContract {
    function execute(bool condition) external pure returns (uint256) {
        return condition ? 1 : 2;
    }
}
"#,
    );
    prj.add_source(
        "AContractTest.sol",
        r#"
import "./test.sol";
import {AContract} from "./AContract.sol";

contract AContractTest is DSTest {
    AContract a = new AContract();

    function testCoverage() external {
        a.execute(true);
    }
}
"#,
    );
    cmd.arg("coverage").assert_success().stdout_eq(str![[r#"
...
╭-------------------+---------------+---------------+--------------+---------------╮
| File              | % Lines       | % Statements  | % Branches   | % Funcs       |
+==================================================================================+
| src/AContract.sol | 100.00% (2/2) | 100.00% (2/2) | 50.00% (1/2) | 100.00% (1/1) |
|-------------------+---------------+---------------+--------------+---------------|
| Total             | 100.00% (2/2) | 100.00% (2/2) | 50.00% (1/2) | 100.00% (1/1) |
╰-------------------+---------------+---------------+--------------+---------------╯

"#]]);
}

#[forgetest]
fn ternary_array_copy(prj: _, cmd: _) {
    prj.insert_ds_test();
    prj.add_source(
        "AContract.sol",
        r#"
contract AContract {
    uint256[] values;

    constructor() {
        values.push(1);
        values.push(2);
    }

    function execute(bool condition) external view returns (uint256[] memory) {
        return condition ? new uint256[](0) : values;
    }
}
"#,
    );
    prj.add_source(
        "AContractTest.sol",
        r#"
import "./test.sol";
import {AContract} from "./AContract.sol";

contract AContractTest is DSTest {
    AContract a = new AContract();

    function testCoverage() external view {
        a.execute(false);
    }
}
"#,
    );
    cmd.arg("coverage").assert_success().stdout_eq(str![[r#"
...
╭-------------------+---------------+---------------+--------------+---------------╮
| File              | % Lines       | % Statements  | % Branches   | % Funcs       |
+==================================================================================+
| src/AContract.sol | 100.00% (5/5) | 100.00% (4/4) | 50.00% (1/2) | 100.00% (2/2) |
|-------------------+---------------+---------------+--------------+---------------|
| Total             | 100.00% (5/5) | 100.00% (4/4) | 50.00% (1/2) | 100.00% (2/2) |
╰-------------------+---------------+---------------+--------------+---------------╯

"#]]);
}

#[forgetest]
fn ternary_nested_partial(prj: _, cmd: _) {
    prj.insert_ds_test();
    prj.add_source(
        "AContract.sol",
        r#"
contract AContract {
    function execute(bool outer, bool inner) external pure returns (uint256) {
        return outer ? (inner ? 1 : 2) : 3;
    }
}
"#,
    );
    prj.add_source(
        "AContractTest.sol",
        r#"
import "./test.sol";
import {AContract} from "./AContract.sol";

contract AContractTest is DSTest {
    AContract a = new AContract();

    function testCoverage() external {
        a.execute(true, true);
        a.execute(true, false);
    }
}
"#,
    );
    cmd.arg("coverage").assert_success().stdout_eq(str![[r#"
...
╭-------------------+---------------+---------------+--------------+---------------╮
| File              | % Lines       | % Statements  | % Branches   | % Funcs       |
+==================================================================================+
| src/AContract.sol | 100.00% (2/2) | 100.00% (2/2) | 75.00% (3/4) | 100.00% (1/1) |
|-------------------+---------------+---------------+--------------+---------------|
| Total             | 100.00% (2/2) | 100.00% (2/2) | 75.00% (3/4) | 100.00% (1/1) |
╰-------------------+---------------+---------------+--------------+---------------╯

"#]]);
}

#[forgetest]
fn ternary_nested_outer_only(prj: _, cmd: _) {
    prj.insert_ds_test();
    prj.add_source(
        "AContract.sol",
        r#"
contract AContract {
    function execute(bool outer, bool inner) external pure returns (uint256) {
        return outer ? (inner ? 1 : 2) : 3;
    }
}
"#,
    );
    prj.add_source(
        "AContractTest.sol",
        r#"
import "./test.sol";
import {AContract} from "./AContract.sol";

contract AContractTest is DSTest {
    AContract a = new AContract();

    function testCoverage() external {
        a.execute(false, false);
    }
}
"#,
    );
    cmd.arg("coverage").assert_success().stdout_eq(str![[r#"
...
╭-------------------+---------------+---------------+--------------+---------------╮
| File              | % Lines       | % Statements  | % Branches   | % Funcs       |
+==================================================================================+
| src/AContract.sol | 100.00% (2/2) | 100.00% (2/2) | 25.00% (1/4) | 100.00% (1/1) |
|-------------------+---------------+---------------+--------------+---------------|
| Total             | 100.00% (2/2) | 100.00% (2/2) | 25.00% (1/4) | 100.00% (1/1) |
╰-------------------+---------------+---------------+--------------+---------------╯

"#]]);
}

#[forgetest]
fn ternary_nested_false_partial(prj: _, cmd: _) {
    prj.insert_ds_test();
    prj.add_source(
        "AContract.sol",
        r#"
contract AContract {
    function execute(bool outer, bool inner) external pure returns (uint256) {
        return outer ? 1 : (inner ? 2 : 3);
    }
}
"#,
    );
    prj.add_source(
        "AContractTest.sol",
        r#"
import "./test.sol";
import {AContract} from "./AContract.sol";

contract AContractTest is DSTest {
    AContract a = new AContract();

    function testCoverage() external {
        a.execute(false, true);
        a.execute(false, false);
    }
}
"#,
    );
    cmd.arg("coverage").assert_success().stdout_eq(str![[r#"
...
╭-------------------+---------------+---------------+--------------+---------------╮
| File              | % Lines       | % Statements  | % Branches   | % Funcs       |
+==================================================================================+
| src/AContract.sol | 100.00% (2/2) | 100.00% (2/2) | 75.00% (3/4) | 100.00% (1/1) |
|-------------------+---------------+---------------+--------------+---------------|
| Total             | 100.00% (2/2) | 100.00% (2/2) | 75.00% (3/4) | 100.00% (1/1) |
╰-------------------+---------------+---------------+--------------+---------------╯

"#]]);
}

#[forgetest]
fn ternary_expression_contexts(prj: _, cmd: _) {
    prj.insert_ds_test();
    prj.add_source(
        "AContract.sol",
        r#"
contract AContract {
    function bare(bool cond) external pure {
        cond ? uint(1) : uint(2);
    }
    function assignment(bool cond) external pure returns (uint x) {
        x = cond ? 1 : 2;
    }
    function declaration(bool cond) external pure returns (uint) {
        uint x = cond ? 1 : 2;
        return x;
    }
    function tuple(bool cond) external pure returns (uint) {
        (uint x, uint y) = (cond ? 1 : 2, 0);
        return x + y;
    }
    function argument(bool cond) external pure returns (uint) {
        return identity(cond ? 1 : 2);
    }
    function identity(uint x) internal pure returns (uint) { return x; }
}
"#,
    );
    prj.add_source(
        "AContractTest.sol",
        r#"
import "./test.sol";
import {AContract} from "./AContract.sol";

contract AContractTest is DSTest {
    AContract a = new AContract();

    function testCoverage() external {
        a.bare(true);
        a.assignment(true);
        a.declaration(true);
        a.tuple(true);
        a.argument(true);
    }
}
"#,
    );
    cmd.arg("coverage").assert_success().stdout_eq(str![[r#"
...
╭-------------------+-----------------+-----------------+---------------+---------------╮
| File              | % Lines         | % Statements    | % Branches    | % Funcs       |
+=======================================================================================+
| src/AContract.sol | 100.00% (13/13) | 100.00% (11/11) | 50.00% (5/10) | 100.00% (6/6) |
|-------------------+-----------------+-----------------+---------------+---------------|
| Total             | 100.00% (13/13) | 100.00% (11/11) | 50.00% (5/10) | 100.00% (6/6) |
╰-------------------+-----------------+-----------------+---------------+---------------╯

"#]]);
}

#[forgetest]
fn ternary_modifier(prj: _, cmd: _) {
    prj.insert_ds_test();
    prj.add_source(
        "AContract.sol",
        r#"
contract AContract {
    modifier check(bool cond) {
        uint256 x = cond ? 1 : 2;
        _;
    }
    function a(bool cond) external check(cond) returns (uint256) { return 1; }
    function b(bool cond) external check(cond) returns (uint256) { return 2; }
}
"#,
    );
    prj.add_source(
        "AContractTest.sol",
        r#"
import "./test.sol";
import {AContract} from "./AContract.sol";

contract AContractTest is DSTest {
    AContract a = new AContract();

    function testCoverage() external {
        a.a(true);
        a.b(false);
    }
}
"#,
    );
    cmd.arg("coverage").assert_success().stdout_eq(str![[r#"
...
╭-------------------+---------------+---------------+--------------+---------------╮
| File              | % Lines       | % Statements  | % Branches   | % Funcs       |
+==================================================================================+
| src/AContract.sol | 100.00% (4/4) | 100.00% (4/4) | 50.00% (1/2) | 100.00% (3/3) |
|-------------------+---------------+---------------+--------------+---------------|
| Total             | 100.00% (4/4) | 100.00% (4/4) | 50.00% (1/2) | 100.00% (3/3) |
╰-------------------+---------------+---------------+--------------+---------------╯

"#]]);
}

// Path 0 of a ternary is its false arm and path 1 its true arm, whichever side of the jump the
// compiler puts each on. solc's legacy pipeline jumps to the true arm, while `--ir-minimum` and
// Solar jump to the false arm of some ternaries.
#[forgetest]
fn ternary_paths_follow_arms(prj: _, cmd: _) {
    prj.insert_ds_test();
    prj.add_source(
        "AContract.sol",
        r#"
contract AContract {
    function clamp(uint256 x) external pure returns (uint256) {
        return x > 99 ? 5 : 4;
    }

    function sub(uint256 a, uint256 b, uint256 c) external pure returns (uint256) {
        return a + b > c ? a + b - c : 0;
    }
}
"#,
    );
    prj.add_source(
        "AContractTest.sol",
        r#"
import "./test.sol";
import {AContract} from "./AContract.sol";

contract AContractTest is DSTest {
    AContract a = new AContract();

    function testCoverage() external view {
        a.clamp(100);
        a.sub(1, 1, 5);
    }
}
"#,
    );

    // `clamp` takes its true arm only and `sub` its false arm only.
    let expected = str![[r#"
...
BRDA:6,0,0,0
BRDA:6,0,1,1
...
BRDA:10,1,0,1
BRDA:10,1,1,0
...
"#]];
    assert_lcov(cmd.arg("coverage").arg("--no-isolate"), expected.clone());
    assert_lcov(
        cmd.forge_fuse().arg("coverage").args(["--no-isolate", "--ir-minimum"]),
        expected.clone(),
    );
    let solar = prj.ensure_foundry_bin("solar");
    assert_lcov(
        cmd.forge_fuse().arg("coverage").arg("--no-isolate").arg("--use").arg(&solar),
        expected,
    );
}

// A branch path counts the times control took its edge, even when its arm has no code of its own.
// The jump of `if (i == 3) { break; }` is the arm, whichever side of it each compiler puts the loop
// exit on. Its arm runs once in the 6 times the jump runs, so swapped or summed counts differ. The
// same holds without braces and for the `break` of an `else if`.
#[forgetest]
fn if_paths_count_jump_edges(prj: _, cmd: _) {
    prj.insert_ds_test();
    prj.add_source(
        "AContract.sol",
        r#"
contract AContract {
    function stop(uint256 n) external pure returns (uint256 s) {
        for (uint256 i = 0; i < n; i++) {
            if (i == 3) {
                break;
            }
            s += i;
        }
    }

    function unbraced(uint256 n) external pure returns (uint256 s) {
        for (uint256 i = 0; i < n; i++) {
            if (i == 3) break;
            s += i;
        }
    }

    function chain(uint256 n) external pure returns (uint256 s) {
        for (uint256 i = 0; i < n; i++) {
            if (i == 1) {
                s += 1;
            } else if (i == 3) {
                break;
            }
            s += 10;
        }
    }
}
"#,
    );
    prj.add_source(
        "AContractTest.sol",
        r#"
import "./test.sol";
import {AContract} from "./AContract.sol";

contract AContractTest is DSTest {
    AContract a = new AContract();

    function testCoverage() external view {
        a.stop(10);
        a.stop(2);
        a.unbraced(10);
        a.unbraced(2);
        a.chain(10);
        a.chain(2);
    }
}
"#,
    );

    let expected = str![[r#"
...
BRDA:7,0,0,1
...
BRDA:16,1,0,1
...
BRDA:23,2,0,2
BRDA:23,2,1,4
...
BRDA:25,3,0,1
...
"#]];
    assert_lcov(cmd.arg("coverage").arg("--no-isolate"), expected.clone());
    assert_lcov(
        cmd.forge_fuse().arg("coverage").args(["--no-isolate", "--ir-minimum"]),
        expected.clone(),
    );
    let solar = prj.ensure_foundry_bin("solar");
    assert_lcov(
        cmd.forge_fuse().arg("coverage").arg("--no-isolate").arg("--use").arg(&solar),
        expected,
    );
}

// An `else if` arm that only calls an internal function has no code of its own once the optimizer
// inlines the function, whose code the source map attributes to the function. The called function
// still tells the two sides of the jump apart, so each path counts its own edge: the arms run 1, 2,
// 4, 8 and 16 times, so swapped, dropped or summed counts differ.
#[forgetest]
fn if_else_if_paths_follow_called_functions_solc_and_solar_lcov(prj: _, cmd: _) {
    prj.add_source(
        "Chain.sol",
        r#"
contract Chain {
    enum Kind { Payable, NonPayable, Zero, Precompile, Forge }

    uint256 public total;

    function check(Kind kind) external {
        if (kind == Kind.Payable) {
            addOne();
        } else if (kind == Kind.NonPayable) {
            addTwo();
        } else if (kind == Kind.Zero) {
            addThree();
        } else if (kind == Kind.Precompile) {
            addFour();
        } else if (kind == Kind.Forge) {
            addFive();
        }
    }

    function addOne() internal {
        total += 1;
    }

    function addTwo() internal {
        total += 2;
    }

    function addThree() internal {
        total += 3;
    }

    function addFour() internal {
        total += 4;
    }

    function addFive() internal {
        total += 5;
    }
}
"#,
    );
    prj.add_test(
        "Chain.t.sol",
        r#"
import "../src/Chain.sol";

contract ChainTest {
    function testPaths() public {
        Chain chain = new Chain();
        chain.check(Chain.Kind.Payable);
        for (uint256 i = 0; i < 2; i++) chain.check(Chain.Kind.NonPayable);
        for (uint256 i = 0; i < 4; i++) chain.check(Chain.Kind.Zero);
        for (uint256 i = 0; i < 8; i++) chain.check(Chain.Kind.Precompile);
        for (uint256 i = 0; i < 16; i++) chain.check(Chain.Kind.Forge);
    }
}
"#,
    );

    let expected = str![[r#"
...
BRDA:10,0,0,1
BRDA:10,0,1,30
...
BRDA:12,1,0,2
BRDA:12,1,1,28
...
BRDA:14,2,0,4
BRDA:14,2,1,24
...
BRDA:16,3,0,8
BRDA:16,3,1,16
...
BRDA:18,4,0,16
...
"#]];
    let optimized = ["coverage", "--no-isolate", "--keep-optimizer", "--optimize"];
    assert_lcov(cmd.arg("coverage").arg("--no-isolate"), expected.clone());
    assert_lcov(cmd.forge_fuse().args(optimized), expected.clone());
    let solar = prj.ensure_foundry_bin("solar");
    assert_lcov(cmd.forge_fuse().args(optimized).arg("--use").arg(&solar), expected);
}

// With via-IR and the optimizer, solc drops the jump of a condition it proves, so the arm that
// remains runs whenever the `if` does and its path must count the `if`'s entries. The other
// counts of the report are not asserted: via-IR source maps do not anchor them exactly.
#[forgetest]
fn if_arm_with_proved_condition_lcov(prj: _, cmd: _) {
    prj.update_config(|config| config.via_ir = true);
    prj.add_source(
        "Gate.sol",
        r#"
contract Gate {
    bool constant OPEN = true;

    uint256 public total;

    function check(uint256 kind) external {
        if (kind == 0) {
            addOne();
        } else if (OPEN) {
            addTwo();
        }
    }

    function addOne() internal {
        total += 1;
    }

    function addTwo() internal {
        total += 2;
    }
}
"#,
    );
    prj.add_test(
        "Gate.t.sol",
        r#"
import "../src/Gate.sol";

contract GateTest {
    function testGate() public {
        Gate gate = new Gate();
        gate.check(0);
        for (uint256 i = 0; i < 2; i++) gate.check(1);
    }
}
"#,
    );

    assert_lcov(
        cmd.args(["coverage", "--no-isolate", "--keep-optimizer", "--optimize"]),
        str![[r#"
...
BRDA:12,1,0,2
...
LH:5
...
"#]],
    );
}

// An `if` in an internal function that nothing calls is not in the bytecode and has no jump, but
// the function it calls runs from elsewhere. That run is not the arm's, so the arm stays unhit.
#[forgetest]
fn if_arm_in_uncalled_function_does_not_take_another_call_lcov(prj: _, cmd: _) {
    prj.add_source(
        "Unused.sol",
        r#"
contract Unused {
    bool constant OPEN = true;

    uint256 public total;

    function run() external {
        bump();
    }

    function never() internal {
        if (OPEN) {
            bump();
        }
    }

    function bump() internal {
        total += 1;
    }
}
"#,
    );
    prj.add_test(
        "Unused.t.sol",
        r#"
import "../src/Unused.sol";

contract UnusedTest {
    function testRun() public {
        Unused unused = new Unused();
        for (uint256 i = 0; i < 3; i++) unused.run();
    }
}
"#,
    );

    assert_lcov(
        cmd.arg("coverage").arg("--no-isolate"),
        str![[r#"
...
BRDA:14,0,0,-
...
"#]],
    );
}

// A public function also runs from outside, so the only internal call to it does not own its hits.
#[forgetest]
fn if_arm_calling_public_function_does_not_take_external_calls_lcov(prj: _, cmd: _) {
    prj.add_source(
        "Public.sol",
        r#"
contract Public {
    bool constant OPEN = true;

    uint256 public total;

    function never() internal {
        if (OPEN) {
            bump();
        }
    }

    function bump() public {
        total += 1;
    }
}
"#,
    );
    prj.add_test(
        "Public.t.sol",
        r#"
import "../src/Public.sol";

contract PublicTest {
    function testBump() public {
        Public target = new Public();
        for (uint256 i = 0; i < 3; i++) target.bump();
    }
}
"#,
    );

    assert_lcov(
        cmd.arg("coverage").arg("--no-isolate"),
        str![[r#"
...
BRDA:10,0,0,-
...
"#]],
    );
}

// A test that inherits the source and calls the arm's function directly is not in the report, but
// its calls still count: in its bytecode the `if` is not compiled, so the arm has no jump and must
// not take the function's hits.
#[forgetest]
fn if_arm_does_not_take_calls_of_an_inheriting_test_lcov(prj: _, cmd: _) {
    prj.add_source(
        "M.sol",
        r#"
abstract contract M {
    uint256 public total;

    function a(bool b) internal {
        if (b) {
            _b(7);
        }
    }

    function _b(uint256 x) internal {
        total |= x;
    }
}

contract C is M {
    function go(bool b) external {
        a(b);
    }
}
"#,
    );
    prj.add_test(
        "M.t.sol",
        r#"
import "../src/M.sol";

contract MTest is M {
    function testGo() public {
        new C().go(true);
        new C().go(false);
        _b(1);
        _b(2);
    }
}
"#,
    );

    assert_lcov(
        cmd.arg("coverage").arg("--no-isolate"),
        str![[r#"
...
BRDA:8,0,0,1
...
"#]],
    );
}

// A call through a member access, such as `super.bump()`, is resolved statically and still runs the
// function that the arm names, so the arm is not its only call.
#[forgetest]
fn if_arm_does_not_take_super_calls_lcov(prj: _, cmd: _) {
    prj.update_config(|config| config.via_ir = true);
    prj.add_source(
        "Super.sol",
        r#"
contract Base {
    uint256 public total;

    function bump() internal {
        total += 1;
    }
}

contract Derived is Base {
    function never() internal {
        if (true) {
            bump();
        }
    }

    function run() external {
        super.bump();
    }
}
"#,
    );
    prj.add_test(
        "Super.t.sol",
        r#"
import "../src/Super.sol";

contract SuperTest {
    function testRun() public {
        Derived derived = new Derived();
        derived.run();
    }
}
"#,
    );

    assert_lcov(
        cmd.args(["coverage", "--no-isolate", "--keep-optimizer", "--optimize"]),
        str![[r#"
...
BRDA:14,0,0,-
...
"#]],
    );
}

// A function pointer can run the function that an arm names from elsewhere, so a reference to the
// function other than a call also keeps the arm from owning its hits.
#[forgetest]
fn if_arm_does_not_take_function_pointer_calls_lcov(prj: _, cmd: _) {
    prj.update_config(|config| config.via_ir = true);
    prj.add_source(
        "Pointer.sol",
        r#"
contract Base {
    uint256 public total;

    function bump() internal {
        total |= 7;
    }
}

contract Derived is Base {
    function never() internal {
        if (true) {
            bump();
        }
    }

    function run() external {
        function() internal f = bump;
        f();
    }
}
"#,
    );
    prj.add_test(
        "Pointer.t.sol",
        r#"
import "../src/Pointer.sol";

contract PointerTest {
    function testRun() public {
        Derived derived = new Derived();
        derived.run();
    }
}
"#,
    );

    assert_lcov(
        cmd.args(["coverage", "--no-isolate", "--keep-optimizer", "--optimize"]),
        str![[r#"
...
BRDA:14,0,0,-
...
"#]],
    );
}

// An arm of an `if` with a proved condition has no jump, and it takes the hits of the function it
// calls only when nothing else can run that function. Every `never` below is not compiled and its
// function runs another way, so no arm may report hits, while the live control reports its own.
// Solc legacy with the optimizer is the mode where a wrong attribution shows as hits.
#[forgetest]
fn if_arms_do_not_take_other_ways_to_run_a_function_lcov(prj: _, cmd: _) {
    prj.add_source(
        "Matrix.sol",
        r#"
// C0: control, the only call of `T` is the arm of a live function.
contract C0 {
    uint256 public t;
    function T() internal { t += 1; }
    function live() external { if (true) { T(); } }
}

// C1: local function pointer.
contract C1 {
    uint256 public t;
    function T() internal { t += 1; }
    function never() internal { if (true) { T(); } }
    function run() external { function() internal f = T; f(); }
}

// C2: state variable pointer.
contract C2 {
    uint256 public t;
    function() internal p;
    function T() internal { t += 1; }
    function never() internal { if (true) { T(); } }
    function run() external { p = T; p(); }
}

// C3: pointer passed as an argument.
contract C3 {
    uint256 public t;
    function T() internal { t += 1; }
    function never() internal { if (true) { T(); } }
    function runit(function() internal f) internal { f(); }
    function run() external { runit(T); }
}

// C4: pointer returned.
contract C4 {
    uint256 public t;
    function T() internal { t += 1; }
    function never() internal { if (true) { T(); } }
    function get() internal pure returns (function() internal f) { f = T; }
    function run() external { get()(); }
}

// C5: `super` and a virtual override.
contract B5 {
    uint256 public t;
    function T() internal virtual { t += 1; }
    function never() internal { if (true) { T(); } }
}
contract C5 is B5 {
    function T() internal override { t += 2; super.T(); }
    function run() external { T(); }
}

// C6: library `using for`.
library L6 {
    function T(uint256[] storage a) internal { a.push(1); }
    function never(uint256[] storage a) internal { if (true) { T(a); } }
}
contract C6 {
    using L6 for uint256[];
    uint256[] a;
    function run() external { a.T(); }
}

// C7: modifier.
contract C7 {
    uint256 public t;
    modifier m() { T(); _; }
    function T() internal { t += 1; }
    function never() internal { if (true) { T(); } }
    function run() external m {}
}

// C8: constructor of a derived contract.
contract B8 {
    uint256 public t;
    function T() internal { t += 1; }
    function never() internal { if (true) { T(); } }
}
contract C8 is B8 {
    constructor() { T(); }
}

// C9: public function reached externally and internally.
contract C9 {
    uint256 public t;
    function T() public { t += 1; }
    function never() internal { if (true) { T(); } }
}

// C10: try/catch arm.
contract C10 {
    uint256 public t;
    function T() internal { t += 1; }
    function never() internal { if (true) { T(); } }
    function bad() external pure { revert(); }
    function run() external { try this.bad() {} catch { T(); } }
}

// C11: inheriting test.
contract B11 {
    uint256 public t;
    function T() internal { t += 1; }
    function never() internal { if (true) { T(); } }
}

// C12: the same function name in two contracts.
contract A12 {
    uint256 public t;
    function T() internal { t += 1; }
    function never() internal { if (true) { T(); } }
}
contract B12 {
    uint256 public t;
    function T() internal { t += 1; }
    function run() external { T(); }
}

// C14: by-name call from a derived contract.
contract B14 {
    uint256 public t;
    function T() internal { t += 1; }
    function never() internal { if (true) { T(); } }
}
contract C14 is B14 {
    function run() external { T(); }
}

// C15: free function used by two contracts.
function F15(uint256[] storage a) {
    a.push(1);
}
contract A15 {
    uint256[] a;
    function never() internal { if (true) { F15(a); } }
}
contract B15 {
    uint256[] a;
    function run() external { F15(a); }
}

// C16: by-name call from a contract in another file.
contract B16 {
    uint256 public t;
    function T() internal { t += 1; }
    function never() internal { if (true) { T(); } }
}

// C17: overloads.
contract C17 {
    uint256 public t;
    function T() internal { t += 1; }
    function T(uint256 x) internal { t += x; }
    function never() internal { if (true) { T(); } }
    function run() external { T(5); }
}
"#,
    );
    prj.add_source(
        "Other.sol",
        r#"
import "./Matrix.sol";

contract C16 is B16 {
    function run() external { T(); }
}
"#,
    );
    prj.add_test(
        "Matrix.t.sol",
        r#"
import "../src/Matrix.sol";
import "../src/Other.sol";

contract MatrixTest is B11 {
    function testAll() public {
        for (uint256 i = 0; i < 3; i++) {
            new C0().live();
            new C1().run();
            new C2().run();
            new C3().run();
            new C4().run();
            new C5().run();
            new C6().run();
            new C7().run();
            new C8();
            new C9().T();
            new C10().run();
            T();
            new A12();
            new B12().run();
            new C14().run();
            new A15();
            new B15().run();
            new C16().run();
            new C17().run();
        }
    }
}
"#,
    );

    assert_lcov(
        cmd.args(["coverage", "--no-isolate", "--keep-optimizer", "--optimize"]),
        str![[r#"
...
SF:src/Matrix.sol
...
BRDA:8,0,0,3
...
BRDA:15,1,0,-
...
BRDA:24,2,0,-
...
BRDA:32,3,0,-
...
BRDA:41,4,0,-
...
BRDA:50,5,0,-
...
BRDA:60,6,0,-
...
BRDA:73,7,0,-
...
BRDA:81,8,0,-
...
BRDA:91,9,0,-
...
BRDA:98,10,0,-
...
BRDA:107,12,0,-
...
BRDA:114,13,0,-
...
BRDA:126,14,0,-
...
BRDA:138,15,0,-
...
BRDA:149,16,0,-
...
BRDA:157,17,0,-
...
"#]],
    );
}

#[forgetest]
fn identical_bytecodes(prj: _, cmd: _) {
    prj.insert_ds_test();
    prj.add_source(
        "AContract.sol",
        r#"
contract AContract {
    uint256 public number;
    address public immutable usdc1;
    address public immutable usdc2;
    address public immutable usdc3;
    address public immutable usdc4;
    address public immutable usdc5;
    address public immutable usdc6;

    constructor() {
        address a = 0x176211869cA2b568f2A7D4EE941E073a821EE1ff;
        usdc1 = a;
        usdc2 = a;
        usdc3 = a;
        usdc4 = a;
        usdc5 = a;
        usdc6 = a;
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
        "AContractTest.sol",
        r#"
import "./test.sol";
import {AContract} from "./AContract.sol";

contract AContractTest is DSTest {
    AContract public counter;

    function setUp() public {
        counter = new AContract();
        counter.setNumber(0);
    }

    function test_Increment() public {
        counter.increment();
        assertEq(counter.number(), 1);
    }
}
    "#,
    );

    cmd.arg("coverage").assert_success().stdout_eq(str![[r#"
...
╭-------------------+-----------------+---------------+------------+---------------╮
| File              | % Lines         | % Statements  | % Branches | % Funcs       |
+==================================================================================+
| src/AContract.sol | 100.00% (12/12) | 100.00% (9/9) | N/A (0/0)  | 100.00% (3/3) |
|-------------------+-----------------+---------------+------------+---------------|
| Total             | 100.00% (12/12) | 100.00% (9/9) | N/A (0/0)  | 100.00% (3/3) |
╰-------------------+-----------------+---------------+------------+---------------╯

"#]]);
}

#[forgetest]
fn constructors(prj: _, cmd: _) {
    prj.insert_ds_test();
    prj.add_source(
        "AContract.sol",
        r#"
contract AContract {
    bool public active;

    constructor() {
        active = true;
    }
}

contract BContract {
    bool public active;

    constructor() {
        active = true;
    }
}
    "#,
    );

    prj.add_source(
        "AContractTest.sol",
        r#"
import "./test.sol";
import "./AContract.sol";

contract AContractTest is DSTest {
    function test_constructors() public {
        AContract a = new AContract();
        BContract b = new BContract();
    }
}
    "#,
    );

    cmd.arg("coverage").assert_success().stdout_eq(str![[r#"
...
╭-------------------+---------------+---------------+------------+---------------╮
| File              | % Lines       | % Statements  | % Branches | % Funcs       |
+================================================================================+
| src/AContract.sol | 100.00% (4/4) | 100.00% (2/2) | N/A (0/0)  | 100.00% (2/2) |
|-------------------+---------------+---------------+------------+---------------|
| Total             | 100.00% (4/4) | 100.00% (2/2) | N/A (0/0)  | 100.00% (2/2) |
╰-------------------+---------------+---------------+------------+---------------╯

"#]]);
}

// https://github.com/foundry-rs/foundry/issues/9270,
// https://github.com/foundry-rs/foundry/issues/9444,
// https://github.com/foundry-rs/foundry/issues/9458
// Test coverage for functions with no statements.
#[forgetest]
fn empty_functions(prj: _, cmd: _) {
    prj.insert_ds_test();
    prj.add_source(
        "AContract.sol",
        r#"
contract AContract {
    constructor() {}

    receive() external payable {}

    fallback() external {}

    function increment() public {}
}
    "#,
    );

    prj.add_source(
        "AContractTest.sol",
        r#"
import "./test.sol";
import "./AContract.sol";

contract AContractTest is DSTest {
    function test_constructors() public {
        AContract a = new AContract();
        a.increment();
        (bool success,) = address(a).call{value: 1}("");
        require(success);
        (success,) = address(a).call(hex"deadbeef");
        require(success);
    }
}
    "#,
    );

    assert_lcov(
        cmd.arg("coverage"),
        str![[r#"
TN:
SF:src/AContract.sol
DA:5,1
FN:5,AContract.constructor
FNDA:1,AContract.constructor
DA:7,1
FN:7,AContract.receive
FNDA:1,AContract.receive
DA:9,1
FN:9,AContract.fallback
FNDA:1,AContract.fallback
DA:11,1
FN:11,AContract.increment
FNDA:1,AContract.increment
FNF:4
FNH:4
LF:4
LH:4
BRF:0
BRH:0
end_of_record

"#]],
    );

    cmd.forge_fuse().arg("coverage").assert_success().stdout_eq(str![[r#"
...
╭-------------------+---------------+--------------+------------+---------------╮
| File              | % Lines       | % Statements | % Branches | % Funcs       |
+===============================================================================+
| src/AContract.sol | 100.00% (4/4) | N/A (0/0)    | N/A (0/0)  | 100.00% (4/4) |
|-------------------+---------------+--------------+------------+---------------|
| Total             | 100.00% (4/4) | N/A (0/0)    | N/A (0/0)  | 100.00% (4/4) |
╰-------------------+---------------+--------------+------------+---------------╯

"#]]);
}

// Test empty shared-memory calldata used by nested calls with isolation disabled.
#[forgetest]
fn empty_shared_calldata(prj: _, cmd: _) {
    prj.insert_ds_test();
    prj.add_source(
        "AContract.sol",
        r#"
contract AContract {
    receive() external payable {}
}
    "#,
    );

    prj.add_source(
        "AContractTest.sol",
        r#"
import "./test.sol";
import "./AContract.sol";

contract AContractTest is DSTest {
    address internal target;

    function setUp() public {
        target = address(new AContract());
    }

    function test_receive() public {
        (bool success,) = target.call{gas: 100_000}("");
        require(success);
    }
}
    "#,
    );

    let expected = str![[r#"
TN:
SF:src/AContract.sol
DA:5,1
FN:5,AContract.receive
FNDA:1,AContract.receive
FNF:1
FNH:1
LF:1
LH:1
BRF:0
BRH:0
end_of_record

"#]];
    assert_lcov(cmd.arg("coverage").arg("--no-isolate"), expected.clone());
    assert_lcov(cmd.forge_fuse().arg("coverage").args(["--no-isolate", "--ir-minimum"]), expected);
}

// Test that inherited empty constructors, receive functions, and fallbacks have distinct anchors.
#[forgetest]
fn empty_special_functions_are_distinct(prj: _, cmd: _) {
    prj.insert_ds_test();
    prj.add_source(
        "AContract.sol",
        r#"
contract Base {
    constructor() {}

    receive() external payable {}

    fallback() external {}

    function increment() public {}
}

contract AContract is Base {}
    "#,
    );

    prj.add_source(
        "AContractTest.sol",
        r#"
import "./test.sol";
import "./AContract.sol";

contract AContractTest is DSTest {
    function test_receive() public {
        AContract a = new AContract();
        (bool success,) = address(a).call{value: 1}("");
        require(success);
    }
}
    "#,
    );

    assert_lcov(
        cmd.arg("coverage"),
        str![[r#"
TN:
SF:src/AContract.sol
DA:5,1
FN:5,Base.constructor
FNDA:1,Base.constructor
DA:7,1
FN:7,Base.receive
FNDA:1,Base.receive
DA:9,0
FN:9,Base.fallback
FNDA:0,Base.fallback
DA:11,0
FN:11,Base.increment
FNDA:0,Base.increment
FNF:4
FNH:2
LF:4
LH:2
BRF:0
BRH:0
end_of_record

"#]],
    );
}

// Test that a fallback without a receive uses the same anchor for empty and non-empty calldata.
#[forgetest]
fn empty_fallback_without_receive(prj: _, cmd: _) {
    prj.insert_ds_test();
    prj.add_source(
        "AContract.sol",
        r#"
contract AContract {
    fallback() external payable {}
}
    "#,
    );

    prj.add_source(
        "AContractTest.sol",
        r#"
import "./test.sol";
import "./AContract.sol";

contract AContractTest is DSTest {
    function test_fallback() public {
        AContract a = new AContract();
        (bool success,) = address(a).call{value: 1}("");
        require(success);
        (success,) = address(a).call(hex"01");
        require(success);
        (success,) = address(a).call(hex"deadbeef");
        require(success);
    }
}
    "#,
    );

    assert_lcov(
        cmd.arg("coverage"),
        str![[r#"
TN:
SF:src/AContract.sol
DA:5,3
FN:5,AContract.fallback
FNDA:3,AContract.fallback
FNF:1
FNH:1
LF:1
LH:1
BRF:0
BRH:0
end_of_record

"#]],
    );
}

// Test that a nonpayable fallback is not covered by a rejected value-bearing call.
#[forgetest]
fn empty_nonpayable_fallback_rejects_value(prj: _, cmd: _) {
    prj.insert_ds_test();
    prj.add_source(
        "AContract.sol",
        r#"
contract AContract {
    fallback() external {}
}
    "#,
    );

    prj.add_source(
        "AContractTest.sol",
        r#"
import "./test.sol";
import "./AContract.sol";

contract AContractTest is DSTest {
    function test_rejected_fallback() public {
        AContract a = new AContract();
        (bool success,) = address(a).call{value: 1}(hex"deadbeef");
        require(!success);
    }
}
    "#,
    );

    assert_lcov(
        cmd.arg("coverage"),
        str![[r#"
TN:
SF:src/AContract.sol
DA:5,0
FN:5,AContract.fallback
FNDA:0,AContract.fallback
FNF:1
FNH:0
LF:1
LH:0
BRF:0
BRH:0
end_of_record

"#]],
    );
}

// Test coverage for `receive` functions.
#[forgetest]
fn receive(prj: _, cmd: _) {
    prj.insert_ds_test();
    prj.add_source(
        "AContract.sol",
        r#"
contract AContract {
    uint256 public counter = 0;

    constructor() {
        counter = 1;
    }

    receive() external payable {
        counter = msg.value;
    }

    fallback() external {}
}
    "#,
    );

    prj.add_source(
        "AContractTest.sol",
        r#"
import "./test.sol";
import "./AContract.sol";

contract AContractTest is DSTest {
    AContract a = new AContract();

    function test_constructors() public {
        address(a).call{value: 5}("");
        require(a.counter() == 5);
    }
}
    "#,
    );

    // Assert the constructor and receive are covered while the empty fallback is not.
    assert_lcov(
        cmd.arg("coverage"),
        str![[r#"
TN:
SF:src/AContract.sol
DA:7,1
FN:7,AContract.constructor
FNDA:1,AContract.constructor
DA:8,1
DA:11,1
FN:11,AContract.receive
FNDA:1,AContract.receive
DA:12,1
DA:15,0
FN:15,AContract.fallback
FNDA:0,AContract.fallback
FNF:3
FNH:2
LF:5
LH:4
BRF:0
BRH:0
end_of_record

"#]],
    );

    cmd.forge_fuse().arg("coverage").assert_success().stdout_eq(str![[r#"
...
╭-------------------+--------------+---------------+------------+--------------╮
| File              | % Lines      | % Statements  | % Branches | % Funcs      |
+==============================================================================+
| src/AContract.sol | 80.00% (4/5) | 100.00% (2/2) | N/A (0/0)  | 66.67% (2/3) |
|-------------------+--------------+---------------+------------+--------------|
| Total             | 80.00% (4/5) | 100.00% (2/2) | N/A (0/0)  | 66.67% (2/3) |
╰-------------------+--------------+---------------+------------+--------------╯

"#]]);
}

// Test empty constructor coverage when via-IR source maps contain no matching span.
#[forgetest]
fn empty_constructor_ir_minimum(prj: _, cmd: _) {
    prj.insert_ds_test();
    prj.add_source(
        "AContract.sol",
        r#"
contract AContract {
    constructor() {}
}
    "#,
    );

    prj.add_source(
        "AContractTest.sol",
        r#"
import "./test.sol";
import "./AContract.sol";

contract AContractTest is DSTest {
    function test_constructor() public {
        new AContract();
    }
}
    "#,
    );

    assert_lcov(
        cmd.arg("coverage").arg("--ir-minimum"),
        str![[r#"
TN:
SF:src/AContract.sol
DA:5,1
FN:5,AContract.constructor
FNDA:1,AContract.constructor
FNF:1
FNH:1
LF:1
LH:1
BRF:0
BRH:0
end_of_record

"#]],
    );
}

// https://github.com/foundry-rs/foundry/issues/9322
// Test coverage with `--ir-minimum` for solidity < 0.8.5.
#[forgetest]
fn ir_minimum_early(prj: _, cmd: _) {
    prj.insert_ds_test();
    prj.add_source(
        "AContract.sol",
        r#"
pragma solidity 0.8.4;

contract AContract {
    function isContract(address account) internal view returns (bool) {
        bytes32 codehash;
        bytes32 accountHash = 0xc5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470;
        assembly {
            codehash := extcodehash(account)
        }
        return (codehash != accountHash && codehash != 0x0);
    }
}
    "#,
    );

    // Assert coverage doesn't fail with `Error: Unknown key "inliner"`.
    cmd.arg("coverage").arg("--ir-minimum").assert_success().stdout_eq(str![[r#"
...
╭-------------------+-------------+--------------+------------+-------------╮
| File              | % Lines     | % Statements | % Branches | % Funcs     |
+===========================================================================+
| src/AContract.sol | 0.00% (0/5) | 0.00% (0/4)  | N/A (0/0)  | 0.00% (0/1) |
|-------------------+-------------+--------------+------------+-------------|
| Total             | 0.00% (0/5) | 0.00% (0/4)  | N/A (0/0)  | 0.00% (0/1) |
╰-------------------+-------------+--------------+------------+-------------╯

"#]]);
}

// Solar compiles any enabled optimizer with `-Ogas` or `-Osize`, which folds away the always-true
// `x == x` condition. `--ir-minimum` has no effect with Solar, so coverage stays unoptimized.
#[forgetest]
fn ir_minimum_ignored_with_solar(prj: _, cmd: _) {
    prj.insert_ds_test();
    prj.add_source(
        "AContract.sol",
        r#"
contract AContract {
    uint256 public y;

    function f(uint256 x) public {
        if (x == x) {
            y = 2;
        }
    }
}
    "#,
    );
    prj.add_source(
        "AContractTest.sol",
        r#"
import "./test.sol";
import "./AContract.sol";

contract AContractTest is DSTest {
    function test_f() public {
        new AContract().f(1);
    }
}
    "#,
    );

    let solar = prj.ensure_foundry_bin("solar");
    cmd.args(["coverage", "--ir-minimum", "--optimize", "--use"])
        .arg(&solar)
        .assert_success()
        .stdout_eq(str![[r#"
...
╭-------------------+---------------+---------------+---------------+---------------╮
| File              | % Lines       | % Statements  | % Branches    | % Funcs       |
+===================================================================================+
| src/AContract.sol | 100.00% (3/3) | 100.00% (2/2) | 100.00% (1/1) | 100.00% (1/1) |
|-------------------+---------------+---------------+---------------+---------------|
| Total             | 100.00% (3/3) | 100.00% (2/2) | 100.00% (1/1) | 100.00% (1/1) |
╰-------------------+---------------+---------------+---------------+---------------╯

"#]])
        .stderr_eq(str![[r#"
...
Warning: `--ir-minimum` has no effect with Solar, so coverage runs unoptimized.
...
"#]]);
}

// `--keep-optimizer` keeps the optimizer, which folds away the always-true `x == x` condition.
#[forgetest]
fn keep_optimizer(prj: _, cmd: _) {
    prj.insert_ds_test();
    prj.add_source(
        "AContract.sol",
        r#"
contract AContract {
    uint256 public y;

    function f(uint256 x) public {
        if (x == x) {
            y = 2;
        }
    }
}
    "#,
    );

    prj.add_source(
        "AContractTest.sol",
        r#"
import "./test.sol";
import "./AContract.sol";

contract AContractTest is DSTest {
    function test_f() public {
        new AContract().f(1);
    }
}
    "#,
    );

    cmd.arg("coverage").assert_success().stdout_eq(str![[r#"
...
╭-------------------+---------------+---------------+---------------+---------------╮
| File              | % Lines       | % Statements  | % Branches    | % Funcs       |
+===================================================================================+
| src/AContract.sol | 100.00% (3/3) | 100.00% (2/2) | 100.00% (1/1) | 100.00% (1/1) |
|-------------------+---------------+---------------+---------------+---------------|
| Total             | 100.00% (3/3) | 100.00% (2/2) | 100.00% (1/1) | 100.00% (1/1) |
╰-------------------+---------------+---------------+---------------+---------------╯

"#]]);

    cmd.forge_fuse()
        .args(["coverage", "--keep-optimizer", "--optimize"])
        .assert_success()
        .stdout_eq(str![[r#"
...
╭-------------------+---------------+--------------+---------------+---------------╮
| File              | % Lines       | % Statements | % Branches    | % Funcs       |
+==================================================================================+
| src/AContract.sol | 100.00% (3/3) | 50.00% (1/2) | 100.00% (1/1) | 100.00% (1/1) |
|-------------------+---------------+--------------+---------------+---------------|
| Total             | 100.00% (3/3) | 50.00% (1/2) | 100.00% (1/1) | 100.00% (1/1) |
╰-------------------+---------------+--------------+---------------+---------------╯

"#]]);

    // The Solar compiler folds the condition away as well.
    let solar = prj.ensure_foundry_bin("solar");
    cmd.forge_fuse().arg("coverage").arg("--use").arg(&solar).assert_success().stdout_eq(str![[
        r#"
...
╭-------------------+---------------+---------------+---------------+---------------╮
| File              | % Lines       | % Statements  | % Branches    | % Funcs       |
+===================================================================================+
| src/AContract.sol | 100.00% (3/3) | 100.00% (2/2) | 100.00% (1/1) | 100.00% (1/1) |
|-------------------+---------------+---------------+---------------+---------------|
| Total             | 100.00% (3/3) | 100.00% (2/2) | 100.00% (1/1) | 100.00% (1/1) |
╰-------------------+---------------+---------------+---------------+---------------╯

"#
    ]]);

    cmd.forge_fuse()
        .args(["coverage", "--keep-optimizer", "--optimize", "--use"])
        .arg(&solar)
        .assert_success()
        .stdout_eq(str![[r#"
...
╭-------------------+---------------+--------------+---------------+---------------╮
| File              | % Lines       | % Statements | % Branches    | % Funcs       |
+==================================================================================+
| src/AContract.sol | 100.00% (3/3) | 50.00% (1/2) | 100.00% (1/1) | 100.00% (1/1) |
|-------------------+---------------+--------------+---------------+---------------|
| Total             | 100.00% (3/3) | 50.00% (1/2) | 100.00% (1/1) | 100.00% (1/1) |
╰-------------------+---------------+--------------+---------------+---------------╯

"#]]);

    cmd.forge_fuse()
        .args(["coverage", "--keep-optimizer", "--ir-minimum"])
        .assert_failure()
        .stderr_eq(str![[r#"
Error: `ir_minimum` (`--ir-minimum`) and `keep_optimizer` (`--keep-optimizer`) cannot both be enabled

"#]]);

    prj.update_config(|config| {
        config.coverage.keep_optimizer = true;
        config.optimizer = Some(true);
    });
    cmd.forge_fuse().arg("coverage").assert_success().stdout_eq(str![[r#"
...
╭-------------------+---------------+--------------+---------------+---------------╮
| File              | % Lines       | % Statements | % Branches    | % Funcs       |
+==================================================================================+
| src/AContract.sol | 100.00% (3/3) | 50.00% (1/2) | 100.00% (1/1) | 100.00% (1/1) |
|-------------------+---------------+--------------+---------------+---------------|
| Total             | 100.00% (3/3) | 50.00% (1/2) | 100.00% (1/1) | 100.00% (1/1) |
╰-------------------+---------------+--------------+---------------+---------------╯

"#]]);

    prj.update_config(|config| {
        config.coverage.keep_optimizer = false;
        config.coverage.ir_minimum = true;
    });
    cmd.forge_fuse().args(["coverage", "--keep-optimizer"]).assert_failure().stderr_eq(str![[r#"
Error: `ir_minimum` (`--ir-minimum`) and `keep_optimizer` (`--keep-optimizer`) cannot both be enabled

"#]]);
}

#[forgetest]
fn no_artifacts_written(prj: _, cmd: _) {
    prj.insert_ds_test();
    prj.add_source(
        "AContract.sol",
        r#"
contract AContract {
    int public i;

    function init() public {
        i = 0;
    }

    function foo() public {
        i = 1;
    }
}
    "#,
    );

    prj.add_source(
        "AContractTest.sol",
        r#"
import "./test.sol";
import {AContract} from "./AContract.sol";

contract AContractTest is DSTest {
    AContract a;

    function setUp() public {
        a = new AContract();
        a.init();
    }

    function testFoo() public {
        a.foo();
    }
}
    "#,
    );

    cmd.forge_fuse().arg("coverage").assert_success().stdout_eq(str![[r#"
...
╭-------------------+---------------+---------------+------------+---------------╮
| File              | % Lines       | % Statements  | % Branches | % Funcs       |
+================================================================================+
| src/AContract.sol | 100.00% (4/4) | 100.00% (2/2) | N/A (0/0)  | 100.00% (2/2) |
|-------------------+---------------+---------------+------------+---------------|
| Total             | 100.00% (4/4) | 100.00% (2/2) | N/A (0/0)  | 100.00% (2/2) |
╰-------------------+---------------+---------------+------------+---------------╯
...
"#]]);

    // no artifacts are to be written
    let files = files_with_ext(prj.artifacts(), "json").collect::<Vec<_>>();

    assert!(files.is_empty());
}

#[forgetest]
fn attribution_report(prj: _, cmd: _) {
    prj.insert_ds_test();
    prj.add_source(
        "AContract.sol",
        r#"
contract AContract {
    int public i;

    function init() public {
        i = 0;
    }

    function foo() public {
        i = 1;
    }

    function bar() public {
        i = 2;
    }
}
    "#,
    );

    prj.add_source(
        "AContractTest.sol",
        r#"
import "./test.sol";
import {AContract} from "./AContract.sol";

contract AContractTest is DSTest {
    AContract a;

    function setUp() public {
        a = new AContract();
        a.init();
    }

    function testFoo() public {
        a.foo();
    }

    function testBar() public {
        a.bar();
    }
}
    "#,
    );

    cmd.arg("coverage").args(["--report=attribution"]).assert_success();

    let attribution = prj.root().join("coverage-attribution.json");
    assert!(attribution.exists(), "coverage attribution report was not created");

    let custom_attribution = prj.root().join("custom-coverage-attribution.json");
    cmd.forge_fuse()
        .arg("coverage")
        .args(["--report=attribution", "--report-file", "custom-coverage-attribution.json"])
        .assert_success();
    assert!(custom_attribution.exists(), "custom coverage attribution report was not created");

    let json: Value = serde_json::from_str(&std::fs::read_to_string(attribution).unwrap()).unwrap();
    let tests = json["tests"].as_array().unwrap();
    assert_eq!(tests.len(), 2);

    for test in tests {
        assert_eq!(test["status"], "success");
        assert_eq!(test["kind"], "unit");

        let covered = test["covered"].as_array().unwrap();
        assert!(
            covered.iter().any(|item| item["source"] == "src/AContract.sol"
                && item["kind"] == "function"
                && item["function"] == "init"),
            "setUp coverage should be attributed to each selected test"
        );
    }

    let test_foo = tests.iter().find(|test| test["test"] == "testFoo()").unwrap();
    assert!(test_foo["covered"].as_array().unwrap().iter().any(|item| {
        item["source"] == "src/AContract.sol"
            && item["kind"] == "function"
            && item["function"] == "foo"
    }));

    let test_bar = tests.iter().find(|test| test["test"] == "testBar()").unwrap();
    assert!(test_bar["covered"].as_array().unwrap().iter().any(|item| {
        item["source"] == "src/AContract.sol"
            && item["kind"] == "function"
            && item["function"] == "bar"
    }));
}

#[forgetest]
fn attribution_report_keeps_items_aligned_after_filtering_sources(prj: _, cmd: _) {
    prj.insert_ds_test();
    prj.add_source(
        "sub/WContract.sol",
        r#"
contract WContract {
    function hit() public pure returns (uint256) {
        return 2;
    }
}
    "#,
    );
    prj.add_source(
        "XContract.sol",
        r#"
contract XContract {
    function hit() public pure returns (uint256) {
        return 4;
    }
}
    "#,
    );
    prj.add_source(
        "YContract.sol",
        r#"
contract YContract {
    function hit() public pure returns (uint256) {
        return 6;
    }
}
    "#,
    );
    prj.add_source(
        "ZContract.sol",
        r#"
contract ZContract {
    function hit() public pure returns (uint256) {
        return 8;
    }
}
    "#,
    );

    prj.add_source(
        "AttributionFilterTest.sol",
        r#"
import "./test.sol";
import {WContract} from "./sub/WContract.sol";
import {XContract} from "./XContract.sol";
import {YContract} from "./YContract.sol";
import {ZContract} from "./ZContract.sol";

contract AttributionFilterTest is DSTest {
    function testW() public {
        assertEq(new WContract().hit(), 2);
    }

    function testX() public {
        assertEq(new XContract().hit(), 4);
    }

    function testY() public {
        assertEq(new YContract().hit(), 6);
    }

    function testZ() public {
        assertEq(new ZContract().hit(), 8);
    }
}
    "#,
    );

    cmd.arg("coverage")
        .args(["--report=attribution", "--no-match-coverage=YContract"])
        .assert_success();

    let attribution = prj.root().join("coverage-attribution.json");
    let json: Value = serde_json::from_str(&std::fs::read_to_string(attribution).unwrap()).unwrap();
    let tests = json["tests"].as_array().unwrap();

    assert!(
        tests.iter().flat_map(|test| test["covered"].as_array().unwrap()).all(|item| {
            item["source"].as_str().is_some_and(|source| !source.contains("YContract.sol"))
        }),
        "filtered source should not be present in attribution output"
    );

    for (test_name, contract_name) in
        [("testW()", "WContract"), ("testX()", "XContract"), ("testZ()", "ZContract")]
    {
        let test = tests.iter().find(|test| test["test"] == test_name).unwrap();
        let covered = test["covered"].as_array().unwrap();
        assert!(
            covered.iter().any(|item| {
                item["source"]
                    .as_str()
                    .is_some_and(|source| source.ends_with(&format!("{contract_name}.sol")))
                    && item["contract"] == contract_name
                    && item["kind"] == "function"
                    && item["function"] == "hit"
                    && item["hits"].as_u64().is_some_and(|hits| hits > 0)
            }),
            "{test_name} should retain coverage for {contract_name}.hit"
        );
    }
}

#[forgetest]
fn attribution_report_with_lcov_uses_default_paths(prj: _, cmd: _) {
    prj.insert_ds_test();
    prj.add_source(
        "AContract.sol",
        r#"
contract AContract {
    int public i;

    function foo() public {
        i = 1;
    }
}
    "#,
    );

    prj.add_source(
        "AContractTest.sol",
        r#"
import "./test.sol";
import {AContract} from "./AContract.sol";

contract AContractTest is DSTest {
    AContract a;

    function setUp() public {
        a = new AContract();
    }

    function testFoo() public {
        a.foo();
    }
}
    "#,
    );

    let stderr = cmd
        .arg("coverage")
        .args(["--report=attribution", "--report=lcov", "--report-file", "combined.out"])
        .assert_success()
        .get_output()
        .stderr_lossy();

    assert!(
        stderr.contains("`--report-file` is ignored when multiple file reports are requested"),
        "expected warning about ignored --report-file, got:\n{stderr}"
    );

    assert!(prj.root().join("coverage-attribution.json").exists());
    assert!(prj.root().join("lcov.info").exists());
    assert!(!prj.root().join("combined.out").exists());
}

// <https://github.com/foundry-rs/foundry/issues/10172>
#[forgetest]
fn constructor_with_args(prj: _, cmd: _) {
    prj.insert_ds_test();
    prj.add_source(
        "ArrayCondition.sol",
        r#"
contract ArrayCondition {
    uint8 public constant MAX_SIZE = 32;
    error TooLarge();
    error EmptyArray();
    // Storage variable to ensure the constructor does something
    uint256 private _arrayLength;

    constructor(uint256[] memory values) {
        // Check for empty array
        if (values.length == 0) {
            revert EmptyArray();
        }

        if (values.length > MAX_SIZE) {
            revert TooLarge();
        }

        // Store the array length
        _arrayLength = values.length;
    }

    function getArrayLength() external view returns (uint256) {
        return _arrayLength;
    }
}
    "#,
    );

    prj.add_source(
        "ArrayConditionTest.sol",
        r#"
import "./test.sol";
import {ArrayCondition} from "./ArrayCondition.sol";

interface Vm {
    function expectRevert(bytes4 revertData) external;
}

contract ArrayConditionTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testValidSize() public {
        uint256[] memory values = new uint256[](10);
        ArrayCondition condition = new ArrayCondition(values);
        assertEq(condition.getArrayLength(), 10);
    }

    // Test with maximum array size (should NOT revert)
    function testMaxSize() public {
        uint256[] memory values = new uint256[](32);
        ArrayCondition condition = new ArrayCondition(values);
        assertEq(condition.getArrayLength(), 32);
    }

    // Test with too large array size (should revert)
    function testTooLarge() public {
        uint256[] memory values = new uint256[](33);
        vm.expectRevert(ArrayCondition.TooLarge.selector);
        new ArrayCondition(values);
    }

    // Test with empty array (should revert)
    function testEmptyArray() public {
        uint256[] memory values = new uint256[](0);
        vm.expectRevert(ArrayCondition.EmptyArray.selector);
        new ArrayCondition(values);
    }
}
    "#,
    );

    cmd.arg("coverage").assert_success().stdout_eq(str![[r#"
...
╭------------------------+---------------+---------------+---------------+---------------╮
| File                   | % Lines       | % Statements  | % Branches    | % Funcs       |
+========================================================================================+
| src/ArrayCondition.sol | 100.00% (8/8) | 100.00% (6/6) | 100.00% (2/2) | 100.00% (2/2) |
|------------------------+---------------+---------------+---------------+---------------|
| Total                  | 100.00% (8/8) | 100.00% (6/6) | 100.00% (2/2) | 100.00% (2/2) |
╰------------------------+---------------+---------------+---------------+---------------╯
...
"#]]);
}

// https://github.com/foundry-rs/foundry/issues/11432
// Test coverage for linked libraries.
#[forgetest]
fn linked_library(prj: _, cmd: _) {
    prj.insert_ds_test();
    prj.add_source(
        "Counter.sol",
        r#"
library LibCounter {
    function increment(uint256 number) external returns (uint256) {
        return number + 1;
    }
}

contract Counter {
    uint256 public number;

    function increment() public {
        number = LibCounter.increment(number);
    }
}
    "#,
    );

    prj.add_source(
        "CounterTest.sol",
        r#"
import "./test.sol";
import {Counter, LibCounter} from "./Counter.sol";

interface Vm {
    function etch(address target, bytes calldata code) external;
}

contract CounterTest is DSTest {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));

    function testIncrement() public {
        Counter counter = new Counter();
        counter.increment();
    }

    function testRelinked() public {
        Counter counter = new Counter();
        address original = address(LibCounter);
        address replacement = address(0x1234567890123456789012345678901234567890);
        vm.etch(replacement, original.code);
        bytes memory code = address(counter).code;
        bytes20 oldAddress = bytes20(original);
        bytes20 newAddress = bytes20(replacement);
        uint256 patches;
        for (uint256 i; i + 20 <= code.length; ++i) {
            bool matchAddress = true;
            for (uint256 j; j < 20; ++j) {
                if (code[i + j] != oldAddress[j]) matchAddress = false;
            }
            if (matchAddress) {
                for (uint256 j; j < 20; ++j) code[i + j] = newAddress[j];
                ++patches;
                i += 19;
            }
        }
        require(patches == 1, "expected one library link");
        vm.etch(address(counter), code);
        counter.increment();
        require(counter.number() == 1);
    }
}
    "#,
    );

    // Assert 100% coverage for both original and relocated library links.
    for test in ["testIncrement", "testRelinked"] {
        cmd.forge_fuse().args(["coverage", "--mt", test]).assert_success().stdout_eq(str![[r#"
...
╭-----------------+---------------+---------------+------------+---------------╮
| File            | % Lines       | % Statements  | % Branches | % Funcs       |
+==============================================================================+
| src/Counter.sol | 100.00% (4/4) | 100.00% (3/3) | N/A (0/0)  | 100.00% (2/2) |
|-----------------+---------------+---------------+------------+---------------|
| Total           | 100.00% (4/4) | 100.00% (3/3) | N/A (0/0)  | 100.00% (2/2) |
╰-----------------+---------------+---------------+------------+---------------╯
...
"#]]);
    }
}

// <https://github.com/foundry-rs/foundry/issues/10422>
// Test that line hits are properly recorded in lcov report.
#[forgetest]
fn do_while_lcov(prj: _, cmd: _) {
    prj.insert_ds_test();
    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    uint256 public number = 21;

    function increment() public {
        uint256 i = 0;
        do {
            number++;
            if (number > 20) {
                number -= 2;
            }
        } while (++i < 10);
    }
}
    "#,
    );

    prj.add_source(
        "Counter.t.sol",
        r#"
import "./test.sol";
import "./Counter.sol";

contract CounterTest is DSTest {
    function test_do_while() public {
        Counter counter = new Counter();
        counter.increment();
    }
}
    "#,
    );

    assert_lcov(
        cmd.arg("coverage"),
        str![[r#"
TN:
SF:src/Counter.sol
DA:7,1
FN:7,Counter.increment
FNDA:1,Counter.increment
DA:8,1
DA:10,10
DA:11,10
BRDA:11,0,0,6
DA:12,6
DA:14,10
FNF:1
FNH:1
LF:6
LH:6
BRF:1
BRH:1
end_of_record

"#]],
    );
}

// <https://github.com/foundry-rs/foundry/issues/11183>
// Test that overridden functions are disambiguated in the LCOV report.
#[forgetest]
fn disambiguate_functions(prj: _, cmd: _) {
    prj.insert_ds_test();
    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    uint256 public number;

    function increment() public {
        number++;
    }
    function increment(uint256 amount) public {
        number += amount;
    }
}
    "#,
    );

    prj.add_source(
        "Counter.t.sol",
        r#"
import "./test.sol";
import "./Counter.sol";

contract CounterTest is DSTest {
    function test_overridden() public {
        Counter counter = new Counter();
        counter.increment();
        counter.increment(1);
        counter.increment(2);
        counter.increment(3);
        assertEq(counter.number(), 7);
    }
}
    "#,
    );

    assert_lcov(
        cmd.arg("coverage"),
        str![[r#"
TN:
SF:src/Counter.sol
DA:7,1
FN:7,Counter.increment.0
FNDA:1,Counter.increment.0
DA:8,1
DA:10,3
FN:10,Counter.increment.1
FNDA:3,Counter.increment.1
DA:11,3
FNF:2
FNH:2
LF:4
LH:4
BRF:0
BRH:0
end_of_record

"#]],
    );
}

// Test that functions of abstract contracts and interfaces should not count in coverage report.
#[forgetest]
fn abstract_contract_and_interface(prj: _, cmd: _) {
    prj.insert_ds_test();
    prj.add_source(
        "Counter.sol",
        r#"
interface ContractIf {
    function setNumber(uint256 newNumber) external;
}

abstract contract AbstractCounter {
    function _setNumber(uint256 newNumber) internal virtual;

    function _incrementNumber(uint256 newNumber) internal virtual returns (uint256 inc) {
        inc = newNumber + 1;
    }
}

contract Counter is AbstractCounter, ContractIf {
    uint256 public number;

    function setNumber(uint256 newNumber) public {
        _setNumber(newNumber);
    }

    function _setNumber(uint256 newNumber) internal override {
        number = _incrementNumber(newNumber);
    }

    function _incrementNumber(uint256 newNumber) internal override returns (uint256 inc) {
        inc = super._incrementNumber(newNumber);
    }
}
    "#,
    );
    prj.add_source(
        "CounterTest.sol",
        r#"
import "./test.sol";
import {Counter} from "./Counter.sol";

contract CounterTest is DSTest {
    function testCounter() public {
        Counter counter = new Counter();
        counter.setNumber(0);
    }
}
    "#,
    );

    // Test there are 4 functions reported:
    // - `setNumber`, `_setNumber` and `_incrementNumber` from `Counter` contract
    // - `_incrementNumber` from `AbstractCounter` (virtual with implementation). `_setNumber` is
    // excluded as it is not implemented.
    cmd.arg("coverage").assert_success().stdout_eq(str![[r#"
...
╭-----------------+---------------+---------------+------------+---------------╮
| File            | % Lines       | % Statements  | % Branches | % Funcs       |
+==============================================================================+
| src/Counter.sol | 100.00% (8/8) | 100.00% (4/4) | N/A (0/0)  | 100.00% (4/4) |
|-----------------+---------------+---------------+------------+---------------|
| Total           | 100.00% (8/8) | 100.00% (4/4) | N/A (0/0)  | 100.00% (4/4) |
╰-----------------+---------------+---------------+------------+---------------╯
...
"#]]);
}

// <https://github.com/foundry-rs/foundry/issues/11548>
// Test BRDA hit values follow LCOV spec: "-" when line never executed, "0" when line hit but
// branch not taken.
#[forgetest]
fn brda_lcov_consistency(prj: _, cmd: _) {
    prj.insert_ds_test();
    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    uint256 public number;

    function setPositive(uint256 newNumber) public {
        if (newNumber > 0) {
            number = newNumber;
        } else {
            number = 1;
        }
    }

    function neverCalled(uint256 x) public {
        if (x > 100) {
            number = x;
        } else {
            number = 100;
        }
    }
}
    "#,
    );

    prj.add_source(
        "Counter.t.sol",
        r#"
import "./test.sol";
import "./Counter.sol";

contract CounterTest is DSTest {
    function test_only_positive_branch() public {
        Counter counter = new Counter();
        counter.setPositive(42);
        counter.setPositive(100);
    }
}
    "#,
    );

    // Verify BRDA values:
    // - BRDA:8,0,0,2 - if branch taken 2 times
    // - BRDA:8,0,1,0 - else branch NOT taken but line was hit (outputs "0", not "-")
    // - BRDA:16,1,0,- - if branch NOT taken AND line never executed (outputs "-")
    // - BRDA:16,1,1,- - else branch NOT taken AND line never executed (outputs "-")
    assert_lcov(
        cmd.arg("coverage"),
        str![[r#"
TN:
SF:src/Counter.sol
DA:7,2
FN:7,Counter.setPositive
FNDA:2,Counter.setPositive
DA:8,2
BRDA:8,0,0,2
BRDA:8,0,1,0
DA:9,2
DA:11,0
DA:15,0
FN:15,Counter.neverCalled
FNDA:0,Counter.neverCalled
DA:16,0
BRDA:16,1,0,-
BRDA:16,1,1,-
DA:17,0
DA:19,0
FNF:2
FNH:1
LF:8
LH:3
BRF:4
BRH:1
end_of_record

"#]],
    );
}

// A false condition still executes its line, even when the true body is the first
// coverage item on that line. Unreached nested conditions must remain uncovered.
#[forgetest]
fn lcov_false_condition_lines(prj: _, cmd: _) {
    prj.add_source(
        "Branches.sol",
        r#"contract Branches {
    uint256 public value;
    function run(bool outer, bool inner) public {
        if (outer) { value = 6;
            if (inner) {
                value = 1;
            } else {
                value = 2;
            }
        } else {
            value = 3;
        }
    }
}
"#,
    );
    prj.add_source(
        "Branches.t.sol",
        r#"
import "./Branches.sol";
contract BranchesTest {
    function test_false_conditions() public {
        Branches branches = new Branches();
        branches.run(false, true);
        require(branches.value() == 3);
    }
}
"#,
    );
    assert_lcov(
        cmd.arg("coverage"),
        str![[r#"
TN:
SF:src/Branches.sol
DA:5,1
FN:5,Branches.run
FNDA:1,Branches.run
DA:6,1
BRDA:6,0,0,0
BRDA:6,0,1,1
DA:7,0
BRDA:7,1,0,-
BRDA:7,1,1,-
DA:8,0
DA:10,0
DA:13,1
FNF:1
FNH:1
LF:6
LH:3
BRF:4
BRH:1
end_of_record

"#]],
    );
}

// Keep no-else conditions snapshot-only until implicit false branches are reported:
// strict genhtml validation rejects hit lines with no evaluated branches.
#[forgetest]
fn lcov_false_condition_lines_without_else(prj: _, cmd: _) {
    prj.add_source(
        "Branches.sol",
        r#"contract Branches {
    uint256 public value;
    function run(bool condition) public {
        if (condition) value = 4;
        if (condition)
        {
            value = 5;
        }
    }
}
"#,
    );
    prj.add_source(
        "Branches.t.sol",
        r#"
import "./Branches.sol";
contract BranchesTest {
    function test_false_conditions() public {
        Branches branches = new Branches();
        branches.run(false);
        require(branches.value() == 0);
    }
}
"#,
    );
    cmd.args(["coverage", "--report=lcov", "--report-file"]).assert_file(str![[r#"
TN:
SF:src/Branches.sol
DA:5,1
FN:5,Branches.run
FNDA:1,Branches.run
DA:6,1
BRDA:6,0,0,0
DA:7,1
BRDA:7,1,0,0
DA:9,0
FNF:1
FNH:1
LF:4
LH:3
BRF:2
BRH:0
end_of_record

"#]]);
}

// A hit assembly condition must include the outcome that skips its body, even without via-IR.
#[forgetest]
fn yul_if_lcov(prj: _, cmd: _) {
    prj.add_source(
        "Guard.sol",
        r#"
contract Guard {
    function skipped(uint256 success) external pure returns (uint256) {
        assembly { if iszero(success) { revert(0, 0) } }
        return success;
    }

    function taken(uint256 x) external pure returns (uint256 result) {
        assembly { if x { result := 1 } }
    }

    function mixed(uint256 x) external pure returns (uint256 result) {
        assembly { if x { result := 1 } }
    }

    function never(uint256 x) external pure returns (uint256 result) {
        assembly { if x { result := 1 } }
    }
}
"#,
    );
    prj.add_test(
        "Guard.t.sol",
        r#"
import "../src/Guard.sol";
contract GuardTest {
    function testPaths() public {
        Guard guard = new Guard();
        require(guard.skipped(1) == 1);
        require(guard.taken(1) == 1);
        require(guard.mixed(0) == 0);
        require(guard.mixed(1) == 1);
        require(guard.mixed(2) == 1);
    }
}
"#,
    );
    assert_lcov(
        cmd.arg("coverage"),
        str![[r#"
TN:
SF:src/Guard.sol
DA:5,1
FN:5,Guard.skipped
FNDA:1,Guard.skipped
DA:6,1
BRDA:6,0,0,0
BRDA:6,0,1,1
DA:7,1
DA:10,1
FN:10,Guard.taken
FNDA:1,Guard.taken
DA:11,1
BRDA:11,1,0,1
BRDA:11,1,1,0
DA:14,3
FN:14,Guard.mixed
FNDA:3,Guard.mixed
DA:15,3
BRDA:15,2,0,2
BRDA:15,2,1,1
DA:18,0
FN:18,Guard.never
FNDA:0,Guard.never
DA:19,0
BRDA:19,3,0,-
BRDA:19,3,1,-
FNF:4
FNH:3
LF:9
LH:7
BRF:8
BRH:4
end_of_record

"#]],
    );
}

// Path 0 of a Yul `if` is the side that enters its body, and the `if` owns its own jump, not those
// of the branches and loops in its body, with solc and with solar, whose layouts differ.
#[forgetest]
fn yul_if_paths_solc_and_solar_lcov(prj: _, cmd: _) {
    prj.add_source(
        "Guard.sol",
        r#"
contract Guard {
    uint256 public stored;

    function reverting(uint256 a) external pure {
        assembly { if iszero(a) { revert(0, 0) } }
    }

    function storing(uint256 a) external {
        assembly {
            if lt(a, 100) { sstore(stored.slot, a) sstore(add(stored.slot, 1), a) }
        }
    }

    function leaving(uint256 a) external pure returns (uint256 r) {
        assembly {
            function f(x) -> y {
                if lt(x, 5) { y := 1 leave }
            }
            r := f(a)
        }
    }

    function nested(uint256 a, uint256 b) external {
        assembly {
            if a {
                if b { sstore(stored.slot, 1) }
            }
        }
    }

    function scan(bytes memory hay, uint256 needle, uint256 h) external pure returns (uint256 found) {
        assembly {
            let end := add(add(hay, 0x20), mload(hay))
            let i := add(hay, 0x20)
            for {} lt(i, end) {} {
                if iszero(eq(byte(0, mload(i)), needle)) {
                    if h {
                        if iszero(eq(byte(0, mload(add(i, 1))), h)) {
                            mstore(0x00, i)
                            i := add(i, 1)
                            continue
                        }
                    }
                    found := add(found, 100)
                }
                i := add(i, 1)
            }
        }
    }

    function spin(uint256 a, uint256 n) external pure returns (uint256 i) {
        assembly {
            if a {
                for {} lt(i, n) {} { i := add(i, 1) }
            }
        }
    }
}
"#,
    );
    prj.add_test(
        "Guard.t.sol",
        r#"
import "../src/Guard.sol";
contract GuardTest {
    function testPaths() public {
        Guard guard = new Guard();
        guard.reverting(5);
        guard.reverting(6);
        guard.reverting(7);
        try guard.reverting(0) {} catch {}
        guard.storing(1);
        guard.storing(2);
        guard.storing(3);
        guard.storing(200);
        guard.leaving(1);
        guard.leaving(2);
        guard.leaving(9);
        guard.nested(1, 1);
        guard.nested(1, 0);
        guard.nested(0, 1);
        guard.nested(0, 0);
        guard.nested(0, 0);
        guard.spin(1, 3);
        guard.spin(1, 2);
        guard.spin(0, 5);
        guard.spin(0, 5);
        guard.spin(0, 5);
        guard.scan(hex"0102030201", 2, 0);
        guard.scan(hex"0102030201", 2, 1);
        guard.scan(hex"0102030201", 2, 3);
        guard.scan(hex"0303", 2, 3);
        guard.scan(hex"0202", 2, 3);
    }
}
"#,
    );
    let solar = prj.ensure_foundry_bin("solar");
    let report = str![[r#"
TN:
SF:src/Guard.sol
DA:7,4
FN:7,Guard.reverting
FNDA:4,Guard.reverting
DA:8,4
BRDA:8,0,0,1
BRDA:8,0,1,3
DA:11,4
FN:11,Guard.storing
FNDA:4,Guard.storing
DA:13,4
BRDA:13,1,0,3
BRDA:13,1,1,1
DA:17,3
FN:17,Guard.leaving
FNDA:3,Guard.leaving
DA:19,3
FN:19,Guard.f
FNDA:3,Guard.f
DA:20,3
BRDA:20,2,0,2
BRDA:20,2,1,1
DA:22,3
DA:26,5
FN:26,Guard.nested
FNDA:5,Guard.nested
DA:28,5
BRDA:28,3,0,2
BRDA:28,3,1,3
DA:29,2
BRDA:29,4,0,1
BRDA:29,4,1,1
DA:34,5
FN:34,Guard.scan
FNDA:5,Guard.scan
DA:36,5
DA:37,5
DA:38,24
DA:39,19
BRDA:39,5,0,11
BRDA:39,5,1,8
DA:40,11
BRDA:40,6,0,8
BRDA:40,6,1,3
DA:41,8
BRDA:41,7,0,7
BRDA:41,7,1,1
DA:42,7
DA:43,7
DA:44,7
DA:47,4
DA:49,12
DA:54,5
FN:54,Guard.spin
FNDA:5,Guard.spin
DA:56,5
BRDA:56,8,0,2
BRDA:56,8,1,3
DA:57,7
FNF:7
FNH:7
LF:26
LH:26
BRF:18
BRH:18
end_of_record

"#]];
    assert_lcov(cmd.arg("coverage"), report.clone());
    assert_lcov(cmd.forge_fuse().arg("coverage").arg("--use").arg(solar), report);
}

// A Yul `if` whose body is only a `continue` or a `leave` is the conditional jump itself, and path
// 0 still enters the body.
#[forgetest]
fn yul_if_jump_body_lcov(prj: _, cmd: _) {
    prj.add_source(
        "Guard.sol",
        r#"
contract Guard {
    uint256 public stored;

    function skip(uint256 n) external {
        assembly {
            for { let i := 0 } lt(i, n) { i := add(i, 1) } {
                if eq(i, 2) { continue }
                sstore(stored.slot, i)
            }
        }
    }

    function early(uint256 x) external pure returns (uint256 r) {
        assembly {
            function f(a) -> y {
                if lt(a, 5) { leave }
                y := 3
            }
            r := f(x)
        }
    }
}
"#,
    );
    prj.add_test(
        "Guard.t.sol",
        r#"
import "../src/Guard.sol";
contract GuardTest {
    function testPaths() public {
        Guard guard = new Guard();
        guard.skip(4);
        guard.skip(1);
        guard.early(1);
        guard.early(9);
        guard.early(9);
    }
}
"#,
    );
    assert_lcov(
        cmd.arg("coverage"),
        str![[r#"
TN:
SF:src/Guard.sol
DA:7,2
FN:7,Guard.skip
FNDA:2,Guard.skip
DA:9,7
DA:10,5
BRDA:10,0,0,1
BRDA:10,0,1,4
DA:11,4
DA:16,3
FN:16,Guard.early
FNDA:3,Guard.early
DA:18,3
FN:18,Guard.f
FNDA:3,Guard.f
DA:19,3
BRDA:19,1,0,1
BRDA:19,1,1,2
DA:20,2
DA:22,3
FNF:3
FNH:3
LF:9
LH:9
BRF:4
BRH:4
end_of_record

"#]],
    );
}

// Test that coverage files are written even when tests fail.
#[forgetest]
fn coverage_with_failing_tests(prj: _, cmd: _) {
    prj.insert_ds_test();
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
        "CounterTest.sol",
        r#"
import "./test.sol";
import {Counter} from "./Counter.sol";

contract CounterTest is DSTest {
    Counter public counter;

    function setUp() public {
        counter = new Counter();
        counter.setNumber(0);
    }

    function test_Increment() public {
        counter.increment();
        assertEq(counter.number(), 1);
    }

    function test_FailingTest() public {
        counter.increment();
        // This assertion will fail
        assertEq(counter.number(), 999);
    }
}
    "#,
    );

    // Run coverage - this should exit with error code 1 due to failing test,
    // but the lcov file should still be written.
    cmd.arg("coverage").args(["--report=lcov"]).assert_failure();

    // Verify that the lcov.info file was created despite test failure
    let lcov = prj.root().join("lcov.info");
    assert!(lcov.exists(), "lcov.info should be created even when tests fail");

    // Verify the coverage data is valid and includes the counter contract
    let lcov_content = std::fs::read_to_string(&lcov).unwrap();
    assert!(lcov_content.contains("SF:src/Counter.sol"), "Coverage should include Counter.sol");
    assert!(lcov_content.contains("FN:"), "Coverage should include function data");
    assert!(lcov_content.contains("DA:"), "Coverage should include line hit data");
}

// The lines run by a failing fuzz input are covered, as they are for a failing unit test.
#[forgetest]
fn coverage_includes_failing_fuzz_input(prj: _, cmd: _) {
    prj.add_source(
        "Risky.sol",
        r#"
contract Risky {
    function check(uint256 x) public pure returns (uint256) {
        if (x > 100) {
            return 7;
        }
        return 1;
    }
}
    "#,
    );

    prj.add_source(
        "RiskyTest.sol",
        r#"
import {Risky} from "./Risky.sol";

contract RiskyTest {
    Risky risky = new Risky();

    function testFuzz_Check(uint256 x) public {
        require(risky.check(x) != 7);
    }
}
    "#,
    );

    cmd.arg("coverage").args(["--report=lcov"]).assert_failure();

    // `return 7;` is reached only by the failing input.
    let source = fs::read_to_string(prj.root().join("src/Risky.sol")).unwrap();
    let line = source.lines().position(|l| l.contains("return 7;")).unwrap() + 1;
    let lcov = fs::read_to_string(prj.root().join("lcov.info")).unwrap();
    let hits = lcov
        .lines()
        .find_map(|l| l.strip_prefix(&format!("DA:{line},")))
        .and_then(|hits| hits.parse::<u64>().ok());
    assert!(hits.is_some_and(|hits| hits > 0), "line {line} should be covered:\n{lcov}");
}

#[forgetest_init]
fn coverage_rejects_mutation_mode_before_compile(prj: _, cmd: _) {
    prj.add_source(
        "Broken.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract Broken {
    function broken() public pure returns (uint256) {
        return;
    }
}
"#,
    );

    let output = cmd.args(["coverage", "--mutate", "src/Broken.sol"]).assert_failure();
    let stderr = output.get_output().stderr_lossy();

    assert!(
        stderr.contains("`--mutate` cannot be combined with: coverage"),
        "unexpected stderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("Compiler run failed"),
        "mutation/coverage conflict should be reported before compile errors:\n{stderr}"
    );
}

#[forgetest_init]
fn coverage_rejects_test_only_modes_before_compile(prj: _, cmd: _) {
    prj.add_source(
        "Broken.sol",
        r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract Broken {
    function broken() public pure returns (uint256) {
        return;
    }
}
"#,
    );

    for args in [
        &["--json"][..],
        &["--junit"],
        &["--list"],
        &["--debug"],
        &["--flamegraph"],
        &["--flamechart"],
        &["--evm-profile"],
        &["--showmap-out", "showmap"],
        &["--brutalize"],
        &["--replay-symbolic-artifact", "counterexample.json"],
        &["--watch", "--json"],
    ] {
        let output = cmd.forge_fuse().arg("coverage").args(args).assert_failure();
        let stderr = output.get_output().stderr_lossy();

        assert!(
            stderr.contains("`forge coverage` cannot be combined with:"),
            "unexpected stderr for {args:?}:\n{stderr}"
        );
        assert!(
            !stderr.contains("Compiler run failed"),
            "coverage conflict should be reported before compile errors for {args:?}:\n{stderr}"
        );
    }
}

#[forgetest_init]
fn coverage_match_path_compiles_selected_tests(prj: _, cmd: _) {
    prj.add_test(
        "Shared.t.sol",
        r#"
abstract contract SharedTest {
    function sharedValue() internal pure returns (uint256) {
        return 1;
    }
}
"#,
    );
    prj.add_test(
        "Selected.t.sol",
        r#"
import {SharedTest} from "./Shared.t.sol";

contract SelectedTest is SharedTest {
    function testSelected() public pure {
        require(sharedValue() == 1);
    }
}
"#,
    );
    prj.add_test(
        "Broken.t.sol",
        r#"
contract BrokenTest {
    function broken() public pure returns (uint256) {
        return;
    }
}
"#,
    );
    prj.add_test(
        "Unselected.t.sol",
        r#"
contract UnselectedTest {
    function testUnselected() public {}
}
"#,
    );
    prj.add_script(
        "Report.s.sol",
        r#"
contract ReportScript {
    function run() public {}
}
"#,
    );

    let output = cmd
        .args(["coverage", "--match-path", "test/Selected.t.sol"])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert!(output.contains("testSelected()"), "selected test did not run:\n{output}");
    assert!(
        output.contains("script/Report.s.sol"),
        "scripts should remain in path-filtered coverage reports:\n{output}"
    );
    assert!(
        !output.contains("test/Unselected.t.sol"),
        "unselected tests should be omitted from coverage reports:\n{output}"
    );
}

// <https://github.com/foundry-rs/foundry/issues/16084>
#[forgetest]
fn coverage_separates_compiler_builds(prj: _, cmd: _) {
    prj.update_config(|config| {
        config.additional_compiler_profiles = vec![SettingsOverrides {
            name: "via-ir".to_owned(),
            via_ir: Some(true),
            evm_version: None,
            optimizer: None,
            optimizer_runs: None,
            bytecode_hash: None,
        }];
        config.compilation_restrictions = vec![CompilationRestrictions {
            paths: "src/A.sol".parse().unwrap(),
            version: None,
            via_ir: Some(true),
            bytecode_hash: None,
            min_optimizer_runs: None,
            optimizer_runs: None,
            max_optimizer_runs: None,
            min_evm_version: None,
            evm_version: None,
            max_evm_version: None,
        }];
    });

    prj.add_source(
        "A.sol",
        "contract A { function value() external pure returns (uint256) { return 1; } }",
    );
    prj.add_source(
        "B.sol",
        "contract B { function value() external pure returns (uint256) { return 2; } }",
    );
    prj.add_test(
        "Coverage.t.sol",
        r#"
import {A} from "../src/A.sol";
import {B} from "../src/B.sol";
contract CoverageTest {
    function testCoverage() public {
        require(new A().value() == 1);
        require(new B().value() == 2);
    }
}
"#,
    );

    assert_lcov(
        cmd.arg("coverage"),
        str![[r#"
TN:
SF:src/A.sol
DA:3,1
FN:3,A.value
FNDA:1,A.value
FNF:1
FNH:1
LF:1
LH:1
BRF:0
BRH:0
end_of_record
TN:
SF:src/B.sol
DA:3,1
FN:3,B.value
FNDA:1,B.value
FNF:1
FNH:1
LF:1
LH:1
BRF:0
BRH:0
end_of_record

"#]],
    );
}

// A file-level (free) function declared after a contract must be attributed at file level, not
// leaked into the earlier contract's scope, and its deferred call must carry the same scope.
// Regression for https://github.com/foundry-rs/foundry/issues/16085
#[forgetest]
fn coverage_reports_free_functions(prj: _, cmd: _) {
    prj.insert_ds_test();
    prj.add_source(
        "Mixed.sol",
        r#"
contract Counter {
    function bump(uint256 x) public pure returns (uint256) {
        return triple(x);
    }
}

function triple(uint256 x) pure returns (uint256) {
    return x * 3;
}
"#,
    );
    prj.add_source(
        "MixedTest.sol",
        r#"
import "./test.sol";
import {Counter} from "./Mixed.sol";

contract MixedTest is DSTest {
    function testMixed() public {
        Counter c = new Counter();
        require(c.bump(2) == 6);
    }
}
"#,
    );

    cmd.args(["coverage", "--report=lcov", "--report=attribution", "--lcov-version=2"])
        .assert_success();
    let lcov = std::fs::read_to_string(prj.root().join("lcov.info")).unwrap();
    let attribution =
        std::fs::read_to_string(prj.root().join("coverage-attribution.json")).unwrap();
    let attribution: Value = serde_json::from_str(&attribution).unwrap();

    assert!(lcov.contains("SF:src/Mixed.sol"), "coverage must include Mixed.sol:\n{lcov}");
    // The contract method keeps its contract scope.
    assert!(
        lcov.lines().any(|l| l.starts_with("FN") && l.ends_with(",Counter.bump")),
        "contract function must be scoped as `Counter.bump`:\n{lcov}"
    );
    // The free function is attributed at file level: bare `triple`, with no contract prefix
    // (`Counter.triple`) and no malformed empty scope (`.triple`).
    assert!(
        lcov.lines().any(|l| l.starts_with("FN") && l.ends_with(",triple")),
        "free function must be attributed as bare `triple`:\n{lcov}"
    );
    assert!(
        !lcov.contains(",Counter.triple"),
        "free function must not leak the contract scope:\n{lcov}"
    );
    assert!(
        !lcov.contains(",.triple"),
        "free function must not have a malformed empty scope:\n{lcov}"
    );
    // Both functions were executed, so both record a nonzero hit count.
    assert!(
        lcov.lines()
            .any(|l| l.starts_with("FNDA:") && !l.starts_with("FNDA:0,") && l.ends_with(",triple")),
        "free function must record its hits:\n{lcov}"
    );
    assert!(
        lcov.lines().any(|l| l.starts_with("FNDA:")
            && !l.starts_with("FNDA:0,")
            && l.ends_with(",Counter.bump")),
        "contract function must record its hits:\n{lcov}"
    );
    let covered = attribution["tests"][0]["covered"].as_array().unwrap();
    assert_eq!(
        covered
            .iter()
            .filter(|item| {
                item["source"] == "src/Mixed.sol"
                    && item["contract"] == "Counter"
                    && item["kind"] == "statement"
            })
            .count(),
        2,
        "the return statement and its deferred call must retain the contract scope:\n{attribution}"
    );
}

// A file whose only code is a file-level (free) function must still be reported, with the
// function named at file level. Before the fix such a file was dropped from coverage entirely.
// Regression for https://github.com/foundry-rs/foundry/issues/16085
#[forgetest]
fn coverage_reports_free_only_functions(prj: _, cmd: _) {
    prj.insert_ds_test();
    prj.add_source(
        "Free.sol",
        r#"
function double(uint256 x) pure returns (uint256) {
    return x * 2;
}
"#,
    );
    prj.add_source(
        "FreeTest.sol",
        r#"
import "./test.sol";
import {double} from "./Free.sol";

contract FreeTest is DSTest {
    function testDouble() public {
        require(double(3) == 6);
    }
}
"#,
    );

    cmd.args(["coverage", "--report=lcov", "--lcov-version=2"]).assert_success();
    let lcov = std::fs::read_to_string(prj.root().join("lcov.info")).unwrap();

    // Inspect the Free.sol record in isolation so FNF/FNH counts are unambiguous.
    let free_block = lcov
        .split("SF:")
        .find(|b| b.starts_with("src/Free.sol"))
        .expect("free-function-only file must have a coverage record");
    // The function is named `double`, not `.double`.
    assert!(
        free_block.lines().any(|l| l.starts_with("FN") && l.ends_with(",double")),
        "free function must be reported as bare `double`:\n{free_block}"
    );
    assert!(!free_block.contains(".double"), "no leading-dot empty scope:\n{free_block}");
    // Exactly one function, found and hit.
    assert!(free_block.contains("FNF:1"), "exactly one function found:\n{free_block}");
    assert!(free_block.contains("FNH:1"), "exactly one function hit:\n{free_block}");
}

// A contract method and a file-level function that share a name live in different scopes, so
// neither should be renamed as if it were an overload of the other.
#[forgetest]
fn coverage_disambiguates_functions_by_scope(prj: _, cmd: _) {
    prj.insert_ds_test();
    prj.add_source(
        "Same.sol",
        r#"
contract Lib {
    function value() public pure returns (uint256) {
        return 1;
    }
}

function value() pure returns (uint256) {
    return 2;
}
"#,
    );
    prj.add_source(
        "SameTest.sol",
        r#"
import "./test.sol";
import {Lib, value} from "./Same.sol";

contract SameTest is DSTest {
    function testValue() public {
        Lib lib = new Lib();
        require(lib.value() == 1);
        require(value() == 2);
    }
}
"#,
    );

    cmd.args(["coverage", "--report=lcov", "--lcov-version=2"]).assert_success();
    let lcov = std::fs::read_to_string(prj.root().join("lcov.info")).unwrap();

    // The contract method stays scoped; the free function stays bare.
    assert!(
        lcov.lines().any(|l| l.starts_with("FN") && l.ends_with(",Lib.value")),
        "contract method must be `Lib.value`:\n{lcov}"
    );
    assert!(
        lcov.lines().any(|l| l.starts_with("FN") && l.ends_with(",value")),
        "free function must be bare `value`:\n{lcov}"
    );
    // Neither is a real overload, so neither gets a `.0` / `.1` disambiguation suffix.
    assert!(
        !lcov.contains("value.0") && !lcov.contains("value.1"),
        "same-named functions in different scopes must not be disambiguated as overloads:\n{lcov}"
    );
}

#[test]
fn coverage_help_renders_notes() {
    let help = CoverageArgs::command().render_long_help().to_string();

    assert!(help.contains(concat!(
        "Source attribution:\n",
        "  Coverage follows compiler source maps. Inherited modifier code is reported under"
    )));
    assert!(help.contains("use `--include-libs` to include their coverage."));
    assert!(help.contains(concat!(
        "Compatibility:\n",
        "  `forge coverage` supports test filters and `--watch`, but not test-only output"
    )));
    assert!(!help.contains("\\n"));
}

// Coverage must retain enum bounds and artifact paths across source edits and filtered requests.
#[forgetest]
fn coverage_cache_preserves_reports_and_analysis(prj: _, cmd: _) {
    for dynamic_test_linking in [false, true] {
        prj.clear_cache_dir();
        prj.update_config(|config| {
            config.dynamic_test_linking = dynamic_test_linking;
            config.fuzz.runs = 8;
        });
        prj.add_source(
            "A.sol",
            "contract A { function value() external pure returns (uint256) { return 1; } }",
        );
        prj.add_source("B.sol", "contract B { enum E { X, Y } function value(E e) external pure returns (uint256) { return uint256(e); } }");
        prj.add_source(
            "Unused.sol",
            "function unused(uint256 x) pure returns (uint256) { return x + 1; }",
        );
        prj.add_test("A.t.sol", r#"
import {A} from "../src/A.sol";
interface Vm {
    function getArtifactPathByCode(bytes calldata code) external view returns (string memory);
    function projectRoot() external view returns (string memory);
}
contract ATest {
    function testValue() public {
        require(new A().value() > 0);
    }
    function testArtifactPath() public view {
        Vm vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
        require(keccak256(bytes(vm.getArtifactPathByCode(type(A).creationCode))) == keccak256(bytes(string.concat(vm.projectRoot(), "/out/A.sol/A.json"))));
    }
}
"#);
        prj.add_test(
            "B.t.sol",
            r#"
import {B} from "../src/B.sol";
contract BTest {
    function testEnum(B.E e) public { require(new B().value(e) == uint256(e)); }
}
"#,
        );
        for stage in 0..6 {
            if stage == 1 {
                prj.add_source(
                    "A.sol",
                    "contract A { function value() external pure returns (uint256) { return 2; } }",
                );
            } else if stage == 2 {
                fs::remove_file(prj.root().join("src/Unused.sol")).unwrap();
            } else if stage == 5 {
                prj.update_config(|config| config.evm_version = "paris".parse().unwrap());
            }
            let mut reference = None::<String>;
            for cached in [false, true, true] {
                prj.update_config(|config| config.cache = cached);
                cmd.forge_fuse().args(["coverage", "--report=lcov"]);
                if stage == 3 {
                    cmd.args(["--match-path", "test/A.t.sol"]);
                }
                cmd.assert_success();
                let report = fs::read_to_string(prj.root().join("lcov.info")).unwrap();
                if let Some(expected) = &reference {
                    foundry_test_utils::snapbox::assert_data_eq!(report, expected.clone());
                } else {
                    reference = Some(report);
                }
            }
        }
    }
}

// The compiler can prune a build containing only free functions because it has no contract
// artifacts. A warm coverage run must still report that source.
#[forgetest]
fn coverage_cache_preserves_free_only_builds(prj: _, cmd: _) {
    prj.update_config(|config| {
        config.additional_compiler_profiles = vec![SettingsOverrides {
            name: "via-ir".to_owned(),
            via_ir: Some(true),
            evm_version: None,
            optimizer: None,
            optimizer_runs: None,
            bytecode_hash: None,
        }];
        config.compilation_restrictions = vec![CompilationRestrictions {
            paths: "src/Free.sol".parse().unwrap(),
            version: None,
            via_ir: Some(true),
            bytecode_hash: None,
            min_optimizer_runs: None,
            optimizer_runs: None,
            max_optimizer_runs: None,
            min_evm_version: None,
            evm_version: None,
            max_evm_version: None,
        }];
    });
    prj.add_source("Free.sol", "function unused() pure returns (uint256) { return 42; }");
    prj.add_test("Coverage.t.sol", "contract CoverageTest { function testPass() public {} }");
    let mut reference = None::<String>;
    for cached in [false, true, true] {
        prj.update_config(|config| config.cache = cached);
        cmd.forge_fuse().args(["coverage", "--report=lcov"]).assert_success();
        let report = fs::read_to_string(prj.root().join("lcov.info")).unwrap();
        if let Some(expected) = &reference {
            foundry_test_utils::snapbox::assert_data_eq!(report, expected.clone());
        } else {
            reference = Some(report);
        }
    }
}

#[forgetest]
fn coverage_cache_isolated_and_cleaned(prj: _, cmd: _) {
    prj.update_config(|config| {
        config.cache_path = "custom-cache".into();
        config.out = "custom-out".into();
        config.build_info = true;
        config.build_info_path = Some("custom-build-info".into());
    });
    prj.add_source(
        "A.sol",
        "contract A { function value() external pure returns (uint256) { return 1; } }",
    );
    prj.add_test("A.t.sol", "import {A} from '../src/A.sol'; contract ATest { function testValue() public { require(new A().value() == 1); } }");
    cmd.forge_fuse().arg("build").assert_success();
    let artifact = prj.root().join("custom-out/A.sol/A.json");
    let original = fs::read(&artifact).unwrap();
    let cache = prj.root().join("custom-cache/coverage");
    for _ in 0..2 {
        cmd.forge_fuse().arg("coverage").assert_success();
        assert_eq!(fs::read(&artifact).unwrap(), original);
        assert!(cache.is_dir());
    }
    cmd.forge_fuse().arg("coverage").assert_success().stdout_eq(str![[r#"
No files changed, compilation skipped
...
"#]]);
    let marker = cache.join("force-marker");
    fs::write(&marker, "old cache").unwrap();
    cmd.forge_fuse().args(["coverage", "--force"]).assert_success();
    assert!(!marker.exists());
    assert!(!artifact.exists());
    cmd.forge_fuse().arg("coverage").assert_success().stdout_eq(str![[r#"
No files changed, compilation skipped
...
"#]]);
    cmd.forge_fuse().arg("clean").assert_success();
    assert!(!cache.exists());
    prj.update_config(|config| config.cache = false);
    cmd.forge_fuse().arg("coverage").assert_success();
    assert!(!cache.exists());
}

#[forgetest]
fn coverage_cache_respects_warning_denial(prj: _, cmd: _) {
    prj.add_test(
        "Warning.t.sol",
        "contract WarningTest { function testWarning() public { uint256 unused = 1; } }",
    );
    cmd.forge_fuse().arg("coverage").assert_success();
    // Warning denial must compile again because cached artifacts do not retain diagnostics.
    cmd.forge_fuse().args(["coverage", "--deny", "warnings"]).assert_failure();
}

#[forgetest]
fn coverage_cache_preserves_artifact_names_after_deletion(prj: _, cmd: _) {
    prj.update_config(|config| config.out = "custom-out".into());
    prj.add_source(
        "A.sol",
        "contract A { function value() public pure returns (uint256) { return 1; } }",
    );
    prj.add_source(
        "nested/A.sol",
        "contract A { function value() public pure returns (uint256) { return 2; } }",
    );
    for (stage, expected) in ["nested/A.sol/A.json", "A.sol/A.json"].into_iter().enumerate() {
        if stage == 1 {
            fs::remove_file(prj.root().join("src/A.sol")).unwrap();
        }
        prj.add_test("Names.t.sol", &format!(r#"
import {{A}} from "../src/nested/A.sol";
interface Vm {{
    function getArtifactPathByCode(bytes calldata code) external view returns (string memory);
    function getArtifactPathByDeployedCode(bytes calldata code) external view returns (string memory);
    function projectRoot() external view returns (string memory);
}}
contract NamesTest {{
    function testNames() public view {{
        Vm vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
        bytes32 expected = keccak256(bytes(string.concat(vm.projectRoot(), "/custom-out/{expected}")));
        require(keccak256(bytes(vm.getArtifactPathByCode(type(A).creationCode))) == expected);
        require(keccak256(bytes(vm.getArtifactPathByDeployedCode(type(A).runtimeCode))) == expected);
    }}
}}
"#));
        for cached in [false, true, true] {
            prj.update_config(|config| config.cache = cached);
            cmd.forge_fuse().arg("coverage").assert_success();
        }
    }
}

// Read-only restored cache files must not turn successful coverage into an error.
#[forgetest]
fn coverage_cache_read_only_files(prj: _, cmd: _) {
    prj.add_source(
        "A.sol",
        "contract A { function value() public pure returns (uint256) { return 1; } }",
    );
    prj.add_test("A.t.sol", "import {A} from '../src/A.sol'; contract ATest { function testValue() public { require(new A().value() > 0); } }");
    cmd.forge_fuse().args(["coverage", "--report=lcov"]).assert_success();
    let reference = fs::read_to_string(prj.root().join("lcov.info")).unwrap();
    let cache = prj.root().join("cache/coverage");
    let compiler_cache = cache.join(SOLIDITY_FILES_CACHE_FILENAME);
    let permissions = std::fs::metadata(&compiler_cache).unwrap().permissions();
    let mut read_only = permissions.clone();
    read_only.set_readonly(true);
    std::fs::set_permissions(&compiler_cache, read_only).unwrap();
    let output = cmd.forge_fuse().args(["coverage", "--report=lcov"]).assert();
    std::fs::set_permissions(&compiler_cache, permissions).unwrap();
    output.success();
    foundry_test_utils::snapbox::assert_data_eq!(
        fs::read_to_string(prj.root().join("lcov.info")).unwrap(),
        reference.clone()
    );

    let marker = cache.join(Config::COVERAGE_CACHE_MARKER);
    for edited in [false, true] {
        if edited {
            prj.add_source(
                "A.sol",
                "contract A { function value() public pure returns (uint256) { return 2; } }",
            );
        }
        let permissions = std::fs::metadata(&marker).unwrap().permissions();
        let mut read_only = permissions.clone();
        read_only.set_readonly(true);
        std::fs::set_permissions(&marker, read_only).unwrap();
        let output = cmd.forge_fuse().args(["coverage", "--report=lcov"]).assert();
        std::fs::set_permissions(&marker, permissions).unwrap();
        output.success();
        foundry_test_utils::snapbox::assert_data_eq!(
            fs::read_to_string(prj.root().join("lcov.info")).unwrap(),
            reference.clone()
        );
    }
    // The failed reset must leave the cache invalidated by its old fingerprint, so the next
    // writable run recompiles before subsequent runs can skip compilation.
    cmd.forge_fuse().arg("coverage").assert_success();
    cmd.forge_fuse().arg("coverage").assert_success().stdout_eq(str![[r#"
No files changed, compilation skipped
...
"#]]);
}

#[forgetest]
fn coverage_cache_clean_respects_lock(prj: _, cmd: _) {
    prj.add_test("A.t.sol", "contract ATest { function testPass() public {} }");
    cmd.forge_fuse().arg("coverage").assert_success();
    let cache = prj.root().join("cache/coverage");
    let lock = Config::lock_coverage_cache(&cache).unwrap();
    cmd.forge_fuse().arg("clean").assert_success().stderr_eq(str![[r#"
Warning: failed to remove coverage cache [..]: [..]
"#]]);
    assert!(cache.join(Config::COVERAGE_CACHE_MARKER).is_file());
    // Coverage also falls back while another operation holds the lock.
    cmd.forge_fuse().arg("coverage").assert_success();
    drop(lock);
    cmd.forge_fuse().arg("coverage").assert_success().stdout_eq(str![[r#"
No files changed, compilation skipped
...
"#]]);
    cmd.forge_fuse().arg("clean").assert_success().stderr_eq("");
    assert!(!cache.exists());
}

#[forgetest]
fn coverage_cache_prunes_obsolete_files(prj: _, cmd: _) {
    prj.update_config(|config| config.build_info = true);
    prj.add_source("Removed.sol", "contract Removed {}");
    prj.add_test("A.t.sol", "contract ATest { function testPass() public {} }");
    cmd.forge_fuse().arg("coverage").assert_success();
    let cache = prj.root().join("cache/coverage");
    let mut builds = files_with_ext(&cache.join("build-info"), "json").collect::<Vec<_>>();
    assert_eq!(builds.len(), 1);
    for edited in [false, true] {
        if edited {
            prj.add_test(
                "A.t.sol",
                "contract ATest { function testPass() public {} function testOther() public {} }",
            );
        } else {
            fs::remove_file(prj.root().join("src/Removed.sol")).unwrap();
        }
        cmd.forge_fuse().arg("coverage").assert_success();
        assert!(builds.iter().all(|path| !path.exists()));
        assert!(!cache.join("artifacts/Removed.sol/Removed.json").exists());
        builds = files_with_ext(&cache.join("build-info"), "json").collect();
        assert_eq!(builds.len(), 1);
    }
}

// Only bytecode matching an artifact is credited to its source; constructor data may vary.
#[forgetest]
fn coverage_credits_only_exact_bytecode(prj: _, cmd: _) {
    prj.add_source(
        "Target.sol",
        r#"
contract Target {
    uint256 public value;

    constructor(uint256 x) {
        value = x;
    }

    function live() external pure returns (uint256) {
        return 0x1111;
    }
}
"#,
    );
    prj.add_test(
        "Target.t.sol",
        r#"
import {Target} from "../src/Target.sol";

interface Vm {
    function etch(address target, bytes calldata newRuntimeBytecode) external;
}

contract TargetTest {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));

    // Patches `PUSH2 0x1111` to `PUSH2 0x2222`.
    function patch(bytes memory code) internal pure returns (bytes memory) {
        for (uint256 i; i + 2 < code.length; ++i) {
            if (code[i] == 0x61 && code[i + 1] == 0x11 && code[i + 2] == 0x11) {
                code[i + 1] = 0x22;
                code[i + 2] = 0x22;
                return code;
            }
        }
        revert("constant not found");
    }

    function deploy(bytes memory initCode) internal returns (Target created) {
        assembly {
            created := create(0, add(initCode, 0x20), mload(initCode))
        }
    }

    function testModifiedCode() external {
        address etched = address(0x1234);
        vm.etch(etched, patch(type(Target).runtimeCode));
        require(Target(etched).live() == 0x2222);

        Target created =
            deploy(patch(abi.encodePacked(type(Target).creationCode, abi.encode(uint256(1)))));
        require(created.value() == 1 && created.live() == 0x2222);
    }

    function testConstructorTrailingData() external {
        Target created = deploy(
            abi.encodePacked(type(Target).creationCode, abi.encode(uint256(42)), hex"ff")
        );
        require(created.value() == 42);
    }
}
"#,
    );

    cmd.args(["coverage", "--report=summary", "--mt", "testModifiedCode"])
        .assert_success()
        .stdout_eq(str![[r#"
...
╭----------------+-------------+--------------+------------+-------------╮
| File           | % Lines     | % Statements | % Branches | % Funcs     |
+========================================================================+
| src/Target.sol | 0.00% (0/4) | 0.00% (0/2)  | N/A (0/0)  | 0.00% (0/2) |
|----------------+-------------+--------------+------------+-------------|
| Total          | 0.00% (0/4) | 0.00% (0/2)  | N/A (0/0)  | 0.00% (0/2) |
╰----------------+-------------+--------------+------------+-------------╯

"#]])
        .stderr_eq(str![[r#"
...
Warning: excluding executed [..] code from coverage: it resembles `src/Target.sol:Target` but does not match it exactly
Warning: excluding executed [..] code from coverage: it resembles `src/Target.sol:Target` but does not match it exactly

"#]]);

    cmd.forge_fuse()
        .args(["coverage", "--report=summary", "--mt", "testConstructorTrailingData"])
        .assert_success()
        .stdout_eq(str![[r#"
...
╭----------------+--------------+--------------+------------+--------------╮
| File           | % Lines      | % Statements | % Branches | % Funcs      |
+==========================================================================+
| src/Target.sol | 50.00% (2/4) | 50.00% (1/2) | N/A (0/0)  | 50.00% (1/2) |
|----------------+--------------+--------------+------------+--------------|
| Total          | 50.00% (2/4) | 50.00% (1/2) | N/A (0/0)  | 50.00% (1/2) |
╰----------------+--------------+--------------+------------+--------------╯

"#]]);
}

#[forgetest]
fn coverage_bytecode_report(prj: _, cmd: _) {
    prj.add_source(
        "Counter.sol",
        "contract Counter { function get() public pure returns (uint256) { return 42; } }",
    );
    prj.add_test(
        "Counter.t.sol",
        r#"
import "src/Counter.sol";

contract CounterTest {
    function testGet() public {
        require(new Counter().get() == 42);
    }
}
"#,
    );
    cmd.args(["coverage", "--report=bytecode"]).assert_success();
    let asm = fs::read_to_string(prj.root().join("bytecode-coverage/Counter.asm")).unwrap();
    assert_data_eq!(
        asm,
        str![[r#"
[002] 00000000: PUSH1 0x80                     // src/Counter.sol: 3:1-3:81 (71-151)
[002] 00000002: PUSH1 0x40                     // src/Counter.sol: 3:1-3:81 (71-151)
[002] 00000004: MSTORE                         // src/Counter.sol: 3:1-3:81 (71-151)
[002] 00000005: CALLVALUE                      // src/Counter.sol: 3:1-3:81 (71-151)
[002] 00000006: DUP1                           // src/Counter.sol: 3:1-3:81 (71-151)
[002] 00000007: ISZERO                         // src/Counter.sol: 3:1-3:81 (71-151)
[002] 00000008: PUSH1 0x0e                     // src/Counter.sol: 3:1-3:81 (71-151)
[002] 0000000a: JUMPI                          // src/Counter.sol: 3:1-3:81 (71-151)
      0000000b: PUSH0                          // src/Counter.sol: 3:1-3:81 (71-151)
...
"#]]
    );
}

// Gaps report offsets and snippets do not depend on CRLF line endings or multibyte text.
#[forgetest]
fn coverage_gaps_report(prj: _, cmd: _) {
    let source = prj.add_source(
        "AContract.sol",
        r#"
contract AContract {
    uint256 public value;

    function set(uint256 x) public {
        // ☃
        if (x > 10) {
            value = x;
        } else {
            value = 0;
        }
    }

    function check(uint256 x) public pure {
        require(x > 0, "zero");
    }

    function unused() public {
        value = 42;
    }
}
"#,
    );
    prj.add_test(
        "AContract.t.sol",
        r#"
import {AContract} from "../src/AContract.sol";

contract AContractTest {
    function testSet() public {
        AContract a = new AContract();
        a.set(11);
        a.check(1);
    }
}
"#,
    );

    let expected = str![[r#"
{
  "version": 1,
  "files": {
    "src/AContract.sol": [
      {
        "kind": "branch",
        "contract": "AContract",
        "function": "set",
        "line_start": 9,
        "line_end": 13,
        "byte_start": 180,
        "byte_end": 266,
        "snippet": "if (x > 10) {",
        "branch_id": 0,
        "path_id": 1
      },
      {
        "kind": "line",
        "contract": "AContract",
        "function": "set",
        "line_start": 12,
        "line_end": 12,
        "byte_start": 246,
        "byte_end": 255,
        "snippet": "value = 0"
      },
      {
        "kind": "branch",
        "contract": "AContract",
        "function": "check",
        "line_start": 17,
        "line_end": 17,
        "byte_start": 326,
        "byte_end": 348,
        "snippet": "require(x > 0, \"zero\")",
        "branch_id": 1,
        "path_id": 0
      },
      {
        "kind": "line",
        "contract": "AContract",
        "function": "unused",
        "line_start": 20,
        "line_end": 20,
        "byte_start": 361,
        "byte_end": 387,
        "snippet": "function unused() public {"
      },
      {
        "kind": "function",
        "contract": "AContract",
        "function": "unused",
        "line_start": 20,
        "line_end": 22,
        "byte_start": 361,
        "byte_end": 413,
        "snippet": "function unused() public {"
      },
      {
        "kind": "line",
        "contract": "AContract",
        "function": "unused",
        "line_start": 21,
        "line_end": 21,
        "byte_start": 396,
        "byte_end": 406,
        "snippet": "value = 42"
      }
    ]
  }
}
"#]]
    .is_json();
    cmd.args(["coverage", "--report=gaps"]).assert_success();
    assert_data_eq!(
        Data::read_from(&prj.root().join("coverage-gaps.json"), None),
        expected.clone()
    );

    let crlf = fs::read_to_string(&source).unwrap().replace('\n', "\r\n");
    fs::write(&source, crlf).unwrap();
    cmd.forge_fuse()
        .args(["coverage", "--report=gaps", "--report-file", "crlf-gaps.json"])
        .assert_success();
    assert_data_eq!(Data::read_from(&prj.root().join("crlf-gaps.json"), None), expected);
}

// Dynamic test linking rewrites `new Child()` in test helpers before compilation, so source maps
// refer to the rewritten text. Items after the rewrite must still anchor to their own code.
#[forgetest]
fn dynamic_test_linking_source_edits(prj: _, cmd: _) {
    prj.insert_ds_test();
    prj.add_source(
        "Child.sol",
        r#"
contract Child {
    uint256 public value;

    constructor(uint256 _value) {
        value = _value;
    }
}
    "#,
    );
    prj.add_test(
        "Factory.sol",
        r#"
import {Child} from "../src/Child.sol";

contract Factory {
    function make(uint256 n) public returns (uint256 total) {
        Child child = new Child(1);
        total = child.value();
        if (n > 1) {
            total += n;
        } else {
            total += 100;
        }
        total += 1;
    }

    function skipped() public pure returns (uint256 x) {
        x = 5;
    }
}
    "#,
    );
    prj.add_test(
        "Factory.t.sol",
        r#"
import "../src/test.sol";
import {Factory} from "./Factory.sol";

contract FactoryTest is DSTest {
    function testMake() public {
        Factory factory = new Factory();
        assertEq(factory.make(2), 4);
    }
}
    "#,
    );

    let solar = prj.ensure_foundry_bin("solar");
    for solar in [None, Some(&solar)] {
        let cmd = cmd.forge_fuse();
        cmd.arg("coverage");
        if let Some(solar) = solar {
            cmd.arg("--use").arg(solar);
        }
        assert_lcov(
            cmd,
            str![[r#"
TN:
SF:src/Child.sol
DA:7,1
FN:7,Child.constructor
FNDA:1,Child.constructor
DA:8,1
FNF:1
FNH:1
LF:2
LH:2
BRF:0
BRH:0
end_of_record
TN:
SF:test/Factory.sol
DA:7,1
FN:7,Factory.make
FNDA:1,Factory.make
DA:8,1
DA:9,1
DA:10,1
BRDA:10,0,0,1
BRDA:10,0,1,0
DA:11,1
DA:13,0
DA:15,1
DA:18,0
FN:18,Factory.skipped
FNDA:0,Factory.skipped
DA:19,0
FNF:2
FNH:1
LF:9
LH:6
BRF:2
BRH:1
end_of_record

"#]],
        );
    }
}
