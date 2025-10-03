// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract RevertingTest is Test {
    /// forge-config: default.allow_internal_expect_revert = true
    function testRevert() public {
        vm.expectRevert("should revert here");
        require(false, "should revert here");
    }
}
