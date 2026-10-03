// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {Escrow} from "../src/Escrow.sol";
import {FuzzBase, bound, vm} from "./utils/FuzzBase.sol";

contract EscrowHandler {
    address public constant BUYER = address(0xB1);
    address public constant SELLER = address(0x5E);
    address public constant ARBITER = address(0xA7);

    Escrow public immutable escrow;
    bool public disputeSettledWithoutArbiter;

    constructor() {
        escrow = new Escrow(BUYER, SELLER, ARBITER);
    }

    function fund(uint256 value) external {
        value = bound(value, 1, 100 ether);
        vm.deal(BUYER, value);
        vm.prank(BUYER);
        escrow.fund{value: value}();
    }

    function markShipped() external {
        vm.prank(SELLER);
        escrow.markShipped();
    }

    function confirmReceipt() external {
        vm.prank(BUYER);
        escrow.confirmReceipt();
    }

    function dispute() external {
        vm.prank(BUYER);
        escrow.dispute();
    }

    function resolve(bool refundBuyer) external {
        vm.prank(ARBITER);
        escrow.resolve(refundBuyer);
    }

    function claimAfterTimeout() external {
        bool disputed = escrow.state() == Escrow.State.Disputed;
        vm.prank(SELLER);
        escrow.claimAfterTimeout();
        if (disputed) disputeSettledWithoutArbiter = true;
    }

    function reopen() external {
        escrow.reopen();
    }

    function wait(uint256 seconds_) external {
        vm.warp(block.timestamp + bound(seconds_, 0, 3 days));
    }
}

contract EscrowInvariantTest is FuzzBase {
    EscrowHandler handler;

    function setUp() public {
        handler = new EscrowHandler();
        targetContract(address(handler));
    }

    function invariant_disputesOnlySettledByArbiter() public view {
        require(!handler.disputeSettledWithoutArbiter(), "dispute settled without arbiter");
    }
}
