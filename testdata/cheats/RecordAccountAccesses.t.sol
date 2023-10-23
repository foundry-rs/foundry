// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Vm.sol";

/// @notice Helper contract with a constructo that makes a call to itself then
///         optionally reverts if zero-length data is passed
contract SelfCaller {
    constructor(bytes memory) payable {
        assembly {
            // call self to test that the cheatcote correctly reports the
            // account as initialized even when there is no code at the
            // contract address
            pop(call(gas(), address(), div(callvalue(), 10), 0, 0, 0, 0))
            if eq(calldataload(0x04), 1) { revert(0, 0) }
        }
    }
}

/// @notice Helper contract that calls itself from the run method
contract Doer {
    function run() public payable {
        this.doStuff{value: msg.value / 10}();
    }

    function doStuff() external payable {}
}

/// @notice Helper contract that selfdestructs to a target address within its
///         constructor
contract SelfDestructor {
    constructor(address target) payable {
        selfdestruct(payable(target));
    }
}

/// @notice Helper contract that calls a Doer from the run method
contract Create2or {
    function create2(bytes32 salt, bytes memory initcode) external payable returns (address result) {
        assembly {
            result := create2(callvalue(), add(initcode, 0x20), mload(initcode), salt)
        }
    }
}

/// @notice Helper contract that calls a Doer from the run method and then
///         reverts
contract Reverter {
    Doer immutable doer;

    constructor(Doer _doer) {
        doer = _doer;
    }

    function run() public payable {
        doer.run{value: msg.value / 10}();
        revert();
    }
}

/// @notice Helper contract that calls a Doer from the run method
contract Succeeder {
    Doer immutable doer;

    constructor(Doer _doer) {
        doer = _doer;
    }

    function run() public payable {
        doer.run{value: msg.value / 10}();
    }
}

/// @notice Helper contract that calls a Reverter and Succeeder from the run
///         method
contract NestedRunner {
    Doer public immutable doer;
    Reverter public immutable reverter;
    Succeeder public immutable succeeder;

    constructor() {
        doer = new Doer();
        reverter = new Reverter(doer);
        succeeder = new Succeeder(doer);
    }

    function run(bool shouldRevert) public payable {
        try reverter.run{value: msg.value / 10}() {
            if (shouldRevert) {
                revert();
            }
        } catch {}
        succeeder.run{value: msg.value / 10}();
        if (shouldRevert) {
            revert();
        }
    }
}

/// @notice Test that the cheatcode correctly records account accesses
contract RecordAccountAccessesTest is DSTest {
    Vm constant cheats = Vm(HEVM_ADDRESS);
    NestedRunner runner;
    Create2or create2or;

    function setUp() public {
        runner = new NestedRunner();
        create2or = new Create2or();
    }

    /// @notice Test that basic account accesses are correctly recorded
    function testRecordAccountAccesses() public {
        cheats.recordAccountAccesses();

        (bool succ,) = address(1234).call("");
        (succ,) = address(5678).call{value: 1 ether}("");
        (succ,) = address(123469).call("hello world");
        (succ,) = address(5678).call("");
        // contract calls to self in constructor
        SelfCaller caller = new SelfCaller{value: 2 ether}('hello2 world2');

        Vm.AccountAccess[] memory called = cheats.getRecordedAccountAccesses();
        assertEq(called.length, 6);
        assertEq(
            called[0],
            Vm.AccountAccess({
                account: address(1234),
                kind: Vm.AccountAccessKind.Call,
                initialized: false,
                value: 0,
                data: "",
                reverted: false
            })
        );

        assertEq(
            called[1],
            Vm.AccountAccess({
                account: address(5678),
                kind: Vm.AccountAccessKind.Call,
                initialized: false,
                value: 1 ether,
                data: "",
                reverted: false
            })
        );
        assertEq(
            called[2],
            Vm.AccountAccess({
                account: address(123469),
                kind: Vm.AccountAccessKind.Call,
                initialized: false,
                value: 0,
                data: "hello world",
                reverted: false
            })
        );
        assertEq(
            called[3],
            Vm.AccountAccess({
                account: address(5678),
                kind: Vm.AccountAccessKind.Call,
                initialized: true,
                value: 0,
                data: "",
                reverted: false
            })
        );
        assertEq(
            called[4],
            Vm.AccountAccess({
                account: address(caller),
                kind: Vm.AccountAccessKind.Create,
                initialized: true,
                value: 2 ether,
                data: abi.encodePacked(type(SelfCaller).creationCode, abi.encode("hello2 world2")),
                reverted: false
            })
        );
        assertEq(
            called[5],
            Vm.AccountAccess({
                account: address(caller),
                kind: Vm.AccountAccessKind.Call,
                initialized: true,
                value: 0.2 ether,
                data: "",
                reverted: false
            })
        );
    }

    /// @notice Test that account accesses are correctly recorded when a call
    ///         reverts
    function testRevertingCall() public {
        cheats.recordAccountAccesses();
        try this.revertingCall{value: 1 ether}(address(1234), "") {} catch {}
        Vm.AccountAccess[] memory called = cheats.getRecordedAccountAccesses();
        assertEq(called.length, 2);
        assertEq(
            called[0],
            Vm.AccountAccess({
                account: address(this),
                kind: Vm.AccountAccessKind.Call,
                initialized: true,
                value: 1 ether,
                data: abi.encodeCall(this.revertingCall, (address(1234), "")),
                reverted: true
            })
        );
        assertEq(
            called[1],
            Vm.AccountAccess({
                account: address(1234),
                kind: Vm.AccountAccessKind.Call,
                initialized: false,
                value: 0.1 ether,
                data: "",
                reverted: true
            })
        );
    }

    /// @notice Test that nested account accesses are correctly recorded
    function testNested() public {
        cheats.recordAccountAccesses();
        runNested(false, false);
    }

    /// @notice Test that nested account accesses are correctly recorded when
    ///         the first call reverts
    function testNested_Revert() public {
        cheats.recordAccountAccesses();
        runNested(true, false);
    }

    /// @notice Helper function to test nested account accesses
    /// @param shouldRevert Whether the first call should revert
    function runNested(bool shouldRevert, bool expectFirstCall) public {
        try runner.run{value: 1 ether}(shouldRevert) {} catch {}
        Vm.AccountAccess[] memory called = cheats.getRecordedAccountAccesses();
        assertEq(called.length, 7 + toUint(expectFirstCall), "incorrect length");
        if (expectFirstCall) {
            assertEq(
                called[0],
                Vm.AccountAccess({
                    account: address(1234),
                    kind: Vm.AccountAccessKind.Call,
                    initialized: false,
                    value: 0,
                    data: "",
                    reverted: false
                })
            );
        }

        uint256 startingIndex = toUint(expectFirstCall);
        assertEq(
            called[startingIndex],
            Vm.AccountAccess({
                account: address(runner),
                kind: Vm.AccountAccessKind.Call,
                initialized: true,
                value: 1 ether,
                data: abi.encodeCall(NestedRunner.run, (shouldRevert)),
                reverted: shouldRevert
            })
        );
        assertEq(
            called[startingIndex + 1],
            Vm.AccountAccess({
                account: address(runner.reverter()),
                kind: Vm.AccountAccessKind.Call,
                initialized: true,
                value: 0.1 ether,
                data: abi.encodeCall(Reverter.run, ()),
                reverted: true
            })
        );
        assertEq(
            called[startingIndex + 2],
            Vm.AccountAccess({
                account: address(runner.doer()),
                kind: Vm.AccountAccessKind.Call,
                initialized: true,
                value: 0.01 ether,
                data: abi.encodeCall(Doer.run, ()),
                reverted: true
            })
        );
        assertEq(
            called[startingIndex + 3],
            Vm.AccountAccess({
                account: address(runner.doer()),
                kind: Vm.AccountAccessKind.Call,
                initialized: true,
                value: 0.001 ether,
                data: abi.encodeCall(Doer.doStuff, ()),
                reverted: true
            })
        );

        assertEq(
            called[startingIndex + 4],
            Vm.AccountAccess({
                account: address(runner.succeeder()),
                kind: Vm.AccountAccessKind.Call,
                initialized: true,
                value: 0.1 ether,
                data: abi.encodeCall(Succeeder.run, ()),
                reverted: shouldRevert
            })
        );
        assertEq(
            called[startingIndex + 5],
            Vm.AccountAccess({
                account: address(runner.doer()),
                kind: Vm.AccountAccessKind.Call,
                initialized: true,
                value: 0.01 ether,
                data: abi.encodeCall(Doer.run, ()),
                reverted: shouldRevert
            })
        );
        assertEq(
            called[startingIndex + 6],
            Vm.AccountAccess({
                account: address(runner.doer()),
                kind: Vm.AccountAccessKind.Call,
                initialized: true,
                value: 0.001 ether,
                data: abi.encodeCall(Doer.doStuff, ()),
                reverted: shouldRevert
            })
        );
    }

    /// @notice Test that account accesses are correctly recorded when the
    ///         recording is started from a lower depth than they are
    ///         retrieved
    function testNested_LowerDepth() public {
        this.startRecordingFromLowerDepth();
        runNested(true, true);
        this.startRecordingFromLowerDepth();
        runNested(false, true);
    }

    /// @notice Test that constructor calls and calls made within a constructor
    ///         are correctly recorded, even if it reverts
    function testCreateRevert() public {
        cheats.recordAccountAccesses();
        bytes memory creationCode = abi.encodePacked(type(SelfCaller).creationCode, abi.encode(""));
        try create2or.create2(bytes32(0), creationCode) {} catch {}
        address hypotheticalAddress = deriveCreate2Address(address(create2or), bytes32(0), keccak256(creationCode));
        Vm.AccountAccess[] memory called = cheats.getRecordedAccountAccesses();
        assertEq(called.length, 3, "incorrect length");
        assertEq(
            called[1],
            Vm.AccountAccess({
                account: hypotheticalAddress,
                kind: Vm.AccountAccessKind.Create,
                initialized: true,
                value: 0,
                data: creationCode,
                reverted: true
            })
        );
        assertEq(
            called[2],
            Vm.AccountAccess({
                account: hypotheticalAddress,
                kind: Vm.AccountAccessKind.Call,
                initialized: true,
                value: 0,
                data: "",
                reverted: true
            })
        );
    }

    /// @notice It is important to test SELFDESTRUCT behavior as long as there
    ///         are public networks that support the opcode, regardless of whether
    ///         or not Ethereum mainnet does.
    function testSelfDestruct() public {
        this.startRecordingFromLowerDepth();
        address a = address(new SelfDestructor{value:1 ether}(address(this)));
        address b = address(new SelfDestructor{value:1 ether}(address(bytes20("doesn't exist yet"))));
        Vm.AccountAccess[] memory called = cheats.getRecordedAccountAccesses();
        assertEq(called.length, 5, "incorrect length");
        assertEq(
            called[1],
            Vm.AccountAccess({
                account: a,
                kind: Vm.AccountAccessKind.Create,
                initialized: true,
                value: 1 ether,
                data: abi.encodePacked(type(SelfDestructor).creationCode, abi.encode(address(this))),
                reverted: false
            })
        );
        assertEq(
            called[2],
            Vm.AccountAccess({
                account: address(this),
                kind: Vm.AccountAccessKind.SelfDestruct,
                initialized: true,
                value: 1 ether,
                data: "",
                reverted: false
            })
        );
        assertEq(
            called[3],
            Vm.AccountAccess({
                account: b,
                kind: Vm.AccountAccessKind.Create,
                initialized: true,
                value: 1 ether,
                data: abi.encodePacked(type(SelfDestructor).creationCode, abi.encode(address(bytes20("doesn't exist yet")))),
                reverted: false
            })
        );
        assertEq(
            called[4],
            Vm.AccountAccess({
                account: address(bytes20("doesn't exist yet")),
                kind: Vm.AccountAccessKind.SelfDestruct,
                initialized: false,
                value: 1 ether,
                data: "",
                reverted: false
            })
        );
    }

    function startRecordingFromLowerDepth() external {
        cheats.recordAccountAccesses();
        assembly {
            pop(call(gas(), 1234, 0, 0, 0, 0, 0))
        }
    }

    function revertingCall(address target, bytes memory data) external payable {
        assembly {
            pop(call(gas(), target, div(callvalue(), 10), add(data, 0x20), mload(data), 0, 0))
        }
        revert();
    }

    function assertEq(Vm.AccountAccess memory actualAccess, Vm.AccountAccess memory expectedAccess) internal {
        assertEq(actualAccess.account, expectedAccess.account, "incorrect account");
        assertEq(toUint(actualAccess.kind), toUint(expectedAccess.kind), "incorrect isCreate");
        assertEq(toUint(actualAccess.initialized), toUint(expectedAccess.initialized), "incorrect initialized");
        assertEq(actualAccess.value, expectedAccess.value, "incorrect value");
        assertEq(actualAccess.data, expectedAccess.data, "incorrect data");
    }

    function toUint(Vm.AccountAccessKind kind) internal pure returns (uint256 value) {
        assembly {
            value := and(kind, 0xff)
        }
    }

    function toUint(bool a) internal pure returns (uint256) {
        return a ? 1 : 0;
    }

    function deriveCreate2Address(address deployer, bytes32 salt, bytes32 codeHash) internal pure returns (address) {
        return address(uint160(uint256(keccak256(abi.encodePacked(bytes1(0xff), deployer, salt, codeHash)))));
    }
}
