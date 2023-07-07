// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Cheats.sol";

contract FeeTest is DSTest {
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
