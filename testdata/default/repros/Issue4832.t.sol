// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity >=0.8.18;

import "utils/Test.sol";

// https://github.com/foundry-rs/foundry/issues/4832
contract Issue4832Test is Test {
    function testFailExample() public {
        assertEq(uint256(1), 2);

        vm.expectRevert();
        revert();
    }
}
