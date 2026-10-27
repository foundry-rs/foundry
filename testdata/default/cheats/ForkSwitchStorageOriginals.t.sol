// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract Slot {
    uint256 value;

    function set(uint256 newValue) external {
        value = newValue;
    }
}

/// forge-config: default.isolate = false
contract ForkSwitchStorageOriginalsTest is Test {
    // EIP-3529 refund for restoring a slot that was zero at transaction start.
    uint256 constant RESTORE_ZERO_REFUND = 19_900;
    address constant WETH = 0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2;

    uint256 forkA;
    uint256 forkB;
    mapping(bool => uint256) values;
    // Contracts that exist only on fork A and are not persistent.
    mapping(bool => Slot) local;

    function setUp() public {
        forkA = vm.createSelectFork("mainnet");
        forkB = vm.createFork("mainnet");
        local[false] = new Slot();
        local[true] = new Slot();
    }

    function store(bool switching, uint256 value) internal returns (uint256 cost) {
        uint256 start = gasleft();
        values[switching] = value;
        cost = start - gasleft();
    }

    function roundTrip(bool switching) internal {
        if (switching) {
            vm.selectFork(forkB);
            vm.selectFork(forkA);
        }
    }

    // Writes a slot that is zero at transaction start, optionally switching forks between writes,
    // and returns the cost of each write after the first.
    function write(bool switching) external returns (uint256 second, uint256 third, uint256 restore) {
        // An isolated call caps its refund at a fifth of its gas, which must stay above the refund.
        uint256 start = gasleft();
        while (start - gasleft() < 5 * RESTORE_ZERO_REFUND) {}
        store(switching, 1);
        roundTrip(switching);
        second = store(switching, 2);
        roundTrip(switching);
        third = store(switching, 3);
        restore = store(switching, 0);
        roundTrip(switching);
    }

    // Fork switches keep the transaction-start originals of storage slots, so writes after a
    // switch are charged and refunded as without one.
    function testForkSwitchKeepsStorageOriginals() public {
        (uint256 second, uint256 third, uint256 restore) = this.write(false);
        assertEq(vm.lastCallGas().gasRefunded, int256(RESTORE_ZERO_REFUND), "control refund");

        (uint256 secondSwitched, uint256 thirdSwitched, uint256 restoreSwitched) = this.write(true);
        assertEq(vm.lastCallGas().gasRefunded, int256(RESTORE_ZERO_REFUND), "restore refund");
        assertEq(secondSwitched, second, "write after one round trip");
        assertEq(thirdSwitched, third, "write after two round trips");
        assertEq(restoreSwitched, restore, "restoring write");
        assertEq(values[true], 0, "restoring write after a round trip");
    }

    function dirty(bool switching) external {
        store(switching, 1);
        roundTrip(switching);
    }

    function rewrite(bool switching) external returns (uint256 cost) {
        // Warm the slot in both cases, which carries its warmth across the switch.
        require(values[switching] == 1);
        if (switching) vm.selectFork(forkB);
        cost = store(switching, 2);
        if (switching) vm.selectFork(forkA);
    }

    // A fork switch in an earlier call does not leave its originals behind for a later call that
    // switches to the same fork.
    function testForkSwitchKeepsLaterCallStorageOriginals() public {
        this.dirty(false);
        uint256 cost = this.rewrite(false);

        this.dirty(true);
        assertEq(this.rewrite(true), cost, "write after switching in a later call");
    }

    function writeLocal(bool switching) external returns (uint256 cost) {
        local[switching].set(1);
        roundTrip(switching);
        uint256 start = gasleft();
        local[switching].set(2);
        cost = start - gasleft();
    }

    // A fork keeps the originals of accounts that do not leave it.
    function testForkSwitchKeepsLocalStorageOriginals() public {
        assertEq(this.writeLocal(true), this.writeLocal(false), "write after a round trip");
    }

    function sloadCost(uint256 key) internal view returns (uint256 cost) {
        assembly {
            cost := gas()
            let value := sload(key)
            cost := sub(cost, gas())
            if eq(value, 0xdeadbeef) { revert(0, 0) }
        }
    }

    // Reads a slot of this persistent contract on fork A, then another one and a slot of WETH,
    // which is local to fork B, on fork B.
    function readAcrossForks() external returns (uint256 onA, uint256 persistentOnB, uint256 localOnB) {
        onA = sloadCost(0x1234);
        vm.selectFork(forkB);
        persistentOnB = sloadCost(0x1235);
        uint256 start = gasleft();
        (bool success,) = WETH.staticcall(abi.encodeWithSignature("balanceOf(address)", address(this)));
        require(success);
        localOnB = start - gasleft();
        vm.selectFork(forkA);
    }

    // A persistent slot an earlier call read on a fork is as warm in a later call as one it read
    // before switching, while a fork-local account stays warm on its fork.
    function testForkSwitchKeepsLaterCallPersistentWarmth() public {
        (,, uint256 localCost) = this.readAcrossForks();
        (uint256 onA, uint256 persistentOnB, uint256 localOnB) = this.readAcrossForks();
        assertEq(persistentOnB, onA, "persistent slot warmth");
        assertEq(localCost - localOnB, 4500, "fork-local account and slot warmth");
    }

    function readAfterRevertAndPersist() external returns (uint256 persistentOnB, uint256 localSaved) {
        uint256 snapshot = vm.snapshotState();
        (,, uint256 localCost) = this.readAcrossForks();
        vm.revertToState(snapshot);
        vm.makePersistent(WETH);
        uint256 localOnB;
        (, persistentOnB, localOnB) = this.readAcrossForks();
        localSaved = localCost - localOnB;
    }

    // Within a call, a fork keeps the warmth of persistent slots it read across a snapshot revert
    // and of accounts made persistent since.
    function testForkSwitchKeepsPersistentWarmthWithinCall() public {
        (uint256 persistentOnB, uint256 localSaved) = this.readAfterRevertAndPersist();
        assertEq(persistentOnB, 110, "persistent slot warmth after a snapshot revert");
        assertEq(localSaved, 4500, "warmth of an account made persistent");
    }
}

/// forge-config: default.isolate = true
contract ForkSwitchStorageOriginalsIsolatedTest is ForkSwitchStorageOriginalsTest {}
