// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.18;

import "ds-test/test.sol";
import "../cheats/Vm.sol";

// https://github.com/foundry-rs/foundry/issues/3437
contract Issue3347Test is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function rever() internal {
        revert();
    }

    function testFailExample() public {
        vm.expectRevert();
        rever();
    }
}
