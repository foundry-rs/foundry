// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract RandomBytes is Test {
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
