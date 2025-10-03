// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity >=0.8.18;

import "utils/Test.sol";

// https://github.com/foundry-rs/foundry/issues/3437
contract Issue3347Test is Test {
    function internalRevert() internal {
        revert();
    }

    function testFailExample() public {
        vm.expectRevert();
        internalRevert();
    }
}
