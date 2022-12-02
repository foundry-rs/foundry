// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "../cheats/Cheats.sol";

contract Issue3437 is DSTest {
    Cheats constant vm = Cheats(HEVM_ADDRESS);

    function rever() internal {
        revert();
    }

    function testFailExample() public {
        vm.expectRevert();
        rever();
    }
}