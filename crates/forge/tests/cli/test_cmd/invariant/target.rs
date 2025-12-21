use super::*;

forgetest!(filters, |prj, cmd| {
    prj.insert_vm();
    prj.insert_ds_test();
    prj.update_config(|config| {
        config.invariant.runs = 50;
        config.invariant.depth = 10;
    });

    prj.add_test(
        "ExcludeContracts.t.sol",
        r#"
import { DSTest as Test } from "src/test.sol";

contract Hello {
    bool public world = true;

    function change() public {
        world = false;
    }
}

contract ExcludeContracts is Test {
    Hello hello;

    function setUp() public {
        hello = new Hello();
        new Hello();
    }

    function excludeContracts() public view returns (address[] memory) {
        address[] memory addrs = new address[](1);
        addrs[0] = address(hello);
        return addrs;
    }

    function invariantTrueWorld() public {
        require(hello.world() == true, "false world");
    }
}
"#,
    );

    prj.add_test(
        "ExcludeSelectors.t.sol",
        r#"
import { DSTest as Test } from "src/test.sol";

struct FuzzSelector {
    address addr;
    bytes4[] selectors;
}

contract Hello {
    bool public world = false;

    function change() public {
        world = true;
    }

    function real_change() public {
        world = false;
    }
}

contract ExcludeSelectors is Test {
    Hello hello;

    function setUp() public {
        hello = new Hello();
    }

    function excludeSelectors() public view returns (FuzzSelector[] memory) {
        FuzzSelector[] memory targets = new FuzzSelector[](1);
        bytes4[] memory selectors = new bytes4[](1);
        selectors[0] = Hello.change.selector;
        targets[0] = FuzzSelector(address(hello), selectors);
        return targets;
    }

    function invariantFalseWorld() public {
        require(hello.world() == false, "true world");
    }
}
"#,
    );

    prj.add_test(
        "ExcludeSenders.t.sol",
        r#"
import { DSTest as Test } from "src/test.sol";

contract Hello {
    address seed_address = address(0xdeadbeef);
    bool public world = true;

    function changeBeef() public {
        require(msg.sender == address(0xdeadbeef));
        world = false;
    }

    // address(0) should be automatically excluded
    function change0() public {
        require(msg.sender == address(0));
        world = false;
    }
}

contract ExcludeSenders is Test {
    Hello hello;

    function setUp() public {
        hello = new Hello();
    }

    function excludeSenders() public view returns (address[] memory) {
        address[] memory addrs = new address[](1);
        addrs[0] = address(0xdeadbeef);
        return addrs;
    }

    // Tests clashing. Exclusion takes priority.
    function targetSenders() public view returns (address[] memory) {
        address[] memory addrs = new address[](1);
        addrs[0] = address(0xdeadbeef);
        return addrs;
    }

    function invariantTrueWorld() public {
        require(hello.world() == true, "false world");
    }
}
"#,
    );

    prj.add_test(
        "TargetContracts.t.sol",
        r#"
import { DSTest as Test } from "src/test.sol";

contract Hello {
    bool public world = true;

    function change() public {
        world = false;
    }
}

contract TargetContracts is Test {
    Hello hello1;
    Hello hello2;

    function setUp() public {
        hello1 = new Hello();
        hello2 = new Hello();
    }

    function targetContracts() public view returns (address[] memory) {
        address[] memory addrs = new address[](1);
        addrs[0] = address(hello1);
        return addrs;
    }

    function invariantTrueWorld() public {
        require(hello2.world() == true, "false world");
    }
}
"#,
    );

    prj.add_test(
        "TargetInterfaces.t.sol",
        r#"
import { DSTest as Test } from "src/test.sol";

struct FuzzInterface {
    address target;
    string[] artifacts;
}

contract Hello {
    bool public world;

    function changeWorld() external {
        world = true;
    }
}

interface IHello {
    function world() external view returns (bool);
    function changeWorld() external;
}

contract HelloProxy {
    address internal immutable _implementation;

    constructor(address implementation_) {
        _implementation = implementation_;
    }

    function _delegate(address implementation) internal {
        assembly {
            calldatacopy(0, 0, calldatasize())

            let result := delegatecall(gas(), implementation, 0, calldatasize(), 0, 0)

            returndatacopy(0, 0, returndatasize())

            switch result
            case 0 { revert(0, returndatasize()) }
            default { return(0, returndatasize()) }
        }
    }

    fallback() external payable {
        _delegate(_implementation);
    }
}

contract TargetWorldInterfaces is Test {
    IHello proxy;

    function setUp() public {
        Hello hello = new Hello();
        proxy = IHello(address(new HelloProxy(address(hello))));
    }

    function targetInterfaces() public view returns (FuzzInterface[] memory) {
        FuzzInterface[] memory targets = new FuzzInterface[](1);

        string[] memory artifacts = new string[](1);
        artifacts[0] = "IHello";

        targets[0] = FuzzInterface(address(proxy), artifacts);

        return targets;
    }

    function invariantTrueWorld() public {
        require(proxy.world() == false, "false world");
    }
}
"#,
    );

    prj.add_test(
        "TargetSelectors.t.sol",
        r#"
import { DSTest as Test } from "src/test.sol";

struct FuzzSelector {
    address addr;
    bytes4[] selectors;
}

contract Hello {
    bool public world = true;

    function change() public {
        world = true;
    }

    function real_change() public {
        world = false;
    }
}

contract TargetSelectors is Test {
    Hello hello;

    function setUp() public {
        hello = new Hello();
    }

    function targetSelectors() public view returns (FuzzSelector[] memory) {
        FuzzSelector[] memory targets = new FuzzSelector[](1);
        bytes4[] memory selectors = new bytes4[](1);
        selectors[0] = Hello.change.selector;
        targets[0] = FuzzSelector(address(hello), selectors);
        return targets;
    }

    function invariantTrueWorld() public {
        require(hello.world() == true, "false world");
    }
}
"#,
    );

    prj.add_test(
        "TargetSenders.t.sol",
        r#"
import { DSTest as Test } from "src/test.sol";

contract Hello {
    bool public world = true;

    function change() public {
        require(msg.sender == address(0xdeadbeef));
        world = false;
    }
}

contract TargetSenders is Test {
    Hello hello;

    function setUp() public {
        hello = new Hello();
    }

    function targetSenders() public view returns (address[] memory) {
        address[] memory addrs = new address[](1);
        addrs[0] = address(0xdeadbeef);
        return addrs;
    }

    function invariantTrueWorld() public {
        require(hello.world() == true, "false world");
    }
}
"#,
    );

    prj.add_test(
        "ExcludeArtifacts.t.sol",
        r#"
import { DSTest as Test } from "src/test.sol";

// Will get automatically excluded. Otherwise it would throw error.
contract NoMutFunctions {
    function no_change() public pure {}
}

contract Excluded {
    bool public world = true;

    function change() public {
        world = false;
    }
}

contract Hello {
    bool public world = true;

    function change() public {
        world = false;
    }
}

contract ExcludeArtifacts is Test {
    Excluded excluded;

    function setUp() public {
        excluded = new Excluded();
        new Hello();
        new NoMutFunctions();
    }

    function excludeArtifacts() public returns (string[] memory) {
        string[] memory abis = new string[](1);
        abis[0] = "test/ExcludeArtifacts.t.sol:Excluded";
        return abis;
    }

    function invariantShouldPass() public {
        require(excluded.world() == true, "false world");
    }
}
"#,
    );

    prj.add_test(
        "TargetArtifactSelectors2.t.sol",
        r#"
import { DSTest as Test } from "src/test.sol";

struct FuzzArtifactSelector {
    string artifact;
    bytes4[] selectors;
}

contract Parent {
    bool public should_be_true = true;
    address public child;

    function change() public {
        child = msg.sender;
        should_be_true = false;
    }

    function create() public {
        new Child();
    }
}

contract Child {
    Parent parent;
    bool public changed = false;

    constructor() {
        parent = Parent(msg.sender);
    }

    function change_parent() public {
        parent.change();
    }

    function tracked_change_parent() public {
        parent.change();
    }
}

contract TargetArtifactSelectors2 is Test {
    Parent parent;

    function setUp() public {
        parent = new Parent();
    }

    function targetArtifactSelectors() public returns (FuzzArtifactSelector[] memory) {
        FuzzArtifactSelector[] memory targets = new FuzzArtifactSelector[](2);
        bytes4[] memory selectors_child = new bytes4[](1);

        selectors_child[0] = Child.change_parent.selector;
        targets[0] = FuzzArtifactSelector(
            "test/TargetArtifactSelectors2.t.sol:Child", selectors_child
        );

        bytes4[] memory selectors_parent = new bytes4[](1);
        selectors_parent[0] = Parent.create.selector;
        targets[1] = FuzzArtifactSelector(
            "test/TargetArtifactSelectors2.t.sol:Parent", selectors_parent
        );
        return targets;
    }

    function invariantShouldFail() public {
        if (!parent.should_be_true()) {
            require(!Child(address(parent.child())).changed(), "should have not happened");
        }
        require(parent.should_be_true() == true, "it's false");
    }
}
"#,
    );

    prj.add_test(
        "TargetArtifactSelectors.t.sol",
        r#"
import { DSTest as Test } from "src/test.sol";

struct FuzzArtifactSelector {
    string artifact;
    bytes4[] selectors;
}

contract Hi {
    bool public world = true;

    function no_change() public {
        world = true;
    }

    function change() public {
        world = false;
    }
}

contract TargetArtifactSelectors is Test {
    Hi hello;

    function setUp() public {
        hello = new Hi();
    }

    function targetArtifactSelectors() public returns (FuzzArtifactSelector[] memory) {
        FuzzArtifactSelector[] memory targets = new FuzzArtifactSelector[](1);
        bytes4[] memory selectors = new bytes4[](1);
        selectors[0] = Hi.no_change.selector;
        targets[0] =
            FuzzArtifactSelector("test/TargetArtifactSelectors.t.sol:Hi", selectors);
        return targets;
    }

    function invariantShouldPass() public {
        require(hello.world() == true, "false world");
    }
}
"#,
    );

    prj.add_test(
        "TargetArtifacts.t.sol",
        r#"
import { DSTest as Test } from "src/test.sol";

contract Targeted {
    bool public world = true;

    function change() public {
        world = false;
    }
}

contract Hello {
    bool public world = true;

    function no_change() public {}
}

contract TargetArtifacts is Test {
    Targeted target1;
    Targeted target2;
    Hello hello;

    function setUp() public {
        target1 = new Targeted();
        target2 = new Targeted();
        hello = new Hello();
    }

    function targetArtifacts() public returns (string[] memory) {
        string[] memory abis = new string[](1);
        abis[0] = "test/TargetArtifacts.t.sol:Targeted";
        return abis;
    }

    function invariantShouldPass() public {
        require(target2.world() == true || target1.world() == true || hello.world() == true, "false world");
    }

    function invariantShouldFail() public {
        require(target2.world() == true || target1.world() == true, "false world");
    }
}
"#,
    );

    // Test ExcludeContracts
    assert_invariant(cmd.forge_fuse().args(["test", "--mc", "ExcludeContracts"]))
        .success()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/ExcludeContracts.t.sol:ExcludeContracts
[PASS] invariantTrueWorld() ([RUNS])

[STATS]

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);

    // Test ExcludeSelectors
    assert_invariant(cmd.forge_fuse().args(["test", "--mc", "ExcludeSelectors"]))
        .success()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/ExcludeSelectors.t.sol:ExcludeSelectors
[PASS] invariantFalseWorld() ([RUNS])

[STATS]

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);

    // Test ExcludeSenders
    assert_invariant(cmd.forge_fuse().args(["test", "--mc", "ExcludeSenders"]))
        .success()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/ExcludeSenders.t.sol:ExcludeSenders
[PASS] invariantTrueWorld() ([RUNS])

[STATS]

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);

    // Test TargetContracts
    assert_invariant(cmd.forge_fuse().args(["test", "--mc", "TargetContracts"]))
        .success()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/TargetContracts.t.sol:TargetContracts
[PASS] invariantTrueWorld() ([RUNS])

[STATS]

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);

    // Test TargetInterfaces (should fail)
    assert_invariant(cmd.forge_fuse().args(["test", "--mc", "TargetWorldInterfaces"]))
        .failure()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/TargetInterfaces.t.sol:TargetWorldInterfaces
[FAIL: false world]
	[SEQUENCE]
 invariantTrueWorld() ([RUNS])

[STATS]

Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 1 failed, 0 skipped (1 total tests)

Failing tests:
Encountered 1 failing test in test/TargetInterfaces.t.sol:TargetWorldInterfaces
[FAIL: false world]
	[SEQUENCE]
 invariantTrueWorld() ([RUNS])

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

"#]]);

    // Test TargetSelectors
    assert_invariant(cmd.forge_fuse().args(["test", "--mc", "TargetSelectors"]))
        .success()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/TargetSelectors.t.sol:TargetSelectors
[PASS] invariantTrueWorld() ([RUNS])

[STATS]

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);

    // Test TargetSenders (should fail)
    assert_invariant(cmd.forge_fuse().args(["test", "--mc", "TargetSenders"])).failure().stdout_eq(
        str![[r#"
...
Ran 1 test for test/TargetSenders.t.sol:TargetSenders
[FAIL: false world]
	[SEQUENCE]
 invariantTrueWorld() ([RUNS])

[STATS]

Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 1 failed, 0 skipped (1 total tests)

Failing tests:
Encountered 1 failing test in test/TargetSenders.t.sol:TargetSenders
[FAIL: false world]
	[SEQUENCE]
 invariantTrueWorld() ([RUNS])

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

"#]],
    );

    // Test ExcludeArtifacts
    assert_invariant(cmd.forge_fuse().args(["test", "--mc", "ExcludeArtifacts"]))
        .success()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/ExcludeArtifacts.t.sol:ExcludeArtifacts
[PASS] invariantShouldPass() ([RUNS])

[STATS]

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);

    // Test TargetArtifactSelectors2 (should fail)
    assert_invariant(cmd.forge_fuse().args(["test", "--mc", "TargetArtifactSelectors2"]))
        .failure()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/TargetArtifactSelectors2.t.sol:TargetArtifactSelectors2
[FAIL: it's false]
	[SEQUENCE]
 invariantShouldFail() ([RUNS])

[STATS]

Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 1 failed, 0 skipped (1 total tests)

Failing tests:
Encountered 1 failing test in test/TargetArtifactSelectors2.t.sol:TargetArtifactSelectors2
[FAIL: it's false]
	[SEQUENCE]
 invariantShouldFail() ([RUNS])

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

"#]]);

    // Test TargetArtifactSelectors
    assert_invariant(cmd.forge_fuse().args(["test", "--mc", "^TargetArtifactSelectors$"]))
        .success()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/TargetArtifactSelectors.t.sol:TargetArtifactSelectors
[PASS] invariantShouldPass() ([RUNS])

[STATS]

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);

    // Test TargetArtifacts
    assert_invariant(cmd.forge_fuse().args(["test", "--mc", "^TargetArtifacts$"]))
        .failure()
        .stdout_eq(str![[r#"
...
Ran 2 tests for test/TargetArtifacts.t.sol:TargetArtifacts
[FAIL: false world]
	[SEQUENCE]
 invariantShouldFail() ([RUNS])

[STATS]

[PASS] invariantShouldPass() ([RUNS])

[STATS]

Suite result: FAILED. 1 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 1 failed, 0 skipped (2 total tests)

Failing tests:
Encountered 1 failing test in test/TargetArtifacts.t.sol:TargetArtifacts
[FAIL: false world]
	[SEQUENCE]
 invariantShouldFail() ([RUNS])

Encountered a total of 1 failing tests, 1 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

"#]]);
});

// https://github.com/foundry-rs/foundry/issues/5625
// https://github.com/foundry-rs/foundry/issues/6166
// `Target.wrongSelector` is not called when handler added as `targetContract`
// `Target.wrongSelector` is called (and test fails) when no `targetContract` set
forgetest!(fuzzed_selected_targets, |prj, cmd| {
    prj.insert_vm();
    prj.insert_ds_test();
    prj.update_config(|config| {
        config.invariant.depth = 10;
        config.invariant.fail_on_revert = true;
    });

    prj.add_test(
        "FuzzedTargetContracts.t.sol",
        r#"
import { DSTest as Test } from "src/test.sol";
import "src/Vm.sol";

contract Target {
    uint256 count;

    function wrongSelector() external {
        revert("wrong target selector called");
    }

    function goodSelector() external {
        count++;
    }
}

contract Handler is Test {
    function increment() public {
        Target(0x6B175474E89094C44Da98b954EedeAC495271d0F).goodSelector();
    }
}

contract ExplicitTargetContract is Test {
    Vm constant vm = Vm(HEVM_ADDRESS);
    Handler handler;

    function setUp() public {
        Target target = new Target();
        bytes memory targetCode = address(target).code;
        vm.etch(address(0x6B175474E89094C44Da98b954EedeAC495271d0F), targetCode);

        handler = new Handler();
    }

    function targetContracts() public view returns (address[] memory) {
        address[] memory addrs = new address[](1);
        addrs[0] = address(handler);
        return addrs;
    }

    function invariant_explicit_target() public {}
}

contract DynamicTargetContract is Test {
    Vm constant vm = Vm(HEVM_ADDRESS);
    Handler handler;

    function setUp() public {
        Target target = new Target();
        bytes memory targetCode = address(target).code;
        vm.etch(address(0x6B175474E89094C44Da98b954EedeAC495271d0F), targetCode);

        handler = new Handler();
    }

    function invariant_dynamic_targets() public {}
}
"#,
    );

    assert_invariant(cmd.args(["test", "-j1"])).failure().stdout_eq(str![[r#"
...
[PASS] invariant_explicit_target() ([RUNS])
...
[FAIL: wrong target selector called]
	[SEQUENCE]
 invariant_dynamic_targets() ([RUNS])
...

"#]]);
});
