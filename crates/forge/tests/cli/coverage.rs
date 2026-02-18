use foundry_common::fs::{self, files_with_ext};
use foundry_test_utils::{
    TestCommand, TestProject,
    snapbox::{Data, IntoData},
};
use std::path::Path;

#[track_caller]
fn assert_lcov(cmd: &mut TestCommand, data: impl IntoData) {
    cmd.args(["--report=lcov", "--report-file"]).assert_file(data.into_data());
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

╭----------------------+---------------+---------------+---------------+---------------╮
| File                 | % Lines       | % Statements  | % Branches    | % Funcs       |
+======================================================================================+
| script/Counter.s.sol | 0.00% (0/5)   | 0.00% (0/3)   | 100.00% (0/0) | 0.00% (0/2)   |
|----------------------+---------------+---------------+---------------+---------------|
| src/Counter.sol      | 100.00% (4/4) | 100.00% (2/2) | 100.00% (0/0) | 100.00% (2/2) |
|----------------------+---------------+---------------+---------------+---------------|
| Total                | 44.44% (4/9)  | 40.00% (2/5)  | 100.00% (0/0) | 50.00% (2/4)  |
╰----------------------+---------------+---------------+---------------+---------------╯

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

forgetest_init!(basic, |prj, cmd| {
    prj.initialize_default_contracts();
    basic_base(prj, cmd);
});

forgetest_init!(basic_crlf, |prj, cmd| {
    prj.initialize_default_contracts();
    // Manually replace `\n` with `\r\n` in the source file.
    let make_crlf = |path: &Path| {
        fs::write(path, fs::read_to_string(path).unwrap().replace('\n', "\r\n")).unwrap()
    };
    make_crlf(&prj.paths().sources.join("Counter.sol"));
    make_crlf(&prj.paths().scripts.join("Counter.s.sol"));

    // Should have identical stdout and lcov output.
    basic_base(prj, cmd);
});

forgetest!(setup, |prj, cmd| {
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
╭-------------------+---------------+---------------+---------------+---------------╮
| File              | % Lines       | % Statements  | % Branches    | % Funcs       |
+===================================================================================+
| src/AContract.sol | 100.00% (4/4) | 100.00% (2/2) | 100.00% (0/0) | 100.00% (2/2) |
|-------------------+---------------+---------------+---------------+---------------|
| Total             | 100.00% (4/4) | 100.00% (2/2) | 100.00% (0/0) | 100.00% (2/2) |
╰-------------------+---------------+---------------+---------------+---------------╯

"#]]);
});

forgetest!(setup_md, |prj, cmd| {
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
| File              | % Lines       | % Statements  | % Branches    | % Funcs       |
|-------------------|---------------|---------------|---------------|---------------|
| src/AContract.sol | 100.00% (4/4) | 100.00% (2/2) | 100.00% (0/0) | 100.00% (2/2) |
| Total             | 100.00% (4/4) | 100.00% (2/2) | 100.00% (0/0) | 100.00% (2/2) |

"#]]);
});

forgetest!(no_match, |prj, cmd| {
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
╭-------------------+---------------+---------------+---------------+---------------╮
| File              | % Lines       | % Statements  | % Branches    | % Funcs       |
+===================================================================================+
| src/BContract.sol | 100.00% (4/4) | 100.00% (2/2) | 100.00% (0/0) | 100.00% (2/2) |
|-------------------+---------------+---------------+---------------+---------------|
| Total             | 100.00% (4/4) | 100.00% (2/2) | 100.00% (0/0) | 100.00% (2/2) |
╰-------------------+---------------+---------------+---------------+---------------╯

"#
    ]]);
});

forgetest!(assert, |prj, cmd| {
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
╭-------------------+--------------+--------------+---------------+---------------╮
| File              | % Lines      | % Statements | % Branches    | % Funcs       |
+=================================================================================+
| src/AContract.sol | 66.67% (2/3) | 50.00% (1/2) | 100.00% (0/0) | 100.00% (1/1) |
|-------------------+--------------+--------------+---------------+---------------|
| Total             | 66.67% (2/3) | 50.00% (1/2) | 100.00% (0/0) | 100.00% (1/1) |
╰-------------------+--------------+--------------+---------------+---------------╯

"#]
    ]);

    // Assert 100% statement coverage for proper assert (assert not considered a branch).
    cmd.forge_fuse().arg("coverage").args(["--mt", "testAssertBranch"]).assert_success().stdout_eq(
        str![[r#"
...
╭-------------------+---------------+---------------+---------------+---------------╮
| File              | % Lines       | % Statements  | % Branches    | % Funcs       |
+===================================================================================+
| src/AContract.sol | 100.00% (3/3) | 100.00% (2/2) | 100.00% (0/0) | 100.00% (1/1) |
|-------------------+---------------+---------------+---------------+---------------|
| Total             | 100.00% (3/3) | 100.00% (2/2) | 100.00% (0/0) | 100.00% (1/1) |
╰-------------------+---------------+---------------+---------------+---------------╯

"#]],
    );
});

forgetest!(require, |prj, cmd| {
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
});

forgetest!(line_hit_not_doubled, |prj, cmd| {
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
});

forgetest!(branch, |prj, cmd| {
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
});

forgetest!(function_call, |prj, cmd| {
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
╭-------------------+-----------------+---------------+---------------+---------------╮
| File              | % Lines         | % Statements  | % Branches    | % Funcs       |
+=====================================================================================+
| src/AContract.sol | 100.00% (14/14) | 100.00% (9/9) | 100.00% (0/0) | 100.00% (5/5) |
|-------------------+-----------------+---------------+---------------+---------------|
| Total             | 100.00% (14/14) | 100.00% (9/9) | 100.00% (0/0) | 100.00% (5/5) |
╰-------------------+-----------------+---------------+---------------+---------------╯

"#]]);
});

forgetest!(try_catch, |prj, cmd| {
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
});

forgetest!(yul, |prj, cmd| {
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
| src/Foo.sol | 100.00% (30/30) | 100.00% (40/40) | 100.00% (1/1) | 100.00% (7/7) |
|-------------+-----------------+-----------------+---------------+---------------|
| Total       | 100.00% (30/30) | 100.00% (40/40) | 100.00% (1/1) | 100.00% (7/7) |
╰-------------+-----------------+-----------------+---------------+---------------╯

"#]]);
});

forgetest!(misc, |prj, cmd| {
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
╭-------------+-----------------+---------------+---------------+---------------╮
| File        | % Lines         | % Statements  | % Branches    | % Funcs       |
+===============================================================================+
| src/Foo.sol | 100.00% (12/12) | 100.00% (9/9) | 100.00% (0/0) | 100.00% (4/4) |
|-------------+-----------------+---------------+---------------+---------------|
| Total       | 100.00% (12/12) | 100.00% (9/9) | 100.00% (0/0) | 100.00% (4/4) |
╰-------------+-----------------+---------------+---------------+---------------╯

"#]]);
});

// https://github.com/foundry-rs/foundry/issues/8605
forgetest!(single_statement, |prj, cmd| {
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
╭-------------------+--------------+--------------+--------------+---------------╮
| File              | % Lines      | % Statements | % Branches   | % Funcs       |
+================================================================================+
| src/AContract.sol | 62.50% (5/8) | 57.14% (4/7) | 50.00% (3/6) | 100.00% (1/1) |
|-------------------+--------------+--------------+--------------+---------------|
| Total             | 62.50% (5/8) | 57.14% (4/7) | 50.00% (3/6) | 100.00% (1/1) |
╰-------------------+--------------+--------------+--------------+---------------╯

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
});

forgetest!(single_statement_loop, |prj, cmd| {
    // TODO(dani): the specific case of `if (x) continue/break` is not properly covered.
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

    // Assert 50% coverage for true branches.
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

    // Assert 50% coverage for false branches.
    cmd.forge_fuse()
        .arg("coverage")
        .args(["--mt", "testFalseCoverage"])
        .assert_success()
        .stdout_eq(str![[r#"
...
╭-------------------+---------------+-----------------+---------------+---------------╮
| File              | % Lines       | % Statements    | % Branches    | % Funcs       |
+=====================================================================================+
| src/AContract.sol | 100.00% (7/7) | 100.00% (10/10) | 100.00% (2/2) | 100.00% (1/1) |
|-------------------+---------------+-----------------+---------------+---------------|
| Total             | 100.00% (7/7) | 100.00% (10/10) | 100.00% (2/2) | 100.00% (1/1) |
╰-------------------+---------------+-----------------+---------------+---------------╯

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
});

// https://github.com/foundry-rs/foundry/issues/8604
forgetest!(branch_with_calldata_reads, |prj, cmd| {
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
});

forgetest!(identical_bytecodes, |prj, cmd| {
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
╭-------------------+-----------------+---------------+---------------+---------------╮
| File              | % Lines         | % Statements  | % Branches    | % Funcs       |
+=====================================================================================+
| src/AContract.sol | 100.00% (12/12) | 100.00% (9/9) | 100.00% (0/0) | 100.00% (3/3) |
|-------------------+-----------------+---------------+---------------+---------------|
| Total             | 100.00% (12/12) | 100.00% (9/9) | 100.00% (0/0) | 100.00% (3/3) |
╰-------------------+-----------------+---------------+---------------+---------------╯

"#]]);
});

forgetest!(constructors, |prj, cmd| {
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
╭-------------------+---------------+---------------+---------------+---------------╮
| File              | % Lines       | % Statements  | % Branches    | % Funcs       |
+===================================================================================+
| src/AContract.sol | 100.00% (4/4) | 100.00% (2/2) | 100.00% (0/0) | 100.00% (2/2) |
|-------------------+---------------+---------------+---------------+---------------|
| Total             | 100.00% (4/4) | 100.00% (2/2) | 100.00% (0/0) | 100.00% (2/2) |
╰-------------------+---------------+---------------+---------------+---------------╯

"#]]);
});

// https://github.com/foundry-rs/foundry/issues/9270, https://github.com/foundry-rs/foundry/issues/9444
// Test that special functions with no statements are not counted.
// TODO: We should support this, but for now just ignore them.
// See TODO in `visit_function_definition`: https://github.com/foundry-rs/foundry/issues/9458
forgetest!(empty_functions, |prj, cmd| {
    prj.insert_ds_test();
    prj.add_source(
        "AContract.sol",
        r#"
contract AContract {
    constructor() {}

    receive() external payable {}

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
    }
}
    "#,
    );

    assert_lcov(
        cmd.arg("coverage"),
        str![[r#"
TN:
SF:src/AContract.sol
DA:9,1
FN:9,AContract.increment
FNDA:1,AContract.increment
FNF:1
FNH:1
LF:1
LH:1
BRF:0
BRH:0
end_of_record

"#]],
    );

    // Assert there's only one function (`increment`) reported.
    cmd.forge_fuse().arg("coverage").assert_success().stdout_eq(str![[r#"
...
╭-------------------+---------------+---------------+---------------+---------------╮
| File              | % Lines       | % Statements  | % Branches    | % Funcs       |
+===================================================================================+
| src/AContract.sol | 100.00% (1/1) | 100.00% (0/0) | 100.00% (0/0) | 100.00% (1/1) |
|-------------------+---------------+---------------+---------------+---------------|
| Total             | 100.00% (1/1) | 100.00% (0/0) | 100.00% (0/0) | 100.00% (1/1) |
╰-------------------+---------------+---------------+---------------+---------------╯

"#]]);
});

// Test coverage for `receive` functions.
forgetest!(receive, |prj, cmd| {
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

    // Assert both constructor and receive functions coverage reported and appear in LCOV.
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
FNF:2
FNH:2
LF:4
LH:4
BRF:0
BRH:0
end_of_record

"#]],
    );

    cmd.forge_fuse().arg("coverage").assert_success().stdout_eq(str![[r#"
...
╭-------------------+---------------+---------------+---------------+---------------╮
| File              | % Lines       | % Statements  | % Branches    | % Funcs       |
+===================================================================================+
| src/AContract.sol | 100.00% (4/4) | 100.00% (2/2) | 100.00% (0/0) | 100.00% (2/2) |
|-------------------+---------------+---------------+---------------+---------------|
| Total             | 100.00% (4/4) | 100.00% (2/2) | 100.00% (0/0) | 100.00% (2/2) |
╰-------------------+---------------+---------------+---------------+---------------╯

"#]]);
});

// https://github.com/foundry-rs/foundry/issues/9322
// Test coverage with `--ir-minimum` for solidity < 0.8.5.
forgetest!(ir_minimum_early, |prj, cmd| {
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
╭-------------------+-------------+--------------+---------------+-------------╮
| File              | % Lines     | % Statements | % Branches    | % Funcs     |
+==============================================================================+
| src/AContract.sol | 0.00% (0/5) | 0.00% (0/4)  | 100.00% (0/0) | 0.00% (0/1) |
|-------------------+-------------+--------------+---------------+-------------|
| Total             | 0.00% (0/5) | 0.00% (0/4)  | 100.00% (0/0) | 0.00% (0/1) |
╰-------------------+-------------+--------------+---------------+-------------╯

"#]]);
});

forgetest!(no_artifacts_written, |prj, cmd| {
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
╭-------------------+---------------+---------------+---------------+---------------╮
| File              | % Lines       | % Statements  | % Branches    | % Funcs       |
+===================================================================================+
| src/AContract.sol | 100.00% (4/4) | 100.00% (2/2) | 100.00% (0/0) | 100.00% (2/2) |
|-------------------+---------------+---------------+---------------+---------------|
| Total             | 100.00% (4/4) | 100.00% (2/2) | 100.00% (0/0) | 100.00% (2/2) |
╰-------------------+---------------+---------------+---------------+---------------╯
...
"#]]);

    // no artifacts are to be written
    let files = files_with_ext(prj.artifacts(), "json").collect::<Vec<_>>();

    assert!(files.is_empty());
});

// <https://github.com/foundry-rs/foundry/issues/10172>
forgetest!(constructor_with_args, |prj, cmd| {
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
});

// https://github.com/foundry-rs/foundry/issues/11432
// Test coverage for linked libraries.
forgetest!(linked_library, |prj, cmd| {
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
import {Counter} from "./Counter.sol";

contract CounterTest is DSTest {
    function testIncrement() public {
        Counter counter = new Counter();
        counter.increment();
    }
}
    "#,
    );

    // Assert 100% coverage for linked libraries.
    cmd.arg("coverage").assert_success().stdout_eq(str![[r#"
...
╭-----------------+---------------+---------------+---------------+---------------╮
| File            | % Lines       | % Statements  | % Branches    | % Funcs       |
+=================================================================================+
| src/Counter.sol | 100.00% (4/4) | 100.00% (3/3) | 100.00% (0/0) | 100.00% (2/2) |
|-----------------+---------------+---------------+---------------+---------------|
| Total           | 100.00% (4/4) | 100.00% (3/3) | 100.00% (0/0) | 100.00% (2/2) |
╰-----------------+---------------+---------------+---------------+---------------╯
...
"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/10422>
// Test that line hits are properly recorded in lcov report.
forgetest!(do_while_lcov, |prj, cmd| {
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
});

// Regression test for multi-line `if` conditions: LCOV branch line numbers should be attributed to
// the condition line, not the branch body line.
//
// See: <https://github.com/foundry-rs/foundry/issues/12657>
forgetest!(multiline_if_condition_lcov_branch_line, |prj, cmd| {
    prj.insert_ds_test();
    prj.add_source(
        "IfMultiline.sol",
        r#"
contract IfMultiline {
    function f(bool a, bool b) external pure returns (uint256) {
        uint256 x = 0;
        if (
            a &&
            b
        ) {
            x = 1;
        } else {
            x = 2;
        }
        return x;
    }
}
    "#,
    );

    prj.add_source(
        "IfMultiline.t.sol",
        r#"
import "./test.sol";
import {IfMultiline} from "./IfMultiline.sol";

contract IfMultilineTest is DSTest {
    function test_true_branch() external {
        IfMultiline c = new IfMultiline();
        assertEq(c.f(true, true), 1);
    }
}
    "#,
    );

    let lcov = prj.root().join("lcov.info");
    cmd.args(["coverage", "--report=lcov", "--report-file"]).arg(&lcov).assert_success();
    assert!(lcov.exists(), "lcov.info was not created");
    let lcov_text = fs::read_to_string(&lcov).unwrap();

    let src_path = prj.paths().sources.join("IfMultiline.sol");
    let src_text = fs::read_to_string(&src_path).unwrap();
    let if_line = src_text.lines().position(|l| l.contains("if (")).unwrap() + 1;
    let body_line = src_text.lines().position(|l| l.contains("x = 1;")).unwrap() + 1;

    // We expect 2 BRDA entries (true/false) attributed to the `if (` line.
    let needle = format!("BRDA:{if_line},");
    assert!(
        lcov_text.matches(&needle).count() >= 2,
        "expected at least 2 branch entries on the condition line; lcov was:\n{lcov_text}"
    );

    // None should be attributed to the branch body line (this was the bug with multi-line
    // conditions when branch items used the body span for their line range).
    let bad_needle = format!("BRDA:{body_line},");
    assert!(
        !lcov_text.contains(&bad_needle),
        "unexpected branch entry on body line; lcov was:\n{lcov_text}"
    );
});

// Regression test based on the POC in <https://github.com/foundry-rs/foundry/issues/12508>.
//
// Goal: LCOV branch coverage should reflect short-circuit conditions for `&&` / `||` regardless of
// whether the conditions are formatted inline or across multiple lines.
//
// Related: <https://github.com/foundry-rs/foundry/issues/12657>
forgetest!(lcov_condition_branches_independent_of_formatting, |prj, cmd| {
    prj.add_source(
        "CounterMulti.sol",
        r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

contract CounterMulti {
    uint256 public number = 1;

    function setNumber(uint256 newNumber) public {
        if (
            newNumber % 2 == 0 ||
            newNumber % 2 == 1 ||
            newNumber != 0 ||
            newNumber != 1 ||
            newNumber != 2
        ) {
            number = newNumber;
        }
    }
}
    "#,
    );

    prj.add_source(
        "CounterInline.sol",
        r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

contract CounterInline {
    uint256 public number = 1;

    function setNumber(uint256 newNumber) public {
        if (newNumber % 2 == 0 || newNumber % 2 == 1 || newNumber != 0 || newNumber != 1 || newNumber != 2) {
            number = newNumber;
        }
    }
}
    "#,
    );

    prj.add_source(
        "Counter.t.sol",
        r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import {CounterMulti} from "./CounterMulti.sol";
import {CounterInline} from "./CounterInline.sol";

contract CounterTest {
    function testMulti_even_shortcircuits() external {
        CounterMulti c = new CounterMulti();
        c.setNumber(2); // first clause true => short-circuit
        require(c.number() == 2, "bad");
    }

    function testInline_even_shortcircuits() external {
        CounterInline c = new CounterInline();
        c.setNumber(2); // first clause true => short-circuit
        require(c.number() == 2, "bad");
    }
}
    "#,
    );

    let lcov = prj.root().join("lcov.info");
    cmd.args(["coverage", "--report=lcov", "--report-file"]).arg(&lcov).assert_success();
    assert!(lcov.exists(), "lcov.info was not created");
    let lcov_text = fs::read_to_string(&lcov).unwrap();

    fn section(lcov: &str, sf: &str) -> String {
        let mut in_sec = false;
        let mut out = String::new();
        for line in lcov.lines() {
            if line.starts_with("SF:") {
                in_sec = line == format!("SF:{sf}");
            }
            if in_sec {
                out.push_str(line);
                out.push('\n');
                if line == "end_of_record" {
                    break;
                }
            }
        }
        out
    }

    // N=5 operands in the `||` chain => (N-1)=4 condition branch points => 8 BR items (2 paths
    // each) plus the existing `if` true-branch item (no else) => total BRF = 9.
    let expected_brf = "BRF:9";

    for sf in ["src/CounterMulti.sol", "src/CounterInline.sol"] {
        let sec = section(&lcov_text, sf);
        assert!(!sec.is_empty(), "expected section for {sf} in lcov, got:\n{lcov_text}");
        assert!(
            sec.contains(expected_brf),
            "expected {expected_brf} in section for {sf}, got:\n{sec}"
        );
        // Must not be fully covered (only first clause short-circuits).
        let brh_line = sec.lines().find(|l| l.starts_with("BRH:")).unwrap();
        let brh: usize = brh_line["BRH:".len()..].parse().unwrap();
        assert!(
            brh < 9,
            "expected BRH < 9 (not all condition branches covered) for {sf}, got {brh_line}:\n{sec}"
        );
    }
});

// Additional regressions for <https://github.com/foundry-rs/foundry/issues/12657>:
// - `require(...)` should also report short-circuit condition branches regardless of formatting.
// - loop conditions (while) should do the same.
forgetest!(lcov_condition_branches_require_and_while, |prj, cmd| {
    prj.add_source(
        "RequireMulti.sol",
        r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

contract RequireMulti {
    function f(uint256 x) external pure returns (uint256) {
        require(
            x != 0 &&
            x != 1 &&
            x != 2 &&
            x != 3 &&
            x != 4
        , "bad");
        return x;
    }
}
    "#,
    );
    prj.add_source(
        "RequireInline.sol",
        r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

contract RequireInline {
    function f(uint256 x) external pure returns (uint256) {
        require(x != 0 && x != 1 && x != 2 && x != 3 && x != 4, "bad");
        return x;
    }
}
    "#,
    );

    prj.add_source(
        "WhileMulti.sol",
        r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

contract WhileMulti {
    function f(uint256 x) external pure returns (uint256) {
        uint256 i = 0;
        while (
            i++ < 1 &&
            x != 0 &&
            x != 1 &&
            x != 2 &&
            x != 3
        ) {
            // no-op
        }
        return i;
    }
}
    "#,
    );
    prj.add_source(
        "WhileInline.sol",
        r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

contract WhileInline {
    function f(uint256 x) external pure returns (uint256) {
        uint256 i = 0;
        while (i++ < 1 && x != 0 && x != 1 && x != 2 && x != 3) {
            // no-op
        }
        return i;
    }
}
    "#,
    );

    prj.add_source(
        "All.t.sol",
        r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import {RequireMulti} from "./RequireMulti.sol";
import {RequireInline} from "./RequireInline.sol";
import {WhileMulti} from "./WhileMulti.sol";
import {WhileInline} from "./WhileInline.sol";

contract AllTest {
    function test_require_shortcircuits() external {
        RequireMulti a = new RequireMulti();
        RequireInline b = new RequireInline();
        // x=0 fails on the first clause, so later clauses are not evaluated.
        // We expect condition-branch items to exist regardless of formatting.
        try a.f(0) { revert("expected revert"); } catch {}
        try b.f(0) { revert("expected revert"); } catch {}
    }

    function test_while_shortcircuits() external {
        WhileMulti a = new WhileMulti();
        WhileInline b = new WhileInline();
        // x=0 makes the second clause false on the first iteration, so later clauses are not evaluated.
        require(a.f(0) > 0, "bad");
        require(b.f(0) > 0, "bad");
    }
}
    "#,
    );

    let lcov = prj.root().join("lcov.info");
    cmd.args(["coverage", "--report=lcov", "--report-file"]).arg(&lcov).assert_success();
    assert!(lcov.exists(), "lcov.info was not created");
    let lcov_text = fs::read_to_string(&lcov).unwrap();

    fn section(lcov: &str, sf: &str) -> String {
        let mut in_sec = false;
        let mut out = String::new();
        for line in lcov.lines() {
            if line.starts_with("SF:") {
                in_sec = line == format!("SF:{sf}");
            }
            if in_sec {
                out.push_str(line);
                out.push('\n');
                if line == "end_of_record" {
                    break;
                }
            }
        }
        out
    }

    // Require: 5 operands in `&&` => 4 branch points => 8 condition BR items,
    // plus the existing `require` branch itself (2 paths) => total BRF = 10.
    let expected_require_brf = "BRF:10";
    for sf in ["src/RequireMulti.sol", "src/RequireInline.sol"] {
        let sec = section(&lcov_text, sf);
        assert!(
            sec.contains(expected_require_brf),
            "expected {expected_require_brf} for {sf}, got:\n{sec}"
        );
    }

    // While: 5 operands in `&&` => 4 branch points => 8 condition BR items
    let expected_while_brf = "BRF:8";
    for sf in ["src/WhileMulti.sol", "src/WhileInline.sol"] {
        let sec = section(&lcov_text, sf);
        assert!(
            sec.contains(expected_while_brf),
            "expected {expected_while_brf} for {sf}, got:\n{sec}"
        );
    }
});

// Additional regressions for loop conditions:
// - `for (...; cond; ...)` should report short-circuit condition branches regardless of formatting.
// - `do { ... } while (cond)` should do the same.
forgetest!(lcov_condition_branches_for_and_do_while, |prj, cmd| {
    prj.add_source(
        "ForMulti.sol",
        r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

contract ForMulti {
    function f(uint256 x) external pure returns (uint256) {
        uint256 i;
        for (
            i = 0;
            i < 1 &&
                x != 0 &&
                x != 1 &&
                x != 2 &&
                x != 3;
            i++
        ) {
            // no-op
        }
        return i;
    }
}
    "#,
    );
    prj.add_source(
        "ForInline.sol",
        r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

contract ForInline {
    function f(uint256 x) external pure returns (uint256) {
        uint256 i;
        for (i = 0; i < 1 && x != 0 && x != 1 && x != 2 && x != 3; i++) {
            // no-op
        }
        return i;
    }
}
    "#,
    );

    prj.add_source(
        "DoWhileMulti.sol",
        r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

contract DoWhileMulti {
    function f(uint256 x) external pure returns (uint256) {
        uint256 i = 0;
        do {
            // no-op
        } while (
            i++ < 1 &&
            x != 0 &&
            x != 1 &&
            x != 2 &&
            x != 3
        );
        return i;
    }
}
    "#,
    );
    prj.add_source(
        "DoWhileInline.sol",
        r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

contract DoWhileInline {
    function f(uint256 x) external pure returns (uint256) {
        uint256 i = 0;
        do { } while (i++ < 1 && x != 0 && x != 1 && x != 2 && x != 3);
        return i;
    }
}
    "#,
    );

    prj.add_source(
        "All.t.sol",
        r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import {ForMulti} from "./ForMulti.sol";
import {ForInline} from "./ForInline.sol";
import {DoWhileMulti} from "./DoWhileMulti.sol";
import {DoWhileInline} from "./DoWhileInline.sol";

contract AllTest {
    function test_for_shortcircuits() external {
        ForMulti a = new ForMulti();
        ForInline b = new ForInline();
        // x=0 makes the second clause false, so later clauses are not evaluated.
        require(a.f(0) == 0, "bad");
        require(b.f(0) == 0, "bad");
    }

    function test_do_while_shortcircuits() external {
        DoWhileMulti a = new DoWhileMulti();
        DoWhileInline b = new DoWhileInline();
        // x=0 makes the second clause false on the first condition check.
        require(a.f(0) > 0, "bad");
        require(b.f(0) > 0, "bad");
    }
}
    "#,
    );

    let lcov = prj.root().join("lcov.info");
    cmd.args(["coverage", "--report=lcov", "--report-file"]).arg(&lcov).assert_success();
    assert!(lcov.exists(), "lcov.info was not created");
    let lcov_text = fs::read_to_string(&lcov).unwrap();

    fn section(lcov: &str, sf: &str) -> String {
        let mut in_sec = false;
        let mut out = String::new();
        for line in lcov.lines() {
            if line.starts_with("SF:") {
                in_sec = line == format!("SF:{sf}");
            }
            if in_sec {
                out.push_str(line);
                out.push('\n');
                if line == "end_of_record" {
                    break;
                }
            }
        }
        out
    }

    // Each loop condition has 5 operands in an `&&` chain => 4 branch points => 8 BR items.
    let expected_brf = "BRF:8";
    for sf in
        ["src/ForMulti.sol", "src/ForInline.sol", "src/DoWhileMulti.sol", "src/DoWhileInline.sol"]
    {
        let sec = section(&lcov_text, sf);
        assert!(sec.contains(expected_brf), "expected {expected_brf} for {sf}, got:\n{sec}");
    }
});

// <https://github.com/foundry-rs/foundry/issues/11183>
// Test that overridden functions are disambiguated in the LCOV report.
forgetest!(disambiguate_functions, |prj, cmd| {
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
});

// Test that functions of abstract contracts and interfaces should not count in coverage report.
forgetest!(abstract_contract_and_interface, |prj, cmd| {
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
╭-----------------+---------------+---------------+---------------+---------------╮
| File            | % Lines       | % Statements  | % Branches    | % Funcs       |
+=================================================================================+
| src/Counter.sol | 100.00% (8/8) | 100.00% (4/4) | 100.00% (0/0) | 100.00% (4/4) |
|-----------------+---------------+---------------+---------------+---------------|
| Total           | 100.00% (8/8) | 100.00% (4/4) | 100.00% (0/0) | 100.00% (4/4) |
╰-----------------+---------------+---------------+---------------+---------------╯
...
"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/11548>
// Test BRDA hit values follow LCOV spec: "-" when line never executed, "0" when line hit but
// branch not taken. This ensures `genhtml` consistency.
forgetest!(brda_lcov_consistency, |prj, cmd| {
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
});

// Test that coverage files are written even when tests fail.
forgetest!(coverage_with_failing_tests, |prj, cmd| {
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
});
