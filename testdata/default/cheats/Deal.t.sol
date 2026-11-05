// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract DealTest is Test {
    function testDeal(uint256 amount) public {
        address target = address(10);
        assertEq(target.balance, 0, "initial balance incorrect");

        // Give half the amount
        vm.deal(target, amount / 2);
        assertEq(target.balance, amount / 2, "half balance is incorrect");

        // Give the entire amount to check that deal is not additive
        vm.deal(target, amount);
        assertEq(target.balance, amount, "deal did not overwrite balance");
    }
}

contract DealInvariantHandler {
    uint256 public calls;

    function succeed() external {
        calls++;
    }

    function fail() external {
        calls++;
        revert();
    }
}

contract DealInvariantTest is Test {
    address constant SENDER = address(0xA11CE);
    DealInvariantHandler handler;

    function setUp() public {
        handler = new DealInvariantHandler();
        vm.deal(SENDER, 100 ether);
    }

    function targetContracts() public view returns (address[] memory targets) {
        targets = new address[](1);
        targets[0] = address(handler);
    }

    function targetSenders() public pure returns (address[] memory senders) {
        senders = new address[](1);
        senders[0] = SENDER;
    }

    // The deal is committed by `setUp`, so reverted calls from the sender must not undo it.
    /// forge-config: default.isolate = false
    /// forge-config: default.invariant.fail-on-revert = false
    function invariant_dealSurvivesRevertedCalls() public {
        assertEq(SENDER.balance, 100 ether);
    }
}
