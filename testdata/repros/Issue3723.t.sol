// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.18;

import "ds-test/test.sol";
import "../cheats/Cheats.sol";

// https://github.com/foundry-rs/foundry/issues/3723
contract Issue3723Test is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function testFailExample() public {
        cheats.expectRevert();
        revert();

        cheats.expectRevert();
        emit log_string("Do not revert");
    }
}
