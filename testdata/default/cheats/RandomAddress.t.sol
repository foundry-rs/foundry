// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract RandomAddress is Test {
    function testRandomAddress() public {
        vm.randomAddress();
    }
}
