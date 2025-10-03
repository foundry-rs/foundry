// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity >=0.8.18;

import "utils/Test.sol";

// https://github.com/foundry-rs/foundry/issues/3723
contract Issue3723Test is Test {
    function testFailExample() public {
        vm.expectRevert();
        revert();

        vm.expectRevert();
        emit log_string("Do not revert");
    }
}
