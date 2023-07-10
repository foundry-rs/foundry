// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Vm.sol";

contract DealTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

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
