// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "forge-std/Test.sol";
import "./Cheats.sol";

contract FeeTest is Test {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function testFee() public {
        cheats.fee(10);
        assertEq(block.basefee, 10, "fee failed");
    }

    function testFeeFuzzed(uint256 fee) public {
        cheats.fee(fee);
        assertEq(block.basefee, fee, "fee failed");
    }
}
