// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "../cheats/Cheats.sol";

contract Payable {
    function pay() public payable {}
}

contract PaymentFailureTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function testCantPay() public {
        Payable target = new Payable();
        cheats.prank(address(1));
        target.pay{value: 1}();
    }
}
