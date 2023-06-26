// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.18;

import "ds-test/test.sol";
import "../cheats/Cheats.sol";

// https://github.com/foundry-rs/foundry/issues/3437
contract Issue3347Test is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function rever() internal {
        revert();
    }

    function testFailExample() public {
        cheats.expectRevert();
        rever();
    }
}
