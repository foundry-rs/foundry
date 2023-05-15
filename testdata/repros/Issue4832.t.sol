// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.18;

import "ds-test/test.sol";
import "../cheats/Cheats.sol";

// https://github.com/foundry-rs/foundry/issues/4832
contract Issue4832Test is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function testFailExample() public {
        assertEq(uint256(1), 2);

        cheats.expectRevert();
        revert();
    }
}
