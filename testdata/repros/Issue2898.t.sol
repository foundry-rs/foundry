// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "../cheats/Vm.sol";
import "../logs/console.sol";

// https://github.com/foundry-rs/foundry/issues/2898
contract Issue2898Test is DSTest {
    address private constant BRIDGE = address(10);
    address private constant BENEFICIARY = address(11);
    Vm constant vm = Vm(HEVM_ADDRESS);

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
