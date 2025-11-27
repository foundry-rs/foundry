use super::*;

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
    prj.insert_utils();
    prj.update_config(|config| {
        config.fuzz.seed = Some(U256::from(1));
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

...

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
    prj.update_config(|config| {
        config.invariant.runs = 1;
        config.invariant.depth = 100;
        // disable literals to test fixtures
        config.invariant.dictionary.max_fuzz_dictionary_literals = 0;
        config.fuzz.dictionary.max_fuzz_dictionary_literals = 0;
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

forgetest_init!(invariant_breaks_without_fixtures, |prj, cmd| {
    prj.update_config(|config| {
        config.fuzz.seed = Some(U256::from(1));
        config.invariant.runs = 1;
        config.invariant.depth = 100;
    });

    prj.add_test(
        "InvariantLiterals.t.sol",
        r#"
import "forge-std/Test.sol";

contract Target {
    bool ownerFound;
    bool amountFound;
    bool magicFound;
    bool keyFound;
    bool backupFound;
    bool extraStringFound;

    function fuzzWithoutFixtures(
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

/// Try to compromise target contract by finding all accepted values without using fixtures.
contract InvariantLiterals is Test {
    Target target;

    function setUp() public {
        target = new Target();
    }

    function invariant_target_not_compromised() public {
        assertEq(target.isCompromised(), false);
    }
}
"#,
    );

    assert_invariant(cmd.args(["test"])).failure().stdout_eq(str![[r#"
...
Ran 1 test for test/InvariantLiterals.t.sol:InvariantLiterals
[FAIL: assertion failed: true != false]
	[SEQUENCE]
 invariant_target_not_compromised() ([RUNS])

[STATS]

Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 1 failed, 0 skipped (1 total tests)

Failing tests:
Encountered 1 failing test in test/InvariantLiterals.t.sol:InvariantLiterals
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

// Here we test that the fuzz engine can include a contract created during the fuzz
// in its fuzz dictionary and eventually break the invariant.
// Specifically, can Judas, a created contract from Jesus, break Jesus contract
// by revealing his identity.
forgetest_init!(
    #[cfg_attr(windows, ignore = "for some reason there's different rng")]
    invariant_inner_contract,
    |prj, cmd| {
        prj.update_config(|config| {
            config.invariant.depth = 10;
        });

        prj.add_test(
            "InvariantInnerContract.t.sol",
            r#"
import "forge-std/Test.sol";

contract Jesus {
    address fren;
    bool public identity_revealed;

    function create_fren() public {
        fren = address(new Judas());
    }

    function kiss() public {
        require(msg.sender == fren);
        identity_revealed = true;
    }
}

contract Judas {
    Jesus jesus;

    constructor() {
        jesus = Jesus(msg.sender);
    }

    function betray() public {
        jesus.kiss();
    }
}

contract InvariantInnerContract is Test {
    Jesus jesus;

    function setUp() public {
        jesus = new Jesus();
    }

    function invariantHideJesus() public {
        require(jesus.identity_revealed() == false, "jesus betrayed");
    }
}
"#,
        );

        assert_invariant(cmd.args(["test"])).failure().stdout_eq(str![[r#"
...
Ran 1 test for test/InvariantInnerContract.t.sol:InvariantInnerContract
[FAIL: jesus betrayed]
	[SEQUENCE]
 invariantHideJesus() ([RUNS])

[STATS]

Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 1 failed, 0 skipped (1 total tests)

Failing tests:
Encountered 1 failing test in test/InvariantInnerContract.t.sol:InvariantInnerContract
[FAIL: jesus betrayed]
	[SEQUENCE]
 invariantHideJesus() ([RUNS])

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

"#]]);

        // `fuzz_seed` at 119 makes this sequence shrinkable from 4 to 2.
        prj.update_config(|config| {
            config.fuzz.seed = Some(U256::from(119u32));
            // Disable persisted failures for rerunning the test.
            config.invariant.failure_persist_dir = Some(
                config
                    .invariant
                    .failure_persist_dir
                    .as_ref()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .join("persistence2"),
            );
        });
        cmd.assert_failure().stdout_eq(str![[r#"
No files changed, compilation skipped

Ran 1 test for test/InvariantInnerContract.t.sol:InvariantInnerContract
[FAIL: jesus betrayed]
	[Sequence] (original: 2, shrunk: 2)
		sender=[..] addr=[test/InvariantInnerContract.t.sol:Jesus][..] calldata=create_fren() args=[]
		sender=[..] addr=[test/InvariantInnerContract.t.sol:Judas][..] calldata=betray() args=[]
 invariantHideJesus() (runs: 0, calls: 0, reverts: 1)
...
"#]]);
    }
);

// https://github.com/foundry-rs/foundry/issues/7219
forgetest!(invariant_preserve_state, |prj, cmd| {
    prj.insert_utils();
    prj.update_config(|config| {
        config.invariant.depth = 10;
        config.invariant.fail_on_revert = true;
    });

    prj.add_test(
        "InvariantPreserveState.t.sol",
        r#"
import "./utils/Test.sol";

struct FuzzSelector {
    address addr;
    bytes4[] selectors;
}

contract Handler is Test {
    function thisFunctionReverts() external {
        if (block.number < 10) {} else {
            revert();
        }
    }

    function advanceTime(uint256 blocks) external {
        blocks = blocks % 10;
        vm.roll(block.number + blocks);
        vm.warp(block.timestamp + blocks * 12);
    }
}

contract InvariantPreserveState is Test {
    Handler handler;

    function setUp() public {
        handler = new Handler();
    }

    function targetSelectors() public returns (FuzzSelector[] memory) {
        FuzzSelector[] memory targets = new FuzzSelector[](1);
        bytes4[] memory selectors = new bytes4[](2);
        selectors[0] = handler.thisFunctionReverts.selector;
        selectors[1] = handler.advanceTime.selector;
        targets[0] = FuzzSelector(address(handler), selectors);
        return targets;
    }

    function invariant_preserve_state() public {
        assertTrue(true);
    }
}
"#,
    );

    assert_invariant(cmd.args(["test"])).failure().stdout_eq(str![[r#"
...
Ran 1 test for test/InvariantPreserveState.t.sol:InvariantPreserveState
[FAIL: EvmError: Revert]
	[SEQUENCE]
 invariant_preserve_state() ([RUNS])

[STATS]

Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 1 failed, 0 skipped (1 total tests)

Failing tests:
Encountered 1 failing test in test/InvariantPreserveState.t.sol:InvariantPreserveState
[FAIL: EvmError: Revert]
	[SEQUENCE]
 invariant_preserve_state() ([RUNS])

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

"#]]);
});

// add code so contract is accounted as valid sender
// see https://github.com/foundry-rs/foundry/issues/4245
forgetest!(invariant_reentrancy, |prj, cmd| {
    prj.insert_utils();
    prj.update_config(|config| {
        config.invariant.depth = 10;
        config.invariant.fail_on_revert = false;
        config.invariant.call_override = true;
    });

    prj.add_test(
        "InvariantReentrancy.t.sol",
        r#"
import "./utils/Test.sol";

contract Malicious {
    function world() public {
        payable(msg.sender).call("");
    }
}

contract Vulnerable {
    bool public open_door = false;
    bool public stolen = false;
    Malicious mal;

    constructor(address _mal) {
        mal = Malicious(_mal);
    }

    function hello() public {
        open_door = true;
        mal.world();
        open_door = false;
    }

    function backdoor() public {
        require(open_door, "");
        stolen = true;
    }
}

contract InvariantReentrancy is Test {
    Vulnerable vuln;
    Malicious mal;

    function setUp() public {
        mal = new Malicious();
        vuln = new Vulnerable(address(mal));
    }

    // do not include `mal` in identified contracts
    // see https://github.com/foundry-rs/foundry/issues/4245
    function targetContracts() public view returns (address[] memory) {
        address[] memory targets = new address[](1);
        targets[0] = address(vuln);
        return targets;
    }

    function invariantNotStolen() public {
        require(vuln.stolen() == false, "stolen");
    }
}
"#,
    );

    assert_invariant(cmd.args(["test"])).failure().stdout_eq(str![[r#"
...
Ran 1 test for test/InvariantReentrancy.t.sol:InvariantReentrancy
[FAIL: stolen]
	[SEQUENCE]
 invariantNotStolen() ([RUNS])

[STATS]

Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 1 failed, 0 skipped (1 total tests)

Failing tests:
Encountered 1 failing test in test/InvariantReentrancy.t.sol:InvariantReentrancy
[FAIL: stolen]
	[SEQUENCE]
 invariantNotStolen() ([RUNS])

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

"#]]);
});

forgetest_init!(invariant_roll_fork, |prj, cmd| {
    prj.add_rpc_endpoints();
    prj.update_config(|config| {
        config.fuzz.seed = Some(U256::from(119u32));
        config.invariant.shrink_run_limit = 0;
    });

    prj.add_test(
        "InvariantRollFork.t.sol",
        r#"
import "forge-std/Test.sol";

interface IERC20 {
    function totalSupply() external view returns (uint256 supply);
}

contract RollForkHandler is Test {
    uint256 public totalSupply;

    function work() external {
        vm.rollFork(block.number + 1);
        totalSupply = IERC20(0x6B175474E89094C44Da98b954EedeAC495271d0F).totalSupply();
    }
}

contract InvariantRollForkBlockTest is Test {
    RollForkHandler forkHandler;

    function setUp() public {
        vm.createSelectFork("mainnet", 19812632);
        forkHandler = new RollForkHandler();
    }

    /// forge-config: default.invariant.runs = 2
    /// forge-config: default.invariant.depth = 4
    function invariant_fork_handler_block() public {
        require(block.number < 19812634, "too many blocks mined");
    }
}

contract InvariantRollForkStateTest is Test {
    RollForkHandler forkHandler;

    function setUp() public {
        vm.createSelectFork("mainnet", 19812632);
        forkHandler = new RollForkHandler();
    }

    /// forge-config: default.invariant.runs = 1
    function invariant_fork_handler_state() public {
        require(forkHandler.totalSupply() < 3254378807384273078310283461, "wrong supply");
    }
}
"#,
    );

    assert_invariant(cmd.args(["test", "-j1"])).failure().stdout_eq(str![[r#"
...
Ran 2 test suites [ELAPSED]: 0 tests passed, 2 failed, 0 skipped (2 total tests)

Failing tests:
Encountered 1 failing test in test/InvariantRollFork.t.sol:InvariantRollForkBlockTest
[FAIL: too many blocks mined]
...
 invariant_fork_handler_block() ([RUNS])

Encountered 1 failing test in test/InvariantRollFork.t.sol:InvariantRollForkStateTest
[FAIL: wrong supply]
...
 invariant_fork_handler_state() ([RUNS])

Encountered a total of 2 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 2 failed tests

"#]]);
});

forgetest_init!(invariant_scrape_values, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.depth = 10;
        config.fuzz.seed = Some(U256::from(100u32));
    });

    prj.add_test(
        "InvariantScrapeValues.t.sol",
        r#"
import "forge-std/Test.sol";

contract FindFromReturnValue {
    bool public found = false;

    function seed() public returns (int256) {
        int256 mystery = 13337;
        return (1337 + mystery);
    }

    function find(int256 i) public {
        int256 mystery = 13337;
        if (i == 1337 + mystery) {
            found = true;
        }
    }
}

contract FindFromReturnValueTest is Test {
    FindFromReturnValue target;

    function setUp() public {
        target = new FindFromReturnValue();
    }

    /// forge-config: default.invariant.runs = 50
    /// forge-config: default.invariant.depth = 300
    /// forge-config: default.invariant.fail-on-revert = true
    function invariant_value_not_found() public view {
        require(!target.found(), "value from return found");
    }
}

contract FindFromLogValue {
    event FindFromLog(int256 indexed mystery, bytes32 rand);

    bool public found = false;

    function seed() public {
        int256 mystery = 13337;
        emit FindFromLog(1337 + mystery, keccak256(abi.encodePacked("mystery")));
    }

    function find(int256 i) public {
        int256 mystery = 13337;
        if (i == 1337 + mystery) {
            found = true;
        }
    }
}

contract FindFromLogValueTest is Test {
    FindFromLogValue target;

    function setUp() public {
        target = new FindFromLogValue();
    }

    /// forge-config: default.invariant.runs = 50
    /// forge-config: default.invariant.depth = 300
    /// forge-config: default.invariant.fail-on-revert = true
    function invariant_value_not_found() public view {
        require(!target.found(), "value from logs found");
    }
}
"#,
    );

    assert_invariant(cmd.args(["test", "-j1"])).failure().stdout_eq(str![[r#"
...
Ran 2 test suites [ELAPSED]: 0 tests passed, 2 failed, 0 skipped (2 total tests)

Failing tests:
Encountered 1 failing test in test/InvariantScrapeValues.t.sol:FindFromLogValueTest
[FAIL: value from logs found]
	[SEQUENCE]
 invariant_value_not_found() ([RUNS])

Encountered 1 failing test in test/InvariantScrapeValues.t.sol:FindFromReturnValueTest
[FAIL: value from return found]
	[SEQUENCE]
 invariant_value_not_found() ([RUNS])

Encountered a total of 2 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 2 failed tests

"#]]);
});

forgetest_init!(invariant_sequence_no_reverts, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.depth = 15;
        config.invariant.fail_on_revert = false;
        // Use original counterexample to test sequence len.
        config.invariant.shrink_run_limit = 0;
    });

    prj.add_test(
        "InvariantSequenceNoReverts.t.sol",
        r#"
import "forge-std/Test.sol";

contract SequenceNoReverts {
    uint256 public count;

    function work(uint256 x) public {
        require(x % 2 != 0);
        count++;
    }
}

contract SequenceNoRevertsTest is Test {
    SequenceNoReverts target;

    function setUp() public {
        target = new SequenceNoReverts();
    }

    function invariant_no_reverts() public view {
        require(target.count() < 10, "condition met");
    }
}
"#,
    );

    // ensure original counterexample len is 10 (even without shrinking)
    cmd.args(["test"]).assert_failure().stdout_eq(str![[r#"
...
Ran 1 test for test/InvariantSequenceNoReverts.t.sol:SequenceNoRevertsTest
[FAIL: condition met]
	[Sequence] (original: 10, shrunk: 10)
...
 invariant_no_reverts() ([..])
...
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 1 failed, 0 skipped (1 total tests)
...
"#]]);
});

forgetest_init!(
    #[cfg_attr(windows, ignore = "for some reason there's different rng")]
    invariant_shrink_big_sequence,
    |prj, cmd| {
        prj.update_config(|config| {
            config.fuzz.seed = Some(U256::from(119u32));
            config.invariant.runs = 1;
            config.invariant.depth = 1000;
            config.invariant.shrink_run_limit = 425;
        });

        prj.add_test(
            "InvariantShrinkBigSequence.t.sol",
            r#"
import "forge-std/Test.sol";

contract ShrinkBigSequence {
    uint256 cond;

    function work(uint256 x) public {
        if (x % 2 != 0 && x < 9000) {
            cond++;
        }
    }

    function checkCond() public view {
        require(cond < 77, "condition met");
    }
}

contract ShrinkBigSequenceTest is Test {
    ShrinkBigSequence target;

    function setUp() public {
        target = new ShrinkBigSequence();
    }

    function invariant_shrink_big_sequence() public view {
        target.checkCond();
    }
}
"#,
        );

        // ensure shrinks to same sequence of 77
        cmd.args(["test"]).assert_failure().stdout_eq(str![[r#"
...
Ran 1 test for test/InvariantShrinkBigSequence.t.sol:ShrinkBigSequenceTest
[FAIL: condition met]
	[Sequence] (original: [..], shrunk: 77)
...
"#]]);
        cmd.assert_failure().stdout_eq(str![[r#"
...
Ran 1 test for test/InvariantShrinkBigSequence.t.sol:ShrinkBigSequenceTest
[FAIL: invariant_shrink_big_sequence replay failure]
	[Sequence] (original: [..], shrunk: 77)
...
"#]]);
    }
);

forgetest_init!(invariant_shrink_fail_on_revert, |prj, cmd| {
    prj.update_config(|config| {
        config.fuzz.seed = Some(U256::from(119u32));
        config.invariant.fail_on_revert = true;
        config.invariant.runs = 1;
        config.invariant.depth = 200;
    });

    prj.add_test(
        "InvariantShrinkFailOnRevert.t.sol",
        r#"
import "forge-std/Test.sol";

contract ShrinkFailOnRevert {
    uint256 cond;

    function work(uint256 x) public {
        if (x % 2 != 0 && x < 9000) {
            cond++;
        }
        require(cond < 10, "condition met");
    }
}

contract ShrinkFailOnRevertTest is Test {
    ShrinkFailOnRevert target;

    function setUp() public {
        target = new ShrinkFailOnRevert();
    }

    function invariant_shrink_fail_on_revert() public view {}
}
"#,
    );

    cmd.args(["test"]).assert_failure().stdout_eq(str![[r#"
...
Ran 1 test for test/InvariantShrinkFailOnRevert.t.sol:ShrinkFailOnRevertTest
[FAIL: condition met]
	[Sequence] (original: [..], shrunk: 10)
...
"#]]);
});

forgetest_init!(invariant_shrink_with_assert, |prj, cmd| {
    prj.update_config(|config| {
        config.fuzz.seed = Some(U256::from(100u32));
        config.invariant.runs = 1;
        config.invariant.depth = 15;
    });

    prj.add_test(
        "InvariantShrinkWithAssert.t.sol",
        r#"
import "forge-std/Test.sol";

contract Counter {
    uint256 public number;

    function increment() public {
        number++;
    }

    function decrement() public {
        number--;
    }
}

contract InvariantShrinkWithAssert is Test {
    Counter public counter;

    function setUp() public {
        counter = new Counter();
    }

    function invariant_with_assert() public {
        assertTrue(counter.number() < 2, "wrong counter assert");
    }

    function invariant_with_require() public {
        require(counter.number() < 2, "wrong counter require");
    }
}
"#,
    );

    cmd.args(["test"]).assert_failure().stdout_eq(str![[r#"
...
Ran 2 tests for test/InvariantShrinkWithAssert.t.sol:InvariantShrinkWithAssert
[FAIL: wrong counter assert]
	[Sequence] (original: 2, shrunk: 2)
...
 invariant_with_assert() ([..])
...
[FAIL: wrong counter require]
	[Sequence] (original: 2, shrunk: 2)
...
 invariant_with_require() ([..])
...
"#]]);
});

forgetest_init!(invariant_test1, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.depth = 10;
    });

    prj.add_test(
        "InvariantTest1.t.sol",
        r#"
import "forge-std/Test.sol";

contract InvariantBreaker {
    bool public flag0 = true;
    bool public flag1 = true;

    function set0(int256 val) public returns (bool) {
        if (val % 100 == 0) {
            flag0 = false;
        }
        return flag0;
    }

    function set1(int256 val) public returns (bool) {
        if (val % 10 == 0 && !flag0) {
            flag1 = false;
        }
        return flag1;
    }
}

contract InvariantTest is Test {
    InvariantBreaker inv;

    function setUp() public {
        inv = new InvariantBreaker();
    }

    function invariant_neverFalse() public {
        require(inv.flag1(), "false");
    }

    function statefulFuzz_neverFalseWithInvariantAlias() public {
        require(inv.flag1(), "false");
    }
}
"#,
    );

    assert_invariant(cmd.args(["test"])).failure().stdout_eq(str![[r#"
...
Ran 2 tests for test/InvariantTest1.t.sol:InvariantTest
[FAIL: false]
	[SEQUENCE]
 invariant_neverFalse() ([RUNS])

[STATS]

[FAIL: false]
	[SEQUENCE]
 statefulFuzz_neverFalseWithInvariantAlias() ([RUNS])

[STATS]

Suite result: FAILED. 0 passed; 2 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 2 failed, 0 skipped (2 total tests)

Failing tests:
Encountered 2 failing tests in test/InvariantTest1.t.sol:InvariantTest
[FAIL: false]
	[SEQUENCE]
 invariant_neverFalse() ([RUNS])
[FAIL: false]
	[SEQUENCE]
 statefulFuzz_neverFalseWithInvariantAlias() ([RUNS])

Encountered a total of 2 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 2 failed tests

"#]]);
});

forgetest_init!(invariant_warp_and_roll, |prj, cmd| {
    prj.update_config(|config| {
        config.fuzz.seed = Some(U256::from(119u32));
        config.invariant.max_time_delay = Some(604800);
        config.invariant.max_block_delay = Some(60480);
        config.invariant.shrink_run_limit = 0;
    });

    prj.add_test(
        "InvariantWarpAndRoll.t.sol",
        r#"
import "forge-std/Test.sol";

contract Counter {
    uint256 public number;

    function setNumber(uint256 newNumber) public {
        number = newNumber;
    }

    function increment() public {
        number++;
    }
}

contract InvariantWarpAndRoll {
    Counter public counter;

    function setUp() public {
        counter = new Counter();
    }

    function invariant_warp() public view {
        require(block.number < 200000, "max block");
    }

    /// forge-config: default.invariant.show_solidity = true
    function invariant_roll() public view {
        require(block.timestamp < 500000, "max timestamp");
    }
}
"#,
    );

    cmd.args(["test", "--mt", "invariant_warp"]).assert_failure().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/InvariantWarpAndRoll.t.sol:InvariantWarpAndRoll
[FAIL: max block]
	[Sequence] (original: 6, shrunk: 6)
		sender=[..] addr=[test/InvariantWarpAndRoll.t.sol:Counter]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f warp=6280 roll=21461 calldata=setNumber(uint256) args=[200000 [2e5]]
		sender=[..] addr=[test/InvariantWarpAndRoll.t.sol:Counter]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f warp=92060 roll=51816 calldata=setNumber(uint256) args=[0]
		sender=[..] addr=[test/InvariantWarpAndRoll.t.sol:Counter]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f warp=198040 roll=60259 calldata=increment() args=[]
		sender=[..] addr=[test/InvariantWarpAndRoll.t.sol:Counter]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f warp=20609 roll=27086 calldata=setNumber(uint256) args=[26717227324157985679793128079000084308648530834088529513797156275625002 [2.671e70]]
		sender=[..] addr=[test/InvariantWarpAndRoll.t.sol:Counter]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f warp=409368 roll=24864 calldata=increment() args=[]
		sender=[..] addr=[test/InvariantWarpAndRoll.t.sol:Counter]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f warp=218105 roll=17834 calldata=setNumber(uint256) args=[24752675372815722001736610830 [2.475e28]]
 invariant_warp() (runs: 0, calls: 0, reverts: 0)
...

"#]]);

    cmd.forge_fuse().args(["test", "--mt", "invariant_roll"]).assert_failure().stdout_eq(str![[r#"
No files changed, compilation skipped

Ran 1 test for test/InvariantWarpAndRoll.t.sol:InvariantWarpAndRoll
[FAIL: max timestamp]
	[Sequence] (original: 5, shrunk: 5)
		vm.warp(block.timestamp + 6280);
		vm.roll(block.number + 21461);
		vm.prank([..]);
		Counter(0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f).setNumber(200000);
		vm.warp(block.timestamp + 92060);
		vm.roll(block.number + 51816);
		vm.prank([..]);
		Counter(0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f).setNumber(0);
		vm.warp(block.timestamp + 198040);
		vm.roll(block.number + 60259);
		vm.prank([..]);
		Counter(0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f).increment();
		vm.warp(block.timestamp + 20609);
		vm.roll(block.number + 27086);
		vm.prank([..]);
		Counter(0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f).setNumber(26717227324157985679793128079000084308648530834088529513797156275625002);
		vm.warp(block.timestamp + 409368);
		vm.roll(block.number + 24864);
		vm.prank([..]);
		Counter(0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f).increment();
 invariant_roll() (runs: 0, calls: 0, reverts: 0)
...

"#]]);

    // Test that time and block advance in target contract as well.
    prj.update_config(|config| {
        config.invariant.fail_on_revert = true;
    });
    prj.add_test(
        "HandlerWarpAndRoll.t.sol",
        r#"
import "forge-std/Test.sol";

contract Counter {
    uint256 public number;
    function setNumber(uint256 newNumber) public {
        require(block.number < 200000, "max block");
        number = newNumber;
    }

    function increment() public {
        require(block.timestamp < 500000, "max timestamp");
        number++;
    }
}

contract HandlerWarpAndRoll {
    Counter public counter;

    function setUp() public {
        counter = new Counter();
    }

    function invariant_handler() public view {
    }
}
"#,
    );

    cmd.forge_fuse().args(["test", "--mt", "invariant_handler"]).assert_failure().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/HandlerWarpAndRoll.t.sol:HandlerWarpAndRoll
[FAIL: max timestamp]
	[Sequence] (original: 7, shrunk: 7)
		sender=[..] addr=[test/HandlerWarpAndRoll.t.sol:Counter]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f warp=6280 roll=21461 calldata=setNumber(uint256) args=[200000 [2e5]]
		sender=[..] addr=[test/HandlerWarpAndRoll.t.sol:Counter]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f warp=92060 roll=51816 calldata=setNumber(uint256) args=[0]
		sender=[..] addr=[test/HandlerWarpAndRoll.t.sol:Counter]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f warp=198040 roll=60259 calldata=increment() args=[]
		sender=[..] addr=[test/HandlerWarpAndRoll.t.sol:Counter]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f warp=20609 roll=27086 calldata=setNumber(uint256) args=[26717227324157985679793128079000084308648530834088529513797156275625002 [2.671e70]]
		sender=[..] addr=[test/HandlerWarpAndRoll.t.sol:Counter]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f warp=409368 roll=24864 calldata=increment() args=[]
		sender=[..] addr=[test/HandlerWarpAndRoll.t.sol:Counter]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f warp=218105 roll=17834 calldata=setNumber(uint256) args=[24752675372815722001736610830 [2.475e28]]
		sender=[..] addr=[test/HandlerWarpAndRoll.t.sol:Counter]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f warp=579093 roll=23244 calldata=increment() args=[]
...

"#]]);
});
