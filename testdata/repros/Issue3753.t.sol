// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "../cheats/Cheats.sol";

// https://github.com/foundry-rs/foundry/issues/3753
contract Issue3753Test is DSTest {
    Cheats constant vm = Cheats(HEVM_ADDRESS);

    function test_repro() public {
        bool res;
        assembly {
            res := staticcall(gas(), 4, 0, 0, 0, 0)
        }
        vm.expectRevert("require");
        require(false, "require");
    }
}
