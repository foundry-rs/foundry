use foundry_test_utils::{assert_data_eq, str};

forgetest!(basic_coverage, |_prj, cmd| {
    cmd.args(["coverage"]);
    cmd.assert_success();
});

forgetest!(report_file_coverage, |prj, cmd| {
    cmd.arg("coverage").args([
        "--report".to_string(),
        "lcov".to_string(),
        "--report-file".to_string(),
        prj.root().join("lcov.info").to_str().unwrap().to_string(),
    ]);
    cmd.assert_success();
});

forgetest!(test_setup_coverage, |prj, cmd| {
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
    cmd.arg("coverage").args(["--summary".to_string()]).assert_success().stdout_eq(str![[r#"
...
| File              | % Lines       | % Statements  | % Branches    | % Funcs       |
|-------------------|---------------|---------------|---------------|---------------|
| src/AContract.sol | 100.00% (2/2) | 100.00% (2/2) | 100.00% (0/0) | 100.00% (2/2) |
| Total             | 100.00% (2/2) | 100.00% (2/2) | 100.00% (0/0) | 100.00% (2/2) |

"#]]);
});

forgetest!(test_no_match_coverage, |prj, cmd| {
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
    cmd.arg("coverage")
        .args([
            "--no-match-coverage".to_string(),
            "AContract".to_string(), // Filter out `AContract`
        ])
        .assert_success()
        .stdout_eq(str![[r#"
...
| File              | % Lines       | % Statements  | % Branches    | % Funcs       |
|-------------------|---------------|---------------|---------------|---------------|
| src/BContract.sol | 100.00% (2/2) | 100.00% (2/2) | 100.00% (0/0) | 100.00% (2/2) |
| Total             | 100.00% (2/2) | 100.00% (2/2) | 100.00% (0/0) | 100.00% (2/2) |

"#]]);
});

forgetest!(test_assert_coverage, |prj, cmd| {
    prj.insert_ds_test();
    prj.add_source(
        "AContract.sol",
        r#"
contract AContract {
    function checkA() external pure returns (bool) {
        assert(10 > 2);
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
    function testA() external {
        AContract a = new AContract();
        bool result = a.checkA();
        assertTrue(result);
    }
}
    "#,
    )
    .unwrap();

    // Assert 100% coverage (assert properly covered).
    cmd.arg("coverage").args(["--summary".to_string()]).assert_success().stdout_eq(str![[r#"
...
| File              | % Lines       | % Statements  | % Branches    | % Funcs       |
|-------------------|---------------|---------------|---------------|---------------|
| src/AContract.sol | 100.00% (2/2) | 100.00% (2/2) | 100.00% (0/0) | 100.00% (1/1) |
| Total             | 100.00% (2/2) | 100.00% (2/2) | 100.00% (0/0) | 100.00% (1/1) |

"#]]);
});

forgetest!(test_require_coverage, |prj, cmd| {
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
    cmd.arg("coverage")
        .args(["--mt".to_string(), "testRequireRevert".to_string()])
        .assert_success()
        .stdout_eq(str![[r#"
...
| File              | % Lines       | % Statements  | % Branches   | % Funcs       |
|-------------------|---------------|---------------|--------------|---------------|
| src/AContract.sol | 100.00% (1/1) | 100.00% (1/1) | 50.00% (1/2) | 100.00% (1/1) |
| Total             | 100.00% (1/1) | 100.00% (1/1) | 50.00% (1/2) | 100.00% (1/1) |

"#]]);

    // Assert 100% branch coverage.
    cmd.forge_fuse().arg("coverage").args(["--summary".to_string()]).assert_success().stdout_eq(
        str![[r#"
...
| File              | % Lines       | % Statements  | % Branches    | % Funcs       |
|-------------------|---------------|---------------|---------------|---------------|
| src/AContract.sol | 100.00% (1/1) | 100.00% (1/1) | 100.00% (2/2) | 100.00% (1/1) |
| Total             | 100.00% (1/1) | 100.00% (1/1) | 100.00% (2/2) | 100.00% (1/1) |

"#]],
    );
});

forgetest!(test_line_hit_not_doubled, |prj, cmd| {
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

    let lcov_info = prj.root().join("lcov.info");
    cmd.arg("coverage").args([
        "--report".to_string(),
        "lcov".to_string(),
        "--report-file".to_string(),
        lcov_info.to_str().unwrap().to_string(),
    ]);
    cmd.assert_success();
    assert!(lcov_info.exists());

    // We want to make sure DA:8,1 is added only once so line hit is not doubled.
    assert_data_eq!(
        std::fs::read_to_string(lcov_info).unwrap(),
        str![[r#"TN:
SF:src/AContract.sol
FN:7,AContract.foo
FNDA:1,AContract.foo
DA:8,1
FNF:1
FNH:1
LF:1
LH:1
BRF:0
BRH:0
end[..]
"#]]
    );
});

forgetest!(test_branch_coverage, |prj, cmd| {
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
            if (a == 1) {
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
}

contract FooTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);
    Foo internal foo = new Foo();

    function test_issue_7784() external view {
        foo.foo(1);
        foo.foo(5);
        foo.foo(60);
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
}
    "#,
    )
    .unwrap();

    // Assert 100% coverage.
    cmd.arg("coverage").args(["--summary".to_string()]).assert_success().stdout_eq(str![[r#"
...
| File        | % Lines         | % Statements    | % Branches      | % Funcs       |
|-------------|-----------------|-----------------|-----------------|---------------|
| src/Foo.sol | 100.00% (20/20) | 100.00% (23/23) | 100.00% (12/12) | 100.00% (7/7) |
| Total       | 100.00% (20/20) | 100.00% (23/23) | 100.00% (12/12) | 100.00% (7/7) |

"#]]);
});

forgetest!(test_function_call_coverage, |prj, cmd| {
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

    // Assert 100% coverage and only 9 lines reported (comments, type conversions and struct
    // constructor calls are not included).
    cmd.arg("coverage").args(["--summary".to_string()]).assert_success().stdout_eq(str![[r#"
...
| File              | % Lines       | % Statements  | % Branches    | % Funcs       |
|-------------------|---------------|---------------|---------------|---------------|
| src/AContract.sol | 100.00% (9/9) | 100.00% (9/9) | 100.00% (0/0) | 100.00% (5/5) |
| Total             | 100.00% (9/9) | 100.00% (9/9) | 100.00% (0/0) | 100.00% (5/5) |

"#]]);
});

forgetest!(test_try_catch_coverage, |prj, cmd| {
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
    cmd.arg("coverage").args(["--mt".to_string(), "happy".to_string()]).assert_success().stdout_eq(
        str![[r#"
...
| File        | % Lines        | % Statements   | % Branches    | % Funcs       |
|-------------|----------------|----------------|---------------|---------------|
| src/Foo.sol | 66.67% (10/15) | 66.67% (14/21) | 100.00% (4/4) | 100.00% (5/5) |
| Total       | 66.67% (10/15) | 66.67% (14/21) | 100.00% (4/4) | 100.00% (5/5) |

"#]],
    );

    // Assert 100% branch coverage (including clauses without body).
    cmd.forge_fuse().arg("coverage").args(["--summary".to_string()]).assert_success().stdout_eq(
        str![[r#"
...
| File        | % Lines         | % Statements    | % Branches    | % Funcs       |
|-------------|-----------------|-----------------|---------------|---------------|
| src/Foo.sol | 100.00% (15/15) | 100.00% (21/21) | 100.00% (4/4) | 100.00% (5/5) |
| Total       | 100.00% (15/15) | 100.00% (21/21) | 100.00% (4/4) | 100.00% (5/5) |

"#]],
    );
});
