// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "./Cheats.sol";

contract DealTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function testDeal(uint256 amount) public {
        address target = address(1);
        assertEq(target.balance, 0, "initial balance incorrect");

        // Give half the amount
        cheats.deal(target, amount / 2);
        assertEq(target.balance, amount / 2, "half balance is incorrect");

        // Give the entire amount to check that deal is not additive
        cheats.deal(target, amount);
        assertEq(target.balance, amount, "deal did not overwrite balance");
    }
}
