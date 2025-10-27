// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract SkipTest is Test {
    function testSkip() public {
        vm.skip(true);
        revert("Should not reach this revert");
    }

    /// forge-config: default.allow_internal_expect_revert = true
    function testRevertIfNotSkip() public {
        vm.skip(false);
        vm.expectRevert("This test should fail");
        revert("This test should fail");
    }

    function testFuzzSkip(uint256 x) public {
        vm.skip(true);
        revert("Should not reach revert");
    }

    /// forge-config: default.allow_internal_expect_revert = true
    function testRevertIfFuzzSkip(uint256 x) public {
        vm.skip(false);
        vm.expectRevert("This test should fail");
        revert("This test should fail");
    }

    function statefulFuzzSkip() public {
        vm.skip(true);
        require(true == false, "Test should not reach invariant");
    }
}
