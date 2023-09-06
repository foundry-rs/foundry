// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import {DSTest} from "ds-test/test.sol";
import {Vm} from "./Vm.sol";

/**
 * @notice Helper contract with a constructor that stores a value in storage
 *         and then optionally reverts.
 */
contract ConstructorStorer {
    constructor(bool shouldRevert) {
        assembly {
            sstore(0x00, 0x01)
            if shouldRevert { revert(0, 0) }
        }
    }
}

/**
 * @notice Helper contract that stores and reads in addition to calling a
 *         function on itself (which also accesses storage)
 */
contract Doer {
    uint256[10] spacer;
    mapping(bytes32 key => uint256 value) slots;

    constructor() {
        slots[bytes32("doer 1")] = 10;
    }

    function run() public {
        slots[bytes32("doer 1")]++;
        this.doStuff();
    }

    function doStuff() external {
        slots[bytes32("doer 2")]++;
    }
}

/**
 * @notice
 */
contract Reverter {
    Doer immutable doer;
    mapping(bytes32 key => uint256 value) slots;

    constructor(Doer _doer) {
        doer = _doer;
    }

    function run() public {
        doer.run();
        slots[bytes32("reverter")]++;
        revert();
    }
}

/**
 * @notice
 */
contract Succeeder {
    Doer immutable doer;
    mapping(bytes32 key => uint256 value) slots;

    constructor(Doer _doer) {
        doer = _doer;
    }

    function run() public {
        slots[bytes32("succeeder")]++;
        doer.run();
    }
}

/**
 * @notice
 */
contract NestedRunner {
    Doer public immutable doer;
    Reverter public immutable reverter;
    Succeeder public immutable succeeder;
    mapping(bytes32 key => uint256 value) slots;

    constructor() {
        doer = new Doer();
        reverter = new Reverter(doer);
        succeeder = new Succeeder(doer);
    }

    function run(bool shouldRevert) public {
        slots[bytes32("runner")]++;
        try reverter.run() {
            if (shouldRevert) {
                revert();
            }
        } catch {}
        succeeder.run();
        if (shouldRevert) {
            revert();
        }
    }
}

/**
 * @notice Helper contract that directly reads from and writes to storage
 */
contract StorageAccessor {
    function read(bytes32 slot) public view returns (bytes32 value) {
        assembly {
            value := sload(slot)
        }
    }

    function write(bytes32 slot, bytes32 value) public {
        assembly {
            sstore(slot, value)
        }
    }
}

contract RecordStorageAccessesTest is DSTest {
    Vm constant cheats = Vm(HEVM_ADDRESS);
    StorageAccessor test1;
    StorageAccessor test2;
    NestedRunner runner;
    uint256 counter;

    function setUp() public {
        test1 = new StorageAccessor();
        test2 = new StorageAccessor();
        runner = new NestedRunner();
        counter = 5;
    }

    /**
     * @notice Test normal, non-nested storage accesses
     */
    function testRecordAccesses() public {
        StorageAccessor one = test1;
        StorageAccessor two = test2;
        cheats.recordStorageAccesses();
        one.read(bytes32(uint256(1234)));
        one.write(bytes32(uint256(1235)), bytes32(uint256(5678)));
        two.write(bytes32(uint256(5678)), bytes32(uint256(123469)));
        two.write(bytes32(uint256(5678)), bytes32(uint256(1234)));

        two.read(bytes32(uint256(5678)));

        Vm.StorageAccess[] memory accessed = cheats.getRecordedStorageAccesses();
        assertEq(accessed.length, 5, "incorrect length");
        Vm.StorageAccess memory access = accessed[0];
        assertEq(
            access,
            Vm.StorageAccess({
                account: address(one),
                slot: bytes32(uint256(1234)),
                isWrite: false,
                previousValue: bytes32(uint256(0)),
                newValue: bytes32(uint256(0)),
                reverted: false
            })
        );

        access = accessed[1];
        assertEq(
            access,
            Vm.StorageAccess({
                account: address(one),
                slot: bytes32(uint256(1235)),
                isWrite: true,
                previousValue: bytes32(uint256(0)),
                newValue: bytes32(uint256(5678)),
                reverted: false
            })
        );

        access = accessed[2];
        assertEq(
            access,
            Vm.StorageAccess({
                account: address(two),
                slot: bytes32(uint256(5678)),
                isWrite: true,
                previousValue: bytes32(uint256(0)),
                newValue: bytes32(uint256(123469)),
                reverted: false
            })
        );

        access = accessed[3];
        assertEq(
            access,
            Vm.StorageAccess({
                account: address(two),
                slot: bytes32(uint256(5678)),
                isWrite: true,
                previousValue: bytes32(uint256(123469)),
                newValue: bytes32(uint256(1234)),
                reverted: false
            })
        );
        access = accessed[4];
        assertEq(
            access,
            Vm.StorageAccess({
                account: address(two),
                slot: bytes32(uint256(5678)),
                isWrite: false,
                previousValue: bytes32(uint256(1234)),
                newValue: bytes32(uint256(1234)),
                reverted: false
            })
        );
    }

    /**
     * @notice Test storage access recordings with multiple nested calls, some
     *         reverting, but overall successful.
     */
    function testNested() public {
        cheats.recordStorageAccesses();
        runNested(false, false);
    }

    /**
     * @notice Test storage access recordings with multiple nested calls, some
     *         reverting, with the first call reverting
     *
     */
    function testNested_Revert() public {
        cheats.recordStorageAccesses();

        runNested(true, false);
    }

    /**
     * @notice Test that constructor storage accesses are recorded, including reverts
     */
    function testConstructorStorage() public {
        cheats.recordStorageAccesses();
        address storer = address(new ConstructorStorer(false));
        try new ConstructorStorer(true) {} catch {}
        address hypotheticalStorer = 0x42997aC9251E5BB0A61F4Ff790E5B991ea07Fd9B;
        Vm.StorageAccess[] memory accessed = cheats.getRecordedStorageAccesses();
        assertEq(accessed.length, 2, "incorrect length");
        assertEq(
            accessed[0],
            Vm.StorageAccess({
                account: storer,
                slot: bytes32(uint256(0)),
                isWrite: true,
                previousValue: bytes32(uint256(0)),
                newValue: bytes32(uint256(1)),
                reverted: false
            })
        );
        assertEq(
            accessed[1],
            Vm.StorageAccess({
                account: hypotheticalStorer,
                slot: bytes32(uint256(0)),
                isWrite: true,
                previousValue: bytes32(uint256(0)),
                newValue: bytes32(uint256(1)),
                reverted: true
            })
        );
    }

    /**
     * @notice Test that storage accesses are still recorded when the recording is started
     *         from a lower call depth than the results are read from
     */
    function testNested_LowerDepth() public {
        this.startRecordingFromLowerDepth();
        runNested(false, true);
    }

    /**
     * @notice Test that storage accesses are still recorded when the recording is started
     *         from a lower call depth than the results are read from, and the first call
     *         reverts
     */
    function testNested_LowerDepth_Revert() public {
        this.startRecordingFromLowerDepth();
        runNested(true, true);
    }

    function runNested(bool shouldRevert, bool expectFirst) internal {
        try runner.run(shouldRevert) {} catch {}
        Vm.StorageAccess[] memory accessed = cheats.getRecordedStorageAccesses();
        assertEq(accessed.length, 15 + toUint(expectFirst), "incorrect length");

        uint256 startingIndex = toUint(expectFirst);
        if (expectFirst) {
            bytes32 counterSlot;
            assembly {
                counterSlot := counter.slot
            }
            assertEq(
                accessed[0],
                Vm.StorageAccess({
                    account: address(this),
                    slot: counterSlot,
                    isWrite: false,
                    previousValue: bytes32(uint256(5)),
                    newValue: bytes32(uint256(5)),
                    reverted: false
                })
            );
        }
        bytes32 runnerSlot;
        assembly {
            runnerSlot := runner.slot
        }
        assertEq(
            accessed[startingIndex],
            Vm.StorageAccess({
                account: address(this),
                slot: runnerSlot,
                isWrite: false,
                previousValue: bytes32(uint256(uint160(address(runner)))),
                newValue: bytes32(uint256(uint160(address(runner)))),
                reverted: false
            })
        );

        assertIncrementEq(
            accessed[startingIndex + 1],
            accessed[startingIndex + 2],
            Vm.StorageAccess({
                account: address(runner),
                slot: keccak256(abi.encodePacked(bytes32("runner"), bytes32(0))),
                isWrite: true,
                previousValue: bytes32(uint256(0)),
                newValue: bytes32(uint256(1)),
                reverted: shouldRevert
            })
        );
        assertIncrementEq(
            accessed[startingIndex + 3],
            accessed[startingIndex + 4],
            Vm.StorageAccess({
                account: address(runner.doer()),
                slot: keccak256(abi.encodePacked(bytes32("doer 1"), uint256(10))),
                isWrite: true,
                previousValue: bytes32(uint256(10)),
                newValue: bytes32(uint256(11)),
                reverted: true
            })
        );

        assertIncrementEq(
            accessed[startingIndex + 5],
            accessed[startingIndex + 6],
            Vm.StorageAccess({
                account: address(runner.doer()),
                slot: keccak256(abi.encodePacked(bytes32("doer 2"), uint256(10))),
                isWrite: true,
                previousValue: bytes32(uint256(0)),
                newValue: bytes32(uint256(1)),
                reverted: true
            })
        );

        assertIncrementEq(
            accessed[startingIndex + 7],
            accessed[startingIndex + 8],
            Vm.StorageAccess({
                account: address(runner.reverter()),
                slot: keccak256(abi.encodePacked(bytes32("reverter"), uint256(0))),
                isWrite: true,
                previousValue: bytes32(uint256(0)),
                newValue: bytes32(uint256(1)),
                reverted: true
            })
        );

        assertIncrementEq(
            accessed[startingIndex + 9],
            accessed[startingIndex + 10],
            Vm.StorageAccess({
                account: address(runner.succeeder()),
                slot: keccak256(abi.encodePacked(bytes32("succeeder"), uint256(0))),
                isWrite: true,
                previousValue: bytes32(uint256(0)),
                newValue: bytes32(uint256(1)),
                reverted: shouldRevert
            })
        );

        Vm.StorageAccess memory expected = Vm.StorageAccess({
            account: address(runner.doer()),
            slot: keccak256(abi.encodePacked(bytes32("doer 1"), uint256(10))),
            isWrite: true,
            previousValue: bytes32(uint256(10)),
            newValue: bytes32(uint256(11)),
            reverted: shouldRevert
        });

        assertIncrementEq(accessed[startingIndex + 11], accessed[startingIndex + 12], expected);

        assertIncrementEq(
            accessed[startingIndex + 13],
            accessed[startingIndex + 14],
            Vm.StorageAccess({
                account: address(runner.doer()),
                slot: keccak256(abi.encodePacked(bytes32("doer 2"), uint256(10))),
                isWrite: true,
                previousValue: bytes32(uint256(0)),
                newValue: bytes32(uint256(1)),
                reverted: shouldRevert
            })
        );
    }

    function startRecordingFromLowerDepth() external returns (uint256 value) {
        cheats.recordStorageAccesses();
        assembly {
            // assign to a return value otherwise optimizer will remove
            value := sload(counter.slot)
        }
    }

    function assertIncrementEq(
        Vm.StorageAccess memory read,
        Vm.StorageAccess memory write,
        Vm.StorageAccess memory expected
    ) internal {
        assertEq(
            read,
            Vm.StorageAccess({
                account: expected.account,
                slot: expected.slot,
                isWrite: false,
                previousValue: expected.previousValue,
                newValue: expected.previousValue,
                reverted: expected.reverted
            })
        );
        assertEq(
            write,
            Vm.StorageAccess({
                account: expected.account,
                slot: expected.slot,
                isWrite: true,
                previousValue: expected.previousValue,
                newValue: expected.newValue,
                reverted: expected.reverted
            })
        );
    }

    function assertEq(Vm.StorageAccess memory actual, Vm.StorageAccess memory expected) internal {
        assertEq(actual.account, expected.account, "incorrect account");
        assertEq(actual.slot, expected.slot, "incorrect slot");
        assertEq(toUint(actual.isWrite), toUint(expected.isWrite), "incorrect isWrite");
        assertEq(actual.previousValue, expected.previousValue, "incorrect previousValue");
        assertEq(actual.newValue, expected.newValue, "incorrect newValue");
        assertEq(toUint(actual.reverted), toUint(expected.reverted), "incorrect reverted");
    }

    function toUint(bool a) internal pure returns (uint256) {
        return a ? 1 : 0;
    }
}
