use super::*;

forgetest!(filters, |prj, cmd| {
    prj.insert_vm();
    prj.insert_ds_test();
    prj.update_config(|config| {
        config.invariant.runs = 10;
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

    assert_invariant(cmd.args(["test", "-j1"])).failure().stdout_eq(str![[r#"
...
Ran 1 test for test/ExcludeArtifacts.t.sol:ExcludeArtifacts
[PASS] invariantShouldPass() ([RUNS])

[STATS]

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/ExcludeContracts.t.sol:ExcludeContracts
[PASS] invariantTrueWorld() ([RUNS])

[STATS]

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/ExcludeSelectors.t.sol:ExcludeSelectors
[PASS] invariantFalseWorld() ([RUNS])

[STATS]

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/ExcludeSenders.t.sol:ExcludeSenders
[PASS] invariantTrueWorld() ([RUNS])

[STATS]

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/TargetArtifactSelectors.t.sol:TargetArtifactSelectors
[PASS] invariantShouldPass() ([RUNS])

[STATS]

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/TargetArtifactSelectors2.t.sol:TargetArtifactSelectors2
[FAIL: it's false]
	[SEQUENCE]
 invariantShouldFail() ([RUNS])

[STATS]

Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 2 tests for test/TargetArtifacts.t.sol:TargetArtifacts
[FAIL: false world]
	[SEQUENCE]
 invariantShouldFail() ([RUNS])

[STATS]

[PASS] invariantShouldPass() ([RUNS])

[STATS]

Suite result: FAILED. 1 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/TargetContracts.t.sol:TargetContracts
[PASS] invariantTrueWorld() ([RUNS])

[STATS]

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/TargetInterfaces.t.sol:TargetWorldInterfaces
[FAIL: false world]
	[SEQUENCE]
 invariantTrueWorld() ([RUNS])

[STATS]

Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/TargetSelectors.t.sol:TargetSelectors
[PASS] invariantTrueWorld() ([RUNS])

[STATS]

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/TargetSenders.t.sol:TargetSenders
[FAIL: false world]
	[SEQUENCE]
 invariantTrueWorld() ([RUNS])

[STATS]

Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 11 test suites [ELAPSED]: 8 tests passed, 4 failed, 0 skipped (12 total tests)

Failing tests:
Encountered 1 failing test in test/TargetArtifactSelectors2.t.sol:TargetArtifactSelectors2
[FAIL: it's false]
	[SEQUENCE]
 invariantShouldFail() ([RUNS])

Encountered 1 failing test in test/TargetArtifacts.t.sol:TargetArtifacts
[FAIL: false world]
	[SEQUENCE]
 invariantShouldFail() ([RUNS])

Encountered 1 failing test in test/TargetInterfaces.t.sol:TargetWorldInterfaces
[FAIL: false world]
	[SEQUENCE]
 invariantTrueWorld() ([RUNS])

Encountered 1 failing test in test/TargetSenders.t.sol:TargetSenders
[FAIL: false world]
	[SEQUENCE]
 invariantTrueWorld() ([RUNS])

Encountered a total of 4 failing tests, 8 tests succeeded

Tip: Run `forge test --rerun` to retry only the 4 failed tests

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

    assert_invariant(cmd.args(["test"])).failure().stdout_eq(str![[r#"
...
Ran 1 test for test/FuzzedTargetContracts.t.sol:DynamicTargetContract
[FAIL: wrong target selector called]
	[SEQUENCE]
 invariant_dynamic_targets() ([RUNS])

[STATS]

Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/FuzzedTargetContracts.t.sol:ExplicitTargetContract
[PASS] invariant_explicit_target() ([RUNS])

[STATS]

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 2 test suites [ELAPSED]: 1 tests passed, 1 failed, 0 skipped (2 total tests)

Failing tests:
Encountered 1 failing test in test/FuzzedTargetContracts.t.sol:DynamicTargetContract
[FAIL: wrong target selector called]
	[SEQUENCE]
 invariant_dynamic_targets() ([RUNS])

Encountered a total of 1 failing tests, 1 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

"#]]);
});

forgetest!(invariant_after_invariant, |prj, cmd| {
    prj.insert_vm();
    prj.insert_ds_test();

    prj.add_test(
        "InvariantAfterInvariant.t.sol",
        r#"
import { DSTest as Test } from "src/test.sol";

struct FuzzSelector {
    address addr;
    bytes4[] selectors;
}

contract AfterInvariantHandler {
    uint256 public count;

    function inc() external {
        count += 1;
    }
}

contract InvariantAfterInvariantTest is Test {
    AfterInvariantHandler handler;

    function setUp() public {
        handler = new AfterInvariantHandler();
    }

    function targetSelectors() public returns (FuzzSelector[] memory) {
        FuzzSelector[] memory targets = new FuzzSelector[](1);
        bytes4[] memory selectors = new bytes4[](1);
        selectors[0] = handler.inc.selector;
        targets[0] = FuzzSelector(address(handler), selectors);
        return targets;
    }

    function afterInvariant() public {
        require(handler.count() < 10, "afterInvariant failure");
    }

    /// forge-config: default.invariant.runs = 1
    /// forge-config: default.invariant.depth = 11
    function invariant_after_invariant_failure() public view {
        require(handler.count() < 20, "invariant after invariant failure");
    }

    /// forge-config: default.invariant.runs = 1
    /// forge-config: default.invariant.depth = 11
    function invariant_failure() public view {
        require(handler.count() < 9, "invariant failure");
    }

    /// forge-config: default.invariant.runs = 1
    /// forge-config: default.invariant.depth = 5
    function invariant_success() public view {
        require(handler.count() < 11, "invariant should not fail");
    }
}
"#,
    );

    assert_invariant(cmd.args(["test"])).failure().stdout_eq(str![[r#"
...
Ran 3 tests for test/InvariantAfterInvariant.t.sol:InvariantAfterInvariantTest
[FAIL: afterInvariant failure]
	[SEQUENCE]
 invariant_after_invariant_failure() ([RUNS])

[STATS]

[FAIL: invariant failure]
	[SEQUENCE]
 invariant_failure() ([RUNS])

[STATS]

[PASS] invariant_success() ([RUNS])

[STATS]

Suite result: FAILED. 1 passed; 2 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 2 failed, 0 skipped (3 total tests)

Failing tests:
Encountered 2 failing tests in test/InvariantAfterInvariant.t.sol:InvariantAfterInvariantTest
[FAIL: afterInvariant failure]
	[SEQUENCE]
 invariant_after_invariant_failure() ([RUNS])
[FAIL: invariant failure]
	[SEQUENCE]
 invariant_failure() ([RUNS])

Encountered a total of 2 failing tests, 1 tests succeeded

Tip: Run `forge test --rerun` to retry only the 2 failed tests

"#]]);
});

forgetest_init!(invariant_assume, |prj, cmd| {
    prj.wipe_contracts();
    prj.update_config(|config| {
        config.invariant.runs = 1;
        config.invariant.depth = 10;
        // Should not treat vm.assume as revert.
        config.invariant.fail_on_revert = true;
    });

    prj.add_test(
        "InvariantAssume.t.sol",
        r#"
import "forge-std/Test.sol";

contract Handler is Test {
    function doSomething(uint256 param) public {
        vm.assume(param == 0);
    }
}

contract InvariantAssume is Test {
    Handler handler;

    function setUp() public {
        handler = new Handler();
    }

    function invariant_dummy() public {}
}
"#,
    );

    assert_invariant(cmd.args(["test"])).success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful with warnings:
Warning (2018): Function state mutability can be restricted to pure
 [FILE]:7:5:
  |
7 |     function doSomething(uint256 param) public {
  |     ^ (Relevant source part starts here and spans across multiple lines).


Ran 1 test for test/InvariantAssume.t.sol:InvariantAssume
[PASS] invariant_dummy() ([RUNS])

[STATS]

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);

    // Test that max_assume_rejects is respected.
    prj.update_config(|config| {
        config.invariant.max_assume_rejects = 1;
    });

    assert_invariant(&mut cmd).failure().stdout_eq(str![[r#"
No files changed, compilation skipped

Ran 1 test for test/InvariantAssume.t.sol:InvariantAssume
[FAIL: `vm.assume` rejected too many inputs (1 allowed)] invariant_dummy() ([RUNS])

[STATS]

Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 1 failed, 0 skipped (1 total tests)

Failing tests:
Encountered 1 failing test in test/InvariantAssume.t.sol:InvariantAssume
[FAIL: `vm.assume` rejected too many inputs (1 allowed)] invariant_dummy() ([RUNS])

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

"#]]);
});

// https://github.com/foundry-rs/foundry/issues/5868
forgetest!(invariant_calldata_dictionary, |prj, cmd| {
    prj.wipe_contracts();
    prj.insert_utils();
    prj.update_config(|config| {
        config.invariant.depth = 10;
    });

    prj.add_test(
        "InvariantCalldataDictionary.t.sol",
        r#"
import "./utils/Test.sol";

struct FuzzSelector {
    address addr;
    bytes4[] selectors;
}

contract Owned {
    address public owner;
    address private ownerCandidate;

    constructor() {
        owner = msg.sender;
    }

    modifier onlyOwner() {
        require(msg.sender == owner);
        _;
    }

    modifier onlyOwnerCandidate() {
        require(msg.sender == ownerCandidate);
        _;
    }

    function transferOwnership(address candidate) external onlyOwner {
        ownerCandidate = candidate;
    }

    function acceptOwnership() external onlyOwnerCandidate {
        owner = ownerCandidate;
    }
}

contract Handler is Test {
    Owned owned;

    constructor(Owned _owned) {
        owned = _owned;
    }

    function transferOwnership(address sender, address candidate) external {
        vm.assume(sender != address(0));
        vm.prank(sender);
        owned.transferOwnership(candidate);
    }

    function acceptOwnership(address sender) external {
        vm.assume(sender != address(0));
        vm.prank(sender);
        owned.acceptOwnership();
    }
}

contract InvariantCalldataDictionary is Test {
    address owner;
    Owned owned;
    Handler handler;
    address[] actors;

    function setUp() public {
        owner = address(this);
        owned = new Owned();
        handler = new Handler(owned);
        actors.push(owner);
        actors.push(address(777));
    }

    function targetSelectors() public returns (FuzzSelector[] memory) {
        FuzzSelector[] memory targets = new FuzzSelector[](1);
        bytes4[] memory selectors = new bytes4[](2);
        selectors[0] = handler.transferOwnership.selector;
        selectors[1] = handler.acceptOwnership.selector;
        targets[0] = FuzzSelector(address(handler), selectors);
        return targets;
    }

    function fixtureSender() external returns (address[] memory) {
        return actors;
    }

    function fixtureCandidate() external returns (address[] memory) {
        return actors;
    }

    function invariant_owner_never_changes() public {
        assertEq(owned.owner(), owner);
    }
}
"#,
    );

    assert_invariant(cmd.args(["test"])).failure().stdout_eq(str![[r#"
...
Ran 1 test for test/InvariantCalldataDictionary.t.sol:InvariantCalldataDictionary
[FAIL: <empty revert data>]
	[SEQUENCE]
 invariant_owner_never_changes() ([RUNS])

[STATS]

Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 1 failed, 0 skipped (1 total tests)

Failing tests:
Encountered 1 failing test in test/InvariantCalldataDictionary.t.sol:InvariantCalldataDictionary
[FAIL: <empty revert data>]
	[SEQUENCE]
 invariant_owner_never_changes() ([RUNS])

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

"#]]);
});

forgetest_init!(invariant_custom_error, |prj, cmd| {
    prj.wipe_contracts();
    prj.update_config(|config| {
        config.invariant.depth = 10;
        config.invariant.fail_on_revert = true;
    });

    prj.add_test(
        "InvariantCustomError.t.sol",
        r#"
import "forge-std/Test.sol";

contract ContractWithCustomError {
    error InvariantCustomError(uint256, string);

    function revertWithInvariantCustomError() external {
        revert InvariantCustomError(111, "custom");
    }
}

contract Handler is Test {
    ContractWithCustomError target;

    constructor() {
        target = new ContractWithCustomError();
    }

    function revertTarget() external {
        target.revertWithInvariantCustomError();
    }
}

contract InvariantCustomError is Test {
    Handler handler;

    function setUp() external {
        handler = new Handler();
    }

    function invariant_decode_error() public {}
}
"#,
    );

    assert_invariant(cmd.args(["test"])).failure().stdout_eq(str![[r#"
...
Ran 1 test for test/InvariantCustomError.t.sol:InvariantCustomError
[FAIL: InvariantCustomError(111, "custom")]
	[SEQUENCE]
 invariant_decode_error() ([RUNS])

[STATS]

Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 1 failed, 0 skipped (1 total tests)

Failing tests:
Encountered 1 failing test in test/InvariantCustomError.t.sol:InvariantCustomError
[FAIL: InvariantCustomError(111, "custom")]
	[SEQUENCE]
 invariant_decode_error() ([RUNS])

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

"#]]);
});

forgetest_init!(invariant_excluded_senders, |prj, cmd| {
    prj.wipe_contracts();
    prj.update_config(|config| {
        config.invariant.depth = 10;
        config.invariant.fail_on_revert = true;
    });

    prj.add_test(
        "InvariantExcludedSenders.t.sol",
        r#"
import "forge-std/Test.sol";

contract InvariantSenders {
    function checkSender() external {
        require(msg.sender != 0x7109709ECfa91a80626fF3989D68f67F5b1DD12D, "sender cannot be cheatcode address");
        require(msg.sender != 0x000000000000000000636F6e736F6c652e6c6f67, "sender cannot be console address");
        require(msg.sender != 0x4e59b44847b379578588920cA78FbF26c0B4956C, "sender cannot be CREATE2 deployer");
    }
}

contract InvariantExcludedSendersTest is Test {
    InvariantSenders target;

    function setUp() public {
        target = new InvariantSenders();
    }

    function invariant_check_sender() public view {}
}
"#,
    );

    assert_invariant(cmd.args(["test"])).success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful with warnings:
Warning (2018): Function state mutability can be restricted to view
 [FILE]:7:5:
  |
7 |     function checkSender() external {
  |     ^ (Relevant source part starts here and spans across multiple lines).


Ran 1 test for test/InvariantExcludedSenders.t.sol:InvariantExcludedSendersTest
[PASS] invariant_check_sender() ([RUNS])

[STATS]

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
});

forgetest_init!(invariant_fixtures, |prj, cmd| {
    prj.wipe_contracts();
    prj.update_config(|config| {
        config.invariant.runs = 1;
        config.invariant.depth = 100;
    });

    prj.add_test(
        "InvariantFixtures.t.sol",
        r#"
import "forge-std/Test.sol";

contract Target {
    bool ownerFound;
    bool amountFound;
    bool magicFound;
    bool keyFound;
    bool backupFound;
    bool extraStringFound;

    function fuzzWithFixtures(
        address owner_,
        uint256 _amount,
        int32 magic,
        bytes32 key,
        bytes memory backup,
        string memory extra
    ) external {
        if (owner_ == address(0x6B175474E89094C44Da98b954EedeAC495271d0F)) {
            ownerFound = true;
        }
        if (_amount == 1122334455) amountFound = true;
        if (magic == -777) magicFound = true;
        if (key == "abcd1234") keyFound = true;
        if (keccak256(backup) == keccak256("qwerty1234")) backupFound = true;
        if (keccak256(abi.encodePacked(extra)) == keccak256(abi.encodePacked("112233aabbccdd"))) {
            extraStringFound = true;
        }
    }

    function isCompromised() public view returns (bool) {
        return ownerFound && amountFound && magicFound && keyFound && backupFound && extraStringFound;
    }
}

/// Try to compromise target contract by finding all accepted values using fixtures.
contract InvariantFixtures is Test {
    Target target;
    address[] public fixture_owner_ = [address(0x6B175474E89094C44Da98b954EedeAC495271d0F)];
    uint256[] public fixture_amount = [1, 2, 1122334455];

    function setUp() public {
        target = new Target();
    }

    function fixtureMagic() external returns (int32[2] memory) {
        int32[2] memory magic;
        magic[0] = -777;
        magic[1] = 777;
        return magic;
    }

    function fixtureKey() external pure returns (bytes32[] memory) {
        bytes32[] memory keyFixture = new bytes32[](1);
        keyFixture[0] = "abcd1234";
        return keyFixture;
    }

    function fixtureBackup() external pure returns (bytes[] memory) {
        bytes[] memory backupFixture = new bytes[](1);
        backupFixture[0] = "qwerty1234";
        return backupFixture;
    }

    function fixtureExtra() external pure returns (string[] memory) {
        string[] memory extraFixture = new string[](1);
        extraFixture[0] = "112233aabbccdd";
        return extraFixture;
    }

    function invariant_target_not_compromised() public {
        assertEq(target.isCompromised(), false);
    }
}
"#,
    );

    assert_invariant(cmd.args(["test"])).failure().stdout_eq(str![[r#"
...
Ran 1 test for test/InvariantFixtures.t.sol:InvariantFixtures
[FAIL: assertion failed: true != false]
	[SEQUENCE]
 invariant_target_not_compromised() ([RUNS])

[STATS]

Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 1 failed, 0 skipped (1 total tests)

Failing tests:
Encountered 1 failing test in test/InvariantFixtures.t.sol:InvariantFixtures
[FAIL: assertion failed: true != false]
	[SEQUENCE]
 invariant_target_not_compromised() ([RUNS])

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

"#]]);
});

forgetest!(invariant_handler_failure, |prj, cmd| {
    prj.insert_utils();
    prj.update_config(|config| {
        config.invariant.fail_on_revert = true;
        config.invariant.runs = 1;
        config.invariant.depth = 10;
    });

    prj.add_test(
        "InvariantHandlerFailure.t.sol",
        r#"
import "./utils/Test.sol";

struct FuzzSelector {
    address addr;
    bytes4[] selectors;
}

contract Handler is Test {
    function doSomething() public {
        require(false, "failed on revert");
    }
}

contract InvariantHandlerFailure is Test {
    bytes4[] internal selectors;

    Handler handler;

    function targetSelectors() public returns (FuzzSelector[] memory) {
        FuzzSelector[] memory targets = new FuzzSelector[](1);
        bytes4[] memory selectors = new bytes4[](1);
        selectors[0] = handler.doSomething.selector;
        targets[0] = FuzzSelector(address(handler), selectors);
        return targets;
    }

    function setUp() public {
        handler = new Handler();
    }

    function statefulFuzz_BrokenInvariant() public {}
}
"#,
    );

    assert_invariant(cmd.args(["test"])).failure().stdout_eq(str![[r#"
...
Ran 1 test for test/InvariantHandlerFailure.t.sol:InvariantHandlerFailure
[FAIL: failed on revert]
	[SEQUENCE]
 statefulFuzz_BrokenInvariant() ([RUNS])

[STATS]

Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 1 failed, 0 skipped (1 total tests)

Failing tests:
Encountered 1 failing test in test/InvariantHandlerFailure.t.sol:InvariantHandlerFailure
[FAIL: failed on revert]
	[SEQUENCE]
 statefulFuzz_BrokenInvariant() ([RUNS])

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

"#]]);
});
