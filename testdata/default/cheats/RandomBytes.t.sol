// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract RandomBytes is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testRandomBytes4() public {
        vm.randomBytes4();
    }

    function testRandomBytes8() public {
        vm.randomBytes8();
    }

    function testFillrandomBytes() public view {
        uint256 len = 16;
        vm.randomBytes(len);
    }
}
