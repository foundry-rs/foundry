// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Cheats.sol";

contract SkipTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function testSkip() public {
        cheats.skip(true);
        revert("Should not reach this revert");
    }

    function testFailNotSkip() public {
        cheats.skip(false);
        revert("This test should fail");
    }

    function testFuzzSkip(uint256 x) public {
        cheats.skip(true);
        revert("Should not reach revert");
    }

    function testFailFuzzSkip(uint256 x) public {
        cheats.skip(false);
        revert("This test should fail");
    }
}
