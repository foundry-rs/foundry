// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import {DSTest} from "ds-test/test.sol";
import {Vm} from "./Vm.sol";

/**
 * @notice Helper contract with a constructo that makes a call to itself then
 *         optionally reverts if zero-length data is passed
 */
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

/**
 * @notice Helper contract that calls itself from the run method
 */
contract Doer {
    function run() public payable {
        this.doStuff{value: msg.value / 10}();
    }

    function doStuff() external payable {}
}

/**
 * @notice Helper contract that calls a Doer from the run method and then
 *         reverts
 */
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

/**
 * @notice Helper contract that calls a Doer from the run method
 */
contract Succeeder {
    Doer immutable doer;

    constructor(Doer _doer) {
        doer = _doer;
    }

    function run() public payable {
        doer.run{value: msg.value / 10}();
    }
}

/**
 * @notice Helper contract that calls a Reverter and Succeeder from the run
 *         method
 */
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

contract RecordAccountAccessesTest is DSTest {
    Vm constant cheats = Vm(HEVM_ADDRESS);
    NestedRunner runner;

    function setUp() public {
        runner = new NestedRunner();
    }

    /**
     * @notice Test that basic account accesses are correctly recorded
     */
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
                isCreate: false,
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
                isCreate: false,
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
                isCreate: false,
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
                isCreate: false,
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
                isCreate: true,
                initialized: false,
                value: 2 ether,
                data: abi.encodePacked(type(SelfCaller).creationCode, abi.encode("hello2 world2")),
                reverted: false
            })
        );
        assertEq(
            called[5],
            Vm.AccountAccess({
                account: address(caller),
                isCreate: false,
                initialized: true,
                value: 0.2 ether,
                data: "",
                reverted: false
            })
        );
    }

    /**
     * @notice Test that account accesses are correctly recorded when a call
     *         reverts
     */
    function testRevertingCall() public {
        cheats.recordAccountAccesses();
        try this.revertingCall{value: 1 ether}(address(1234), "") {} catch {}
        Vm.AccountAccess[] memory called = cheats.getRecordedAccountAccesses();
        assertEq(called.length, 2);
        assertEq(
            called[0],
            Vm.AccountAccess({
                account: address(this),
                isCreate: false,
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
                isCreate: false,
                initialized: false,
                value: 0.1 ether,
                data: "",
                reverted: true
            })
        );
    }

    /**
     * @notice Test that nested account accesses are correctly recorded
     */
    function testNested() public {
        runNested(false);
    }

    /**
     * @notice Test that nested account accesses are correctly recorded when
     *         the first call reverts
     */
    function testNested_Revert() public {
        runNested(true);
    }

    /**
     * @notice Helper function to test nested account accesses
     * @param shouldRevert Whether the first call should revert
     */
    function runNested(bool shouldRevert) public {
        cheats.recordAccountAccesses();
        try runner.run{value: 1 ether}(shouldRevert) {} catch {}
        Vm.AccountAccess[] memory called = cheats.getRecordedAccountAccesses();
        assertEq(called.length, 7, "incorrect length");
        assertEq(
            called[0],
            Vm.AccountAccess({
                account: address(runner),
                isCreate: false,
                initialized: true,
                value: 1 ether,
                data: abi.encodeCall(NestedRunner.run, (shouldRevert)),
                reverted: shouldRevert
            })
        );
        assertEq(
            called[1],
            Vm.AccountAccess({
                account: address(runner.reverter()),
                isCreate: false,
                initialized: true,
                value: 0.1 ether,
                data: abi.encodeCall(Reverter.run, ()),
                reverted: true
            })
        );
        assertEq(
            called[2],
            Vm.AccountAccess({
                account: address(runner.doer()),
                isCreate: false,
                initialized: true,
                value: 0.01 ether,
                data: abi.encodeCall(Doer.run, ()),
                reverted: true
            })
        );
        assertEq(
            called[3],
            Vm.AccountAccess({
                account: address(runner.doer()),
                isCreate: false,
                initialized: true,
                value: 0.001 ether,
                data: abi.encodeCall(Doer.doStuff, ()),
                reverted: true
            })
        );

        assertEq(
            called[4],
            Vm.AccountAccess({
                account: address(runner.succeeder()),
                isCreate: false,
                initialized: true,
                value: 0.1 ether,
                data: abi.encodeCall(Succeeder.run, ()),
                reverted: shouldRevert
            })
        );
        assertEq(
            called[5],
            Vm.AccountAccess({
                account: address(runner.doer()),
                isCreate: false,
                initialized: true,
                value: 0.01 ether,
                data: abi.encodeCall(Doer.run, ()),
                reverted: shouldRevert
            })
        );
        assertEq(
            called[6],
            Vm.AccountAccess({
                account: address(runner.doer()),
                isCreate: false,
                initialized: true,
                value: 0.001 ether,
                data: abi.encodeCall(Doer.doStuff, ()),
                reverted: shouldRevert
            })
        );
    }

    /**
     * @notice Test that account accesses are correctly recorded when the
     *         recording is started from a lower depth than they are
     *         retrieved
     */
    function testNested_LowerDepth() public {
        this.startRecordingFromLowerDepth();
        testNested();
        this.startRecordingFromLowerDepth();
        testNested_Revert();
    }

    /**
     * @notice Test that constructor calls and calls made within a constructor
     *         are correctly recorded, even if it reverts
     */
    function testCreateRevert() public {
        cheats.recordAccountAccesses();
        try new SelfCaller('') {} catch {}
        address hypotheticalAddress = 0x185a4dc360CE69bDCceE33b3784B0282f7961aea;
        Vm.AccountAccess[] memory called = cheats.getRecordedAccountAccesses();
        assertEq(called.length, 2, "incorrect length");
        assertEq(
            called[0],
            Vm.AccountAccess({
                account: hypotheticalAddress,
                isCreate: true,
                initialized: false,
                value: 0,
                data: abi.encodePacked(type(SelfCaller).creationCode, abi.encode("")),
                reverted: true
            })
        );
        assertEq(
            called[1],
            Vm.AccountAccess({
                account: hypotheticalAddress,
                isCreate: false,
                initialized: true,
                value: 0,
                data: "",
                reverted: true
            })
        );
    }

    function startRecordingFromLowerDepth() external {
        cheats.recordAccountAccesses();
    }

    function revertingCall(address target, bytes memory data) external payable {
        assembly {
            pop(call(gas(), target, div(callvalue(), 10), add(data, 0x20), mload(data), 0, 0))
        }
        revert();
    }

    function assertEq(Vm.AccountAccess memory actualAccess, Vm.AccountAccess memory expectedAccess) internal {
        assertEq(actualAccess.account, expectedAccess.account, "incorrect account");
        assertEq(toUint(actualAccess.isCreate), toUint(expectedAccess.isCreate), "incorrect isCreate");
        assertEq(toUint(actualAccess.initialized), toUint(expectedAccess.initialized), "incorrect initialized");
        assertEq(actualAccess.value, expectedAccess.value, "incorrect value");
        assertEq(actualAccess.data, expectedAccess.data, "incorrect data");
    }

    function toUint(bool a) internal pure returns (uint256) {
        return a ? 1 : 0;
    }
}
