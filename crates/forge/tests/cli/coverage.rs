use foundry_common::fs;
use foundry_test_utils::{
    snapbox::{Data, IntoData},
    TestCommand, TestProject,
};
use std::path::Path;

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
    basic_base(prj, cmd);
});

forgetest_init!(basic_crlf, |prj, cmd| {
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
    )
    .unwrap();

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
    )
    .unwrap();

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
    )
    .unwrap();

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
    )
    .unwrap();

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
    )
    .unwrap();

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
    )
    .unwrap();

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
    )
    .unwrap();

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
    function testAssertBranch() external {
        AContract a = new AContract();
        bool result = a.checkA(10);
        assertTrue(result);
    }

    function testAssertRevertBranch() external {
        AContract a = new AContract();
        vm.expectRevert();
        a.checkA(1);
    }
}
    "#,
    )
    .unwrap();

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
    )
    .unwrap();

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
    function testRequireRevert() external {
        AContract a = new AContract();
        vm.expectRevert(abi.encodePacked("reverted"));
        a.checkRequire(false);
    }

    function testRequireNoRevert() external {
        AContract a = new AContract();
        a.checkRequire(true);
    }
}
    "#,
    )
    .unwrap();

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
    )
    .unwrap();

    prj.add_source(
        "AContractTest.sol",
        r#"
import "./test.sol";
import {AContract} from "./AContract.sol";

contract AContractTest is DSTest {
    function testFoo() public {
        AContract a = new AContract();
        a.foo();
    }
}
    "#,
    )
    .unwrap();

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
    )
    .unwrap();

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
    )
    .unwrap();

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
    )
    .unwrap();

    prj.add_source(
        "AContractTest.sol",
        r#"
import "./test.sol";
import {AContract} from "./AContract.sol";

contract AContractTest is DSTest {
    function testTypeConversionCoverage() external {
        AContract a = new AContract();
        a.coverMe();
    }
}
    "#,
    )
    .unwrap();

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
}
    "#,
    )
    .unwrap();

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
        bar.tryCatchExternalCall(1);
    }

    function test_coverage() external {
        Bar bar = new Bar();
        bar.tryCatchNewContract(0x0000000000000000000000000000000000000000);
        bar.tryCatchNewContract(0x0000000000000000000000000000000000000001);
        bar.tryCatchExternalCall(0);
    }
}
    "#,
    )
    .unwrap();

    // Assert coverage not 100% for happy paths only.
    cmd.arg("coverage").args(["--mt", "happy"]).assert_success().stdout_eq(str![[r#"
...
╭-------------+----------------+----------------+--------------+---------------╮
| File        | % Lines        | % Statements   | % Branches   | % Funcs       |
+==============================================================================+
| src/Foo.sol | 75.00% (15/20) | 66.67% (14/21) | 75.00% (3/4) | 100.00% (5/5) |
|-------------+----------------+----------------+--------------+---------------|
| Total       | 75.00% (15/20) | 66.67% (14/21) | 75.00% (3/4) | 100.00% (5/5) |
╰-------------+----------------+----------------+--------------+---------------╯

"#]]);

    // Assert 100% branch coverage (including clauses without body).
    cmd.forge_fuse().arg("coverage").assert_success().stdout_eq(str![[r#"
...
╭-------------+-----------------+-----------------+---------------+---------------╮
| File        | % Lines         | % Statements    | % Branches    | % Funcs       |
+=================================================================================+
| src/Foo.sol | 100.00% (20/20) | 100.00% (21/21) | 100.00% (4/4) | 100.00% (5/5) |
|-------------+-----------------+-----------------+---------------+---------------|
| Total       | 100.00% (20/20) | 100.00% (21/21) | 100.00% (4/4) | 100.00% (5/5) |
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
    )
    .unwrap();

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
    )
    .unwrap();

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
        b = (new B{value: 1})(2);
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
    )
    .unwrap();

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
    )
    .unwrap();

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

    function ifElseStatementIgnored(bool flag) external {
        if (flag) emit IsTrue(true);
        else emit IsFalse(false);

        if (flag) flag = true;
        else flag = false;
    }
}
    "#,
    )
    .unwrap();

    prj.add_source(
        "AContractTest.sol",
        r#"
import "./test.sol";
import {AContract} from "./AContract.sol";

contract AContractTest is DSTest {
    function testTrueCoverage() external {
        AContract a = new AContract();
        a.ifElseStatementIgnored(true);
    }

    function testFalseCoverage() external {
        AContract a = new AContract();
        a.ifElseStatementIgnored(false);
    }
}
    "#,
    )
    .unwrap();

    // Assert 50% coverage for true branches.
    cmd.arg("coverage").args(["--mt", "testTrueCoverage"]).assert_success().stdout_eq(str![[r#"
...
╭-------------------+--------------+--------------+--------------+---------------╮
| File              | % Lines      | % Statements | % Branches   | % Funcs       |
+================================================================================+
| src/AContract.sol | 60.00% (3/5) | 50.00% (2/4) | 50.00% (2/4) | 100.00% (1/1) |
|-------------------+--------------+--------------+--------------+---------------|
| Total             | 60.00% (3/5) | 50.00% (2/4) | 50.00% (2/4) | 100.00% (1/1) |
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
| src/AContract.sol | 60.00% (3/5) | 50.00% (2/4) | 50.00% (2/4) | 100.00% (1/1) |
|-------------------+--------------+--------------+--------------+---------------|
| Total             | 60.00% (3/5) | 50.00% (2/4) | 50.00% (2/4) | 100.00% (1/1) |
╰-------------------+--------------+--------------+--------------+---------------╯

"#]]);

    // Assert 100% coverage (true/false branches properly covered).
    cmd.forge_fuse().arg("coverage").assert_success().stdout_eq(str![[r#"
...
╭-------------------+---------------+---------------+---------------+---------------╮
| File              | % Lines       | % Statements  | % Branches    | % Funcs       |
+===================================================================================+
| src/AContract.sol | 100.00% (5/5) | 100.00% (4/4) | 100.00% (4/4) | 100.00% (1/1) |
|-------------------+---------------+---------------+---------------+---------------|
| Total             | 100.00% (5/5) | 100.00% (4/4) | 100.00% (4/4) | 100.00% (1/1) |
╰-------------------+---------------+---------------+---------------+---------------╯

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
    )
    .unwrap();

    prj.add_source(
        "AContractTest.sol",
        r#"
import "./test.sol";
import {AContract} from "./AContract.sol";

contract AContractTest is DSTest {
    function testTrueCoverage() external {
        AContract a = new AContract();
        bool[] memory isTrue = new bool[](1);
        isTrue[0] = true;
        a.execute(isTrue);
    }

    function testFalseCoverage() external {
        AContract a = new AContract();
        bool[] memory isFalse = new bool[](1);
        isFalse[0] = false;
        a.execute(isFalse);
    }
}
    "#,
    )
    .unwrap();

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
| src/AContract.sol | 60.00% (3/5) | 80.00% (4/5) | 50.00% (1/2) | 100.00% (1/1) |
|-------------------+--------------+--------------+--------------+---------------|
| Total             | 60.00% (3/5) | 80.00% (4/5) | 50.00% (1/2) | 100.00% (1/1) |
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
    )
    .unwrap();

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
    )
    .unwrap();

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
    )
    .unwrap();

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
    )
    .unwrap();

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
    )
    .unwrap();

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
    )
    .unwrap();

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
    )
    .unwrap();

    prj.add_source(
        "AContractTest.sol",
        r#"
import "./test.sol";
import "./AContract.sol";

contract AContractTest is DSTest {
    function test_constructors() public {
        AContract a = new AContract();
        address(a).call{value: 5}("");
        require(a.counter() == 5);
    }
}
    "#,
    )
    .unwrap();

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
    )
    .unwrap();

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

#[track_caller]
fn assert_lcov(cmd: &mut TestCommand, data: impl IntoData) {
    cmd.args(["--report=lcov", "--report-file"]).assert_file(data.into_data());
}
