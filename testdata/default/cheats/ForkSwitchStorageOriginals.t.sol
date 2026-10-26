// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

/// forge-config: default.isolate = false
contract ForkSwitchStorageOriginalsTest is Test {
    // EIP-3529 refund for restoring a slot that was zero at transaction start.
    uint256 constant RESTORE_ZERO_REFUND = 19_900;

    uint256 forkA;
    uint256 forkB;
    mapping(bool => uint256) values;

    function setUp() public {
        forkA = vm.createSelectFork("mainnet");
        forkB = vm.createFork("mainnet");
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
    }
}

/// forge-config: default.isolate = true
contract ForkSwitchStorageOriginalsIsolatedTest is ForkSwitchStorageOriginalsTest {}
