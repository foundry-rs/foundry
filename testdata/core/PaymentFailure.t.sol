// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "forge-std/Test.sol";
import "../cheats/Cheats.sol";

contract Payable {
    function pay() payable public {}
}

contract PaymentFailureTest is Test {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function testCantPay() public {
        Payable target = new Payable();
        cheats.prank(address(1));
        target.pay{value: 1}();
    }
}
