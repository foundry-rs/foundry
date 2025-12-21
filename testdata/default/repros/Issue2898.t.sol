// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

// https://github.com/foundry-rs/foundry/issues/2898
contract Issue2898Test is Test {
    address private constant BRIDGE = address(10);
    address private constant BENEFICIARY = address(11);

    function setUp() public {
        vm.deal(BRIDGE, 100);
        vm.deal(BENEFICIARY, 99);

        vm.setNonce(BRIDGE, 10);
    }

    function testDealBalance() public {
        assertEq(BRIDGE.balance, 100);
        assertEq(BENEFICIARY.balance, 99);
    }

    function testSetNonce() public {
        assertEq(vm.getNonce(BRIDGE), 10);
    }
}
