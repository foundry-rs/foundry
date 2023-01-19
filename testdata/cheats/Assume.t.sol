// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "./Cheats.sol";

contract AssumeTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function testAssume(uint8 x) public {
        cheats.assume(x < 2 ** 7);
        assertTrue(x < 2 ** 7, "did not discard inputs");
    }
}
