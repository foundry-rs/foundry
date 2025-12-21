// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract FeeTest is Test {
    function testFee() public {
        vm.fee(10);
        assertEq(block.basefee, 10, "fee failed");
    }

    function testFeeFuzzed(uint64 fee) public {
        vm.fee(fee);
        assertEq(block.basefee, fee, "fee failed");
    }
}
