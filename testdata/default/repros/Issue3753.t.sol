// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

// https://github.com/foundry-rs/foundry/issues/3753
contract Issue3753Test is Test {
    function test_repro() public {
        bool res;
        assembly {
            res := staticcall(gas(), 4, 0, 0, 0, 0)
        }
        vm.expectRevert("require");
        this.revert_require();
    }

    function revert_require() public {
        revert("require");
    }
}
